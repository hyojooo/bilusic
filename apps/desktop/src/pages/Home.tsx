import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import i18n from '@/i18n';
import { SearchIcon, PlayIcon } from '@/components/icons';
import AddToPlaylistMenu from '@/components/AddToPlaylistMenu';
import ArtistTile from '@/components/ArtistTile';
import CoverImage from '@/components/CoverImage';
import { useSettings } from '@/stores/settings';
import { usePlayer } from '@/stores/player';
import {
  metadataHome,
  historyList,
  pluginList,
  type HomeFeed,
  type FeedSong,
  type HomeSection,
  type SectionItem,
  type MetadataTrack,
  type PluginListItem,
  extractErrorMessage,
} from '@/lib/bili';
import { getHomeCache } from '@/lib/homeCache';

// 后端传来的中文 hint → i18n slug 映射；未收录的 hint 回退显示原文。
// 必须放在模块顶层（Home 函数体外部）——见下方 SectionBlock 中的 t 调用。
// 如果在 Home 函数内部 `return` 之后再声明 `const`，JSX 渲染 <SectionBlock>
// 时该 const 还没执行到，会触发 TDZ: ReferenceError: Cannot access
// uninitialized variable.（function 声明可 hoist；const 不可。）
const HINT_KEYS: Record<string, string> = {
  '每周更新': 'weekly',
  '本周热度': 'weeklyHot',
  '本周热门': 'thisWeekHot',
  '近期发行': 'recentRelease',
  '编辑精选': 'editorPick',
};

/** Convert a home-page SectionItem (kind === "song") into the FeedSong shape
 *  the song renderer expects. */
function itemToFeedSong(it: SectionItem): FeedSong {
  return {
    source_id: it.id,
    title: it.title,
    artist: it.artist ?? '',
    cover: it.cover,
    duration_ms: it.duration_ms ?? 0,
    source: it.source,
    engine_hint: it.engine_hint,
  };
}

/** Convert a home-page SectionItem (browse card) into the legacy FeedItem shape. */
function itemToFeedItem(it: SectionItem): SectionItem {
  return {
    id: it.id,
    title: it.title,
    subtitle: it.subtitle,
    cover: it.cover,
    kind: it.kind,
    source: it.source,
    artist: it.artist,
    duration_ms: it.duration_ms,
    engine_hint: it.engine_hint,
  };
}

function fmtDur(ms: number): string {
  if (!ms || !isFinite(ms)) return '0:00';
  const total = Math.floor(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, '0')}`;
}

function songToTrack(s: FeedSong): MetadataTrack {
  return {
    source_id: s.source_id,
    source: s.source,
    engine_hint: s.engine_hint,
    title: s.title,
    artist: s.artist,
    album: '',
    duration_ms: s.duration_ms,
    cover: s.cover,
    lyrics_id: null,
  };
}

function SectionTitle({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="mb-3 flex items-baseline justify-between">
      <h2 className="text-lg font-semibold text-fg">{title}</h2>
      {hint && <span className="text-xs text-muted">{hint}</span>}
    </div>
  );
}

function SongCard({
  song,
  onPlay,
  playing,
}: {
  song: FeedSong;
  onPlay: () => void;
  playing: boolean;
}) {
  return (
    <div className="group flex w-44 shrink-0 flex-col gap-2">
      <div className="relative">
        <CoverImage
          src={song.cover}
          alt=""
          wrapperClassName="aspect-square w-full rounded-xl"
          className="object-cover"
        />
        <AddToPlaylistMenu
          track={songToTrack(song)}
          className="absolute right-2 top-2"
        />
        <button
          onClick={onPlay}
          className={`absolute bottom-2 right-2 flex h-10 w-10 items-center justify-center rounded-full bg-accent-500 text-white shadow-lg transition-opacity ${
            playing ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'
          }`}
          aria-label={i18n.t('common.play')}
        >
          <PlayIcon width={18} height={18} />
        </button>
      </div>
      <p className="truncate text-sm text-fg" title={song.title}>
        {song.title}
      </p>
      <p className="truncate text-xs text-muted">
        {song.artist} · {fmtDur(song.duration_ms)}
      </p>
    </div>
  );
}

function FeedCard({
  item,
  onClick,
}: {
  item: SectionItem;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="group flex w-40 shrink-0 flex-col gap-2 text-left"
    >
      <CoverImage
        src={item.cover}
        alt=""
        wrapperClassName={`w-full aspect-square transition-transform group-hover:scale-[1.03] ${
          item.kind === 'artist' ? '!rounded-full' : 'rounded-xl'
        }`}
        className="object-cover"
      />
      {/* Flex item width defaults to `auto` (shrink-to-fit by intrinsic
          content). On `<img>` we override with `w-full`; on `<p>` we do the
          same here. Without `w-full` the paragraph honors intrinsic width
          regardless of `overflow:hidden`, so long titles bleed onto the next
          card even with `min-w-0 truncate`. `line-clamp-1` is a redundant
          safety net in case `white-space:nowrap` ever collapses (e.g. an
          embedded line-break) — `-webkit-line-clamp:1` enforces 1 line via a
          second independent pipeline. */}
      <p
        className="min-w-0 w-full line-clamp-1 truncate text-[13px] leading-snug text-fg"
        title={item.title}
      >
        {item.title}
      </p>
      {item.subtitle && (
        <p
          className="min-w-0 w-full line-clamp-1 truncate text-xs text-muted"
          title={item.subtitle}
        >
          {item.subtitle}
        </p>
      )}
    </button>
  );
}

export default function Home() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const metadataSource = useSettings((s) => s.metadataSource);
  const [feed, setFeed] = useState<HomeFeed | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [playingId, setPlayingId] = useState<string | null>(null);
  const [recent, setRecent] = useState<MetadataTrack[]>([]);
  // Map metadata plugin id -> display name, so the hero line follows the user's
  // active source choice (not a hard-coded fallback like "QQ 音乐").
  const [metadataNames, setMetadataNames] = useState<Record<string, string>>(
    {},
  );

  useEffect(() => {
    let alive = true;
    void pluginList().then((items: PluginListItem[]) => {
      if (!alive) return;
      const next: Record<string, string> = {};
      for (const it of items) {
        if (it.kind === 'metadata') next[it.id] = it.name;
      }
      setMetadataNames(next);
    });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    let alive = true;
    const cache = getHomeCache(metadataSource);

    // 1) Synchronously fill from cache first. This is the key fix for the
    //    "switch tab -> flash loading -> covers load one-by-one" jank: the
    //    layout is stable the instant you return to Home, with no `null`
    //    reset. Only when there is NO cache at all do we fall back to the
    //    loading state (first-ever launch / cache evicted).
    const cf = cache.feed();
    if (cf) setFeed(cf);
    else setFeed(null);
    setError(null);

    // 2) Within TTL we already rendered the cached data -- fire no requests.
    if (cache.isFeedFresh()) {
      return () => {
        alive = false;
      };
    }

    // 兜底：即使后端因某种原因（极端网络抖动 / 接口 hang）让 promise 永远不
    // resolve，前端也不该无限转圈。30s 超时后若该源仍无数据，显示错误态 +
    // 「重试」入口，用户可以主动再试，而不是被困在「正在加载首页…」。
    // 后端 reqwest 已配 15s 超时（见 qq_music_metadata::client），QQ 源首页
    // 最坏 ~15s 必返回；这里的 30s 是针对「其它元数据源也可能 hang」的双保险。
    const withTimeout = <T,>(p: Promise<T>, ms: number): Promise<T> =>
      Promise.race([
        p,
        new Promise<T>((_, reject) =>
          setTimeout(
            () => reject(new Error(`首页加载超时（${Math.round(ms / 1000)}s）`)),
            ms,
          ),
        ),
      ]);

    // 3) Expired (or first launch): refresh silently in the background.
    //    Because the cache was already populated above, the user sees the
    //    old data seamlessly replaced by new data -- no loading flash.
    withTimeout(metadataHome(metadataSource), 30_000)
      .then((f) => {
        if (alive) {
          setFeed(f);
          cache.putFeed(f);
        }
      })
      .catch((e: unknown) => {
        // Only surface an error when we had nothing to show at all.
        if (alive && !cf) setError(extractErrorMessage(e));
      });
    historyList(20)
      .then((r) => {
        if (alive) setRecent(r);
      })
      .catch(() => {});
    // The home feed now arrives as a single `sections` array declared by the
    // active metadata plugin (see docs/home-architecture.md). Charts are no
    // longer fetched separately — the plugin's `home` op already bundles
    // them as `kind: "song"` sections with real titles. No extra requests,
    // no front-end per-source registry.
    return () => {
      alive = false;
    };
  }, [metadataSource]);

  const play = async (s: FeedSong) => {
    try {
      setPlayingId(s.source_id);
      await usePlayer.getState().resolveAndPlay(songToTrack(s));
    } catch (e: unknown) {
      setError(extractErrorMessage(e));
    }
  };

  const total =
    feed == null
      ? 0
      : feed.sections.reduce((acc, s) => acc + s.items.length, 0);

  return (
    <div className="mx-auto max-w-5xl px-4 pb-10">
      {/* Hero */}
      <div className="flex flex-col gap-4 pt-10 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <span className="font-brand text-5xl text-accent-500 select-none">
            Bilusic
          </span>
          <p className="mt-2 text-sm text-muted">
            {t('home.tagline')} ·{' '}
            {t('home.fromSource', {
              source: metadataNames[metadataSource] ?? metadataSource,
            })}
          </p>
        </div>
        <button
          onClick={() => navigate('/search')}
          className="btn-accent self-start"
        >
          <SearchIcon width={18} height={18} /> {t('common.search')}
        </button>
      </div>

      {error && (
        <p className="mt-6 rounded-xl border border-line bg-surface-2 px-4 py-3 text-xs text-muted">
          {t('home.loadFailed', { error })}
          <br />
          {t('home.loadFailedHint')}
        </p>
      )}

      {feed == null && !error && (
        <p className="mt-10 text-center text-sm text-muted">
          {t('home.loading')}
        </p>
        // <></>
      )}

      {feed && total === 0 && !error && (
        <p className="mt-10 text-center text-sm text-muted">
          {t('home.empty')}
        </p>
      )}

      {recent.length > 0 && (
        <section className="mt-8">
          <SectionTitle title={t('home.recent')} hint={t('home.recentHint')} />
          <div className="flex gap-4 overflow-x-auto pb-2 [contain:paint]">
            {recent.map((track) => (
              <div
                key={`${track.source}:${track.source_id}`}
                className="group flex w-44 shrink-0 flex-col gap-2"
              >
                <div className="relative">
                  <CoverImage
                    src={track.cover}
                    alt=""
                    wrapperClassName="aspect-square w-full rounded-xl"
                    className="object-cover"
                  />
<AddToPlaylistMenu
                  track={track}
                  className="absolute right-2 top-2"
                />
                  <button
                    onClick={() => void play(songFromTrack(track))}
                    className="absolute bottom-2 right-2 flex h-10 w-10 items-center justify-center rounded-full bg-accent-500 text-white shadow-lg opacity-0 transition-opacity group-hover:opacity-100"
                    aria-label={t('common.play')}
                  >
                    <PlayIcon width={18} height={18} />
                  </button>
                </div>
                <p className="truncate text-sm text-fg" title={track.title}>
                  {track.title}
                </p>
                <p className="truncate text-xs text-muted">{track.artist}</p>
              </div>
            ))}
          </div>
        </section>
      )}

      {/* Plugin-declared sections: the active metadata source supplies an
          ordered list of sections (each with a real title + item kind). We
          render them generically on a fixed canvas. The "recently played"
          platform layer above is the only front-end-owned block. */}
      {feed &&
        feed.sections.map((sec) => (
          <SectionBlock
            key={sec.id}
            sec={sec}
            playingId={playingId}
            onPlay={play}
            metadataSource={metadataSource}
          />
        ))}
    </div>
  );
}

/** Render a single plugin-declared section. Songs (kind === "song") use the
 *  playable SongCard row; playlist/album use FeedCard; artist uses ArtistTile.
 *
 * 顶级组件（不在 Home 内嵌套）：HINT_KEYS 已是模块级常量，这里再自己调用
 * useTranslation / useNavigate，metadataSource 通过 prop 传入——彻底避免
 * 之前「const 写在 return 之后导致 TDZ 白屏」的陷阱。 */
function SectionBlock({
  sec,
  playingId,
  onPlay,
  metadataSource,
}: {
  sec: HomeSection;
  playingId: string | null;
  onPlay: (song: FeedSong) => void;
  metadataSource: string;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  if (sec.items.length === 0) return null;
  // 区块标题/hint 走 i18n：按 元数据源 + section.id 查翻译，未命中回退后端中文。
  const sectionTitle = t(`home.section.${metadataSource}.${sec.id}`, {
    defaultValue: sec.title,
  });
  const sectionHint = sec.hint
    ? t(`home.hint.${HINT_KEYS[sec.hint] ?? '_'}`, { defaultValue: sec.hint })
    : undefined;
  return (
    <section className="mt-8">
      <SectionTitle title={sectionTitle} hint={sectionHint} />
      {sec.kind === 'song' ? (
        <div className="flex gap-4 overflow-x-auto pb-2 [contain:paint]">
          {sec.items.map((it) => {
            const song = itemToFeedSong(it);
            return (
              <SongCard
                key={`${it.source}:${it.id}`}
                song={song}
                playing={playingId === it.id}
                onPlay={() => onPlay(song)}
              />
            );
          })}
        </div>
      ) : sec.kind === 'card' && it_kind_is_artist(sec) ? (
        <div className="flex gap-3 overflow-x-auto pb-2 [contain:paint]">
          {sec.items.map((it) => (
            <div key={it.id} className="w-24 shrink-0">
              <ArtistTile
                name={it.title}
                cover={it.cover}
                subtitle={it.subtitle ?? ''}
                onClick={() =>
                  navigate(`/search?q=${encodeURIComponent(it.title)}`)
                }
              />
            </div>
          ))}
        </div>
      ) : (
        <div className="flex gap-4 overflow-x-auto pb-2 [contain:paint]">
          {sec.items.map((it) => {
            // Playlists have a real detail view (the actual song list);
            // albums still fall through to the legacy `/search?q=<title>`
            // fallback until AlbumDetail ships — they're a separate track
            // of work.
            const handleClick =
              it.kind === 'playlist'
                ? () =>
                    navigate(
                      `/discover/playlist/${encodeURIComponent(
                        it.source,
                      )}/${encodeURIComponent(it.id)}`,
                      {
                        state: {
                          title: it.title,
                          cover: it.cover,
                          subtitle: it.subtitle ?? undefined,
                        },
                      },
                    )
                : () =>
                    navigate(`/search?q=${encodeURIComponent(it.title)}`);
            return (
              <FeedCard
                key={it.id}
                item={itemToFeedItem(it)}
                onClick={handleClick}
              />
            );
          })}
        </div>
      )}
    </section>
  );
}

/** True when every item in the section is an artist card. */
function it_kind_is_artist(sec: HomeSection): boolean {
  return sec.items.every((it) => it.kind === 'artist');
}

function songFromTrack(t: MetadataTrack): FeedSong {
  return {
    source_id: t.source_id,
    title: t.title,
    artist: t.artist,
    cover: t.cover,
    duration_ms: t.duration_ms,
    source: t.source,
    engine_hint: t.engine_hint,
  };
}
