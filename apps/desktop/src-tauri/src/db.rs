//! Local persistence layer (P3).
//!
//! A single SQLite database (bundled via `rusqlite`'s `bundled` feature, so no
//! system SQLite is required) stores the user's playlists, a denormalized
//! track cache, and play history. The DB is opened at the app's data dir and
//! shared across commands through Tauri managed state.
//!
//! Design notes:
//! - All access is synchronous (rusqlite is blocking). Commands lock the
//!   `Mutex<Connection>`, do their work, and drop the guard *before* any
//!   `.await` — holding a std `Mutex` across an await point would deadlock.
//! - The track cache is keyed by `<source>:<source_id>` so the same song from
//!   QQ music and from Bilibili metadata are distinct rows.

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::metadata::MetadataTrack;

/// A cached track is considered stale past this age; the Settings page surfaces
/// the count and a manual refresh re-fetches metadata for stale entries.
const CACHE_TTL_MS: i64 = 30 * 24 * 3600 * 1000; // 30 days
/// Absolute safety ceiling on cache rows. Only *unreferenced* (orphan) rows are
/// ever evicted to honour this cap — never rows still referenced by a playlist.
const MAX_CACHE_ROWS: i64 = 50_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistSummary {
    pub id: String,
    pub name: String,
    pub cover: Option<String>,
    pub track_count: u32,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistTrack {
    pub position: i64,
    pub track: MetadataTrack,
}

/// Snapshot of cache health, shown in Settings so staleness is visible.
#[derive(Debug, Clone, Serialize)]
pub struct CacheStats {
    /// Total rows in `tracks_cache`.
    pub rows: usize,
    /// Rows still referenced by a playlist or play history (user data).
    pub referenced: usize,
    /// Rows older than `CACHE_TTL_MS` (candidates for refresh).
    pub stale: usize,
    /// Age of the oldest row, in milliseconds.
    pub oldest_age_ms: i64,
}

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open (creating if needed) the database at `path` and run migrations.
    pub fn open(path: PathBuf) -> Result<Database, AppError> {
        let conn = Connection::open(&path).map_err(|e| AppError::msg(format!("打开数据库失败：{e}")))?;
        // SQLite leaves foreign keys OFF by default, which silently disables the
        // `ON DELETE CASCADE` declared on `playlist_tracks`. Enable it so a
        // playlist delete actually cascades to its track links.
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| AppError::msg(format!("启用外键失败：{e}")))?;
        let db = Database { conn: Mutex::new(conn) };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS playlists (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                cover       TEXT,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tracks_cache (
                id    TEXT PRIMARY KEY,
                data  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS playlist_tracks (
                playlist_id TEXT NOT NULL,
                track_id    TEXT NOT NULL,
                position    INTEGER NOT NULL,
                PRIMARY KEY (playlist_id, track_id),
                FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
                FOREIGN KEY (track_id)   REFERENCES tracks_cache(id)
            );

            CREATE TABLE IF NOT EXISTS play_history (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                track_id   TEXT NOT NULL,
                played_at  INTEGER NOT NULL
            );
            "#,
        )
        .map_err(|e| AppError::msg(format!("数据库迁移失败：{e}")))?;

        // `tracks_cache` gained `updated_at` in a later build; add the column
        // if this is an older database that predates it.
        let has_col: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tracks_cache') WHERE name = 'updated_at'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if has_col == 0 {
            conn.execute(
                "ALTER TABLE tracks_cache ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(|e| AppError::msg(format!("迁移 tracks_cache 失败：{e}")))?;
        }
        Ok(())
    }

    fn cache_key(track: &MetadataTrack) -> String {
        format!("{}:{}", track.source, track.source_id)
    }

    /// Insert-or-replace a track into the cache, stamping `updated_at` so the
    /// row's freshness (and LRU order among orphans) is tracked.
    fn upsert_cache(conn: &Connection, track: &MetadataTrack) -> Result<(), AppError> {
        let data = serde_json::to_string(track)
            .map_err(|e| AppError::msg(format!("序列化曲目失败：{e}")))?;
        conn.execute(
            "INSERT OR REPLACE INTO tracks_cache (id, data, updated_at) VALUES (?1, ?2, ?3)",
            params![Self::cache_key(track), data, now_ms()],
        )
        .map_err(|e| AppError::msg(format!("写入曲目缓存失败：{e}")))?;
        Ok(())
    }

    fn read_cache(conn: &Connection, id: &str) -> Result<MetadataTrack, AppError> {
        let data: String = conn
            .query_row("SELECT data FROM tracks_cache WHERE id = ?1", params![id], |r| {
                r.get(0)
            })
            .map_err(|e| AppError::msg(format!("读取曲目缓存失败：{e}")))?;
        serde_json::from_str(&data).map_err(|e| AppError::msg(format!("解析曲目失败：{e}")))
    }

    /// Public entry: delete **all** `tracks_cache` rows that are no longer
    /// referenced by any playlist or play history. Returns rows removed. This is
    /// what the manual "清理无效缓存" button and every write path use, so a
    /// deleted playlist's tracks leave no cache residue.
    pub fn prune_orphan_cache(&self) -> Result<usize, AppError> {
        let conn = self.conn.lock().unwrap();
        Self::prune_conn_full(&*conn)
    }

    /// Startup-only: also drop `playlist_tracks` rows whose playlist was deleted
    /// while the foreign-key cascade used to be disabled, then enforce the
    /// absolute cache cap. Safe to call always.
    pub fn prune_orphans(&self) -> Result<usize, AppError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id NOT IN (SELECT id FROM playlists)",
            [],
        )
        .map_err(|e| AppError::msg(format!("清理孤立曲目失败：{e}")))?;
        let n = Self::prune_conn_full(&*conn)?;
        self.enforce_cache_cap_conn(&conn, MAX_CACHE_ROWS)?;
        Ok(n)
    }

    /// Defensive hard ceiling: if the cache somehow exceeds `max_rows`, evict
    /// the *oldest unreferenced* (orphan) rows only. Rows still referenced by a
    /// playlist are never touched — user library data is永远 preserved.
    pub fn enforce_cache_cap(&self, max_rows: usize) -> Result<usize, AppError> {
        let conn = self.conn.lock().unwrap();
        self.enforce_cache_cap_conn(&*conn, max_rows as i64)
    }

    fn enforce_cache_cap_conn(&self, conn: &Connection, max_rows: i64) -> Result<usize, AppError> {
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM tracks_cache", [], |r| r.get(0))
            .map_err(|e| AppError::msg(format!("统计缓存失败：{e}")))?;
        if total <= max_rows {
            return Ok(0);
        }
        let over = total - max_rows;
        conn.execute(
            "DELETE FROM tracks_cache
             WHERE id NOT IN (SELECT track_id FROM playlist_tracks)
               AND id NOT IN (SELECT track_id FROM play_history)
               AND id IN (SELECT id FROM tracks_cache ORDER BY updated_at ASC LIMIT ?1)",
            params![over],
        )
        .map(|n| n as usize)
        .map_err(|e| AppError::msg(format!("缓存封顶失败：{e}")))
    }

    /// Delete every `tracks_cache` row not referenced by a playlist or history.
    fn prune_conn_full(conn: &Connection) -> Result<usize, AppError> {
        conn.execute(
            "DELETE FROM tracks_cache
             WHERE id NOT IN (SELECT track_id FROM playlist_tracks)
               AND id NOT IN (SELECT track_id FROM play_history)",
            [],
        )
        .map(|n| n as usize)
        .map_err(|e| AppError::msg(format!("清理缓存失败：{e}")))
    }

    // ---- playlists -------------------------------------------------------

    pub fn create_playlist(
        &self,
        name: &str,
        cover: Option<&str>,
    ) -> Result<PlaylistSummary, AppError> {
        let conn = self.conn.lock().unwrap();
        let id = format!("pl_{}", uuid_now());
        let now = now_ms();
        conn.execute(
            "INSERT INTO playlists (id, name, cover, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, name, cover, now, now],
        )
        .map_err(|e| AppError::msg(format!("创建歌单失败：{e}")))?;
        Ok(PlaylistSummary {
            id,
            name: name.to_string(),
            cover: cover.map(|s| s.to_string()),
            track_count: 0,
            updated_at: now,
        })
    }

    pub fn list_playlists(&self) -> Result<Vec<PlaylistSummary>, AppError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT p.id, p.name, p.cover, p.updated_at,
                        (SELECT COUNT(*) FROM playlist_tracks t WHERE t.playlist_id = p.id)
                 FROM playlists p ORDER BY p.updated_at DESC",
            )
            .map_err(|e| AppError::msg(format!("查询歌单失败：{e}")))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PlaylistSummary {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    cover: r.get(2)?,
                    track_count: r.get(3)?,
                    updated_at: r.get(4)?,
                })
            })
            .map_err(|e| AppError::msg(format!("读取歌单失败：{e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::msg(format!("读取歌单失败：{e}")))
    }

    pub fn rename_playlist(&self, id: &str, name: &str) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE playlists SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![name, now_ms(), id],
        )
        .map_err(|e| AppError::msg(format!("重命名歌单失败：{e}")))?;
        Ok(())
    }

    pub fn delete_playlist(&self, id: &str) -> Result<usize, AppError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM playlists WHERE id = ?1", params![id])
            .map_err(|e| AppError::msg(format!("删除歌单失败：{e}")))?;
        // The cascade removed this playlist's rows from `playlist_tracks`;
        // drop any track-cache entries that are now orphaned.
        Self::prune_conn_full(&*conn)
    }

    // ---- playlist tracks -------------------------------------------------

    pub fn add_track(&self, playlist_id: &str, track: &MetadataTrack) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap();
        Self::upsert_cache(&conn, track)?;
        let key = Self::cache_key(track);
        conn.execute(
            "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(position), 0) + 1 FROM playlist_tracks WHERE playlist_id = ?1))",
            params![playlist_id, key],
        )
        .map_err(|e| AppError::msg(format!("添加曲目失败：{e}")))?;
        conn.execute(
            "UPDATE playlists SET updated_at = ?1 WHERE id = ?2",
            params![now_ms(), playlist_id],
        )
        .map_err(|e| AppError::msg(format!("更新歌单失败：{e}")))?;
        Ok(())
    }

    pub fn remove_track(&self, playlist_id: &str, track_id: &str) -> Result<usize, AppError> {
        let mut conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
            params![playlist_id, track_id],
        )
        .map_err(|e| AppError::msg(format!("移除曲目失败：{e}")))?;
        self.renumber(&mut conn, playlist_id)?;
        // The removed track may have lost its last reference (not in history).
        Self::prune_conn_full(&*conn)
    }

    pub fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<PlaylistTrack>, AppError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT pt.position, pt.track_id
                 FROM playlist_tracks pt WHERE pt.playlist_id = ?1 ORDER BY pt.position ASC",
            )
            .map_err(|e| AppError::msg(format!("查询歌单曲目失败：{e}")))?;
        let ids = stmt
            .query_map(params![playlist_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| AppError::msg(format!("读取歌单曲目失败：{e}")))?;
        let mut out = Vec::new();
        for row in ids {
            let (position, key) = row.map_err(|e| AppError::msg(format!("读取歌单曲目失败：{e}")))?;
            let track = Self::read_cache(&conn, &key)?;
            out.push(PlaylistTrack { position, track });
        }
        Ok(out)
    }

    /// Persist an explicit ordering of track ids for a playlist.
    pub fn reorder(&self, playlist_id: &str, ordered_ids: &[String]) -> Result<(), AppError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction()
            .map_err(|e| AppError::msg(format!("事务开启失败：{e}")))?;
        for (i, id) in ordered_ids.iter().enumerate() {
            tx.execute(
                "UPDATE playlist_tracks SET position = ?1 WHERE playlist_id = ?2 AND track_id = ?3",
                params![i as i64, playlist_id, id],
            )
            .map_err(|e| AppError::msg(format!("排序失败：{e}")))?;
        }
        tx.commit()
            .map_err(|e| AppError::msg(format!("提交排序失败：{e}")))?;
        Ok(())
    }

    /// After a delete, rewrite positions to be contiguous 0..n.
    fn renumber(&self, conn: &mut Connection, playlist_id: &str) -> Result<(), AppError> {
        // Collect ids first so the read statement is dropped before we open
        // the write transaction (a `Statement` borrow would otherwise conflict
        // with the mutable `transaction()` borrow).
        let ids: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position ASC",
                )
                .map_err(|e| AppError::msg(format!("查询失败：{e}")))?;
            let rows = stmt
                .query_map(params![playlist_id], |r| r.get::<_, String>(0))
                .map_err(|e| AppError::msg(format!("查询失败：{e}")))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| AppError::msg(format!("查询失败：{e}")))?
        };
        let tx = conn
            .transaction()
            .map_err(|e| AppError::msg(format!("事务开启失败：{e}")))?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE playlist_tracks SET position = ?1 WHERE playlist_id = ?2 AND track_id = ?3",
                params![i as i64, playlist_id, id],
            )
            .map_err(|e| AppError::msg(format!("重排失败：{e}")))?;
        }
        tx.commit()
            .map_err(|e| AppError::msg(format!("提交重排失败：{e}")))?;
        Ok(())
    }

    // ---- play history ----------------------------------------------------

    pub fn add_history(&self, track: &MetadataTrack) -> Result<usize, AppError> {
        let conn = self.conn.lock().unwrap();
        Self::upsert_cache(&conn, track)?;
        conn.execute(
            "INSERT INTO play_history (track_id, played_at) VALUES (?1, ?2)",
            params![Self::cache_key(track), now_ms()],
        )
        .map_err(|e| AppError::msg(format!("写入历史失败：{e}")))?;
        // Keep at most 200 history rows.
        conn.execute(
            "DELETE FROM play_history WHERE id NOT IN (SELECT id FROM play_history ORDER BY played_at DESC LIMIT 200)",
            [],
        )
        .map_err(|e| AppError::msg(format!("清理历史失败：{e}")))?;
        // Rows dropped by the cap may have been a track's only reference.
        Self::prune_conn_full(&*conn)
    }

    pub fn list_history(&self, limit: u32) -> Result<Vec<MetadataTrack>, AppError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT c.data FROM play_history h
                 JOIN tracks_cache c ON c.id = h.track_id
                 ORDER BY h.played_at DESC LIMIT ?1",
            )
            .map_err(|e| AppError::msg(format!("查询历史失败：{e}")))?;
        let rows = stmt
            .query_map(params![limit as i64], |r| r.get::<_, String>(0))
            .map_err(|e| AppError::msg(format!("读取历史失败：{e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let data = row.map_err(|e| AppError::msg(format!("读取历史失败：{e}")))?;
            out.push(
                serde_json::from_str(&data)
                    .map_err(|e| AppError::msg(format!("解析历史失败：{e}")))?,
            );
        }
        Ok(out)
    }

    /// Clear every play-history row. Orphaned cache entries are pruned, and the
    /// now-free pages are `VACUUM`ed so the file actually shrinks.
    pub fn clear_history(&self) -> Result<usize, AppError> {
        let conn = self.conn.lock().unwrap();
        let n = conn
            .execute("DELETE FROM play_history", [])
            .map_err(|e| AppError::msg(format!("清除历史失败：{e}")))?;
        Self::prune_conn_full(&*conn)?;
        // `DELETE` only frees pages logically; compact so the disk footprint
        // shrinks. Best-effort — if VACUUM fails the data is already cleared.
        if let Err(e) = conn.execute("VACUUM", []) {
            eprintln!("[bilusic] 清历史后 VACUUM 失败：{e}");
        }
        Ok(n as usize)
    }

    /// Wipe all local data: playlists, their tracks, history, and the cache.
    /// Destructive — the frontend must confirm with the user first. The file is
    /// `VACUUM`ed afterwards so the wipe is reflected on disk immediately.
    pub fn reset_all(&self) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "DELETE FROM playlist_tracks;
             DELETE FROM play_history;
             DELETE FROM playlists;
             DELETE FROM tracks_cache;",
        )
        .map_err(|e| AppError::msg(format!("重置数据失败：{e}")))?;
        if let Err(e) = conn.execute("VACUUM", []) {
            eprintln!("[bilusic] 重置后 VACUUM 失败（数据已清空）：{e}");
        }
        Ok(())
    }

    // ---- disk compaction ------------------------------------------------

    /// Reclaim disk space. SQLite `DELETE` only marks pages as reusable — the
    /// `.db` file does not shrink on its own (especially under WAL). `VACUUM`
    /// rewrites the database into a minimal file. Returns the approximate bytes
    /// reclaimed (`page_count * page_size` before/after). If nothing was free,
    /// returns 0.
    ///
    /// Note: a `VACUUM` rewrites the whole file and blocks the connection, so
    /// call it only on explicit user actions (prune / reset / compact), never on
    /// the hot write paths or at startup.
    pub fn vacuum(&self) -> Result<i64, AppError> {
        let conn = self.conn.lock().unwrap();
        let before = Self::approx_file_size(&conn)?;
        conn.execute("VACUUM", [])
            .map_err(|e| AppError::msg(format!("压缩数据库失败：{e}")))?;
        let after = Self::approx_file_size(&conn)?;
        Ok((before - after).max(0))
    }

    /// Approximate on-disk size from SQLite's own page accounting. This is the
    /// main database file size; the `-wal`/`-shm` sidecars are not included.
    fn approx_file_size(conn: &Connection) -> Result<i64, AppError> {
        let pages: i64 = conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .map_err(|e| AppError::msg(format!("读取页计数失败：{e}")))?;
        let psize: i64 = conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .map_err(|e| AppError::msg(format!("读取页大小失败：{e}")))?;
        Ok(pages * psize)
    }

    /// Exit-time compaction. Only runs `VACUUM` when there are actually free
    /// pages to reclaim (`freelist_count > 0`) — i.e. this session deleted or
    /// reset data. A clean exit with no writes skips the full rewrite so the
    /// shutdown stays fast. Returns bytes reclaimed (`0` if nothing to do).
    pub fn auto_vacuum(&self) -> Result<i64, AppError> {
        let conn = self.conn.lock().unwrap();
        let freelist: i64 = conn
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .map_err(|e| AppError::msg(format!("读取空闲页失败：{e}")))?;
        if freelist <= 0 {
            return Ok(0);
        }
        let before = Self::approx_file_size(&conn)?;
        conn.execute("VACUUM", [])
            .map_err(|e| AppError::msg(format!("压缩数据库失败：{e}")))?;
        let after = Self::approx_file_size(&conn)?;
        Ok((before - after).max(0))
    }

    /// Cheap check (a single `PRAGMA`, microseconds) used at shutdown to decide
    /// whether a deferred `VACUUM` is worth scheduling. Never blocks on I/O.
    pub fn needs_vacuum(&self) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.query_row("PRAGMA freelist_count", [], |r| r.get::<_, i64>(0))
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    // ---- cache maintenance ----------------------------------------------

    /// Public upsert used by the cache-refresh path (re-stamp `updated_at`).
    pub fn upsert_track(&self, track: &MetadataTrack) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap();
        Self::upsert_cache(&conn, track)
    }

    /// Return every cached track (used by the refresh command to re-fetch fresh
    /// metadata from the active source).
    pub fn all_cached_tracks(&self) -> Result<Vec<MetadataTrack>, AppError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT data FROM tracks_cache")
            .map_err(|e| AppError::msg(format!("查询缓存失败：{e}")))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| AppError::msg(format!("读取缓存失败：{e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let data = row.map_err(|e| AppError::msg(format!("读取缓存失败：{e}")))?;
            out.push(
                serde_json::from_str(&data)
                    .map_err(|e| AppError::msg(format!("解析缓存失败：{e}")))?,
            );
        }
        Ok(out)
    }

    /// A health snapshot of the track cache, surfaced in Settings.
    pub fn cache_stats(&self) -> Result<CacheStats, AppError> {
        let conn = self.conn.lock().unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM tracks_cache", [], |r| r.get(0))
            .map_err(|e| AppError::msg(format!("统计缓存失败：{e}")))?;
        let referenced: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracks_cache
                 WHERE id IN (SELECT track_id FROM playlist_tracks)
                    OR id IN (SELECT track_id FROM play_history)",
                [],
                |r| r.get(0),
            )
            .map_err(|e| AppError::msg(format!("统计缓存失败：{e}")))?;
        let cutoff = now_ms() - CACHE_TTL_MS;
        let stale: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracks_cache WHERE updated_at > 0 AND updated_at < ?1",
                params![cutoff],
                |r| r.get(0),
            )
            .map_err(|e| AppError::msg(format!("统计缓存失败：{e}")))?;
        let oldest: i64 = conn
            .query_row("SELECT COALESCE(MIN(updated_at), 0) FROM tracks_cache", [], |r| {
                r.get(0)
            })
            .map_err(|e| AppError::msg(format!("统计缓存失败：{e}")))?;
        let oldest_age_ms = if oldest > 0 { now_ms() - oldest } else { 0 };
        Ok(CacheStats {
            rows: rows as usize,
            referenced: referenced as usize,
            stale: stale as usize,
            oldest_age_ms,
        })
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A cheap unique id without pulling in a uuid crate: timestamp + counter.
fn uuid_now() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    format!("{:x}{}", now_ms(), C.fetch_add(1, Ordering::Relaxed))
}
