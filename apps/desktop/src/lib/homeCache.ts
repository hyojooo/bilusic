import type { AudioCandidate, HomeFeed, MetadataTrack } from './bili';

/**
 * 首页 / 榜单缓存层。
 *
 * 这些内容是「固定时间才更新」的（QQ 音乐榜单、推荐歌单、新碟…），不需要
 * 每次切换左侧 tab 回到首页都重新发请求。这里做一层带 TTL 的缓存：
 *
 *   • 切回首页时先**同步**用缓存填充 state —— 布局立即稳定，不再闪 loading；
 *   • TTL 内直接渲染缓存，**不发任何请求**；
 *   • TTL 过期后**后台静默刷新**，有缓存时不显示 loading（用户看到的是旧
 *     数据无缝被新数据替换，几乎无感）。
 *
 * 缓存同时落 localStorage（跨 app 重启 / HMR 仍有效）和内存镜像（避免每次
 * JSON.parse），key 带 `source` 维度，切换音源时不会误用旧音源的数据。
 *
 * TTL 可调：榜单一般按小时/天更新，30 分钟对「流畅」足够且不会太旧。
 */
const FEED_TTL_MS = 30 * 60 * 1000;
const CHARTS_TTL_MS = 30 * 60 * 1000;

interface Envelope<T> {
  ts: number;
  data: T;
}

// 内存镜像：避免每次读 localStorage 都 JSON.parse，且 module 被 HMR 重置时
// localStorage 仍是兜底（本模块自身不依赖内存做持久）。
const mem = new Map<string, Envelope<unknown>>();

// Home-feed / charts cache keys are source-scoped. There is deliberately NO
// schema_rev / version bump here: local-cache invalidation is driven by
// (a) the TTL below, (b) the Settings-page "Clear local cache" action
// (`clearLocalCache`), and (c) source switches (`invalidateHomeCache`).
// A backend layout change (e.g. reshuffling QQ home sections) simply waits
// out the 30-min TTL — or the user taps Clear — no front-end version
// constant to maintain. See N11p.
const kHomeFeed = (source: string) => `bilusic.cache.home_feed.${source}`;
const kHomeCharts = (source: string) => `bilusic.cache.charts.${source}`;

function read<T>(key: string, ttl: number): T | null {
  const now = Date.now();
  const hit = mem.get(key);
  if (hit && now - hit.ts <= ttl) return hit.data as T;
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return null;
    const env = JSON.parse(raw) as Envelope<T>;
    if (now - env.ts > ttl) return null;
    mem.set(key, env);
    return env.data;
  } catch {
    return null;
  }
}

function write<T>(key: string, data: T): void {
  const env: Envelope<T> = { ts: Date.now(), data };
  mem.set(key, env);
  try {
    localStorage.setItem(key, JSON.stringify(env));
  } catch {
    // 隐私模式 / 配额超限：静默降级为仅内存缓存，不影响主流程。
  }
}

/** Build a source-scoped cache handle for the home page. */
export function getHomeCache(source: string) {
  const kFeed = kHomeFeed(source);
  const kCharts = kHomeCharts(source);
  return {
    // Plain TTL-based reads. Local cache invalidation is driven by TTL expiry,
    // the Settings-page "Clear local cache" action (see `clearLocalCache`),
    // and source switches (see `invalidateHomeCache`). No schema_rev gate —
    // see N11p.
    feed: () => read<HomeFeed>(kFeed, FEED_TTL_MS),
    charts: () => read<Record<string, MetadataTrack[]>>(kCharts, CHARTS_TTL_MS),
    putFeed: (d: HomeFeed) => write(kFeed, d),
    putCharts: (d: Record<string, MetadataTrack[]>) => write(kCharts, d),
    isFeedFresh: () => read<HomeFeed>(kFeed, FEED_TTL_MS) !== null,
    isChartsFresh: () =>
      read<Record<string, MetadataTrack[]>>(kCharts, CHARTS_TTL_MS) !== null,
  };
}

/**
 * Wipe ALL frontend-local caches (home feeds, charts, audio-search candidates)
 * from both the in-memory mirror and localStorage. Called from the Settings
 * page "Clear local cache" action. This is the user-facing escape hatch that
 * replaces the old hand-maintained `HOME_SCHEMA_REV` version stamp: instead of
 * bumping a constant on every backend home-feed change, the user (or a future
 * "force refresh") simply clears local storage and the next visit re-fetches.
 *
 * Note: this only touches keys under the `bilusic.cache.` prefix. Backend
 * SQLite caches (cachePrune / cacheReset) are handled separately via the Rust
 * commands exposed in `bili.ts`.
 */
export function clearLocalCache(): void {
  const keys: string[] = [];
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i);
      if (k && k.startsWith("bilusic.cache.")) keys.push(k);
    }
    for (const k of keys) localStorage.removeItem(k);
  } catch {
    // 隐私模式：localStorage 可能抛错，仅内存清理已生效。
  }
  // 同步清空内存镜像，避免刚删完 localStorage 又从 mem 读到旧值。
  for (const k of mem.keys()) {
    if (k.startsWith("bilusic.cache.")) mem.delete(k);
  }
}

/**
 * 切换活跃元数据源时，清掉「即将展示的那个 source」的本地首页缓存。
 *
 * 首页缓存本来就是按 source 分桶的（`bilusic.cache.home_feed.${source}`），
 * 正常不会串源。但两个场景下旧数据会赖着不走：
 *   1. 后端 `home_cache` 在 source 切换瞬间若仍读到旧 active id，可能把旧源
 *      的 feed 写进新源 key（竞态）—— 后端已在 `set_active_metadata` 里
 *      `clear()` 兜底；前端这里再清一层，双保险。
 *   2. localStorage 跨重启持久化：旧版本/旧会话一旦写过一条「污染」缓存，
 *      30 分钟 TTL 内会一直显示旧源内容。切到该 source 时主动 removeItem，
 *      强制下次进首页重新拉取新源数据。
 *
 * 只清「 incoming source 」那一组 key，不动其它 source 的缓存——频繁来回切
 * 时仍享受各自缓存，不必重复请求。
 */
export function invalidateHomeCache(source: string): void {
  const kFeed = kHomeFeed(source);
  const kCharts = kHomeCharts(source);
  mem.delete(kFeed);
  mem.delete(kCharts);
  try {
    localStorage.removeItem(kFeed);
    localStorage.removeItem(kCharts);
  } catch {
    // 隐私模式：静默忽略，仅内存清理已生效。
  }
}

/**
 * 播放器「其他音源」候选缓存。
 *
 * 与首页缓存同构（localStorage + 内存镜像 + TTL），key 取 `songKey =
 * title + artist`（与 PlayerBar 里发起搜索的 `sourceQuery` 完全一致），
 * 所以同一首歌在 30 分钟内反复打开弹窗**不发任何请求**，结果稳定不变。
 *
 * TTL 与 `FEED_TTL_MS` 对齐（30 分钟）——榜单/推荐是「按小时更新」，
 * 音源候选同理，半小时足够稳且不会太旧。
 */
const AUDIO_SEARCH_TTL_MS = 30 * 60 * 1000;

export function getAudioSearchCache(songKey: string) {
  const k = `bilusic.cache.audio_search.${songKey}`;
  return {
    /** 命中且未过期返回缓存；否则 null（调用方需发请求）。 */
    get: () => read<AudioCandidate[]>(k, AUDIO_SEARCH_TTL_MS),
    /** 写入缓存（仅在有真实结果时调用，见 PlayerBar）。 */
    put: (d: AudioCandidate[]) => write(k, d),
  };
}
