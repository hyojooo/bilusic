# 首页 / 插件展示架构改造方案（Home Canvas + Plugin Sections）

> 状态：方案待评审（N11k 后续）
> 日期：2026-08-22
> 关联：N11i（JS 插件诊断日志）→ N11j（REGION_CHARTS 按源隔离）→ 本次架构设计

---

## 0. 背景与痛点

当前首页（Home）与元数据插件（MetadataSource）之间的「谁来定义展示内容」职责不清，导致两个具体问题：

1. **同质内容由两套机制驱动，违背单一职责**
   - 首页的 4 类内容（歌单 / 新歌 / 新碟 / 艺人）由插件 `home()` 返回的 `HomeFeed { playlists, new_songs, albums, artists }` 字段驱动（`apps/desktop/src/pages/Home.tsx:478/494/514/534`）。
   - 但首页的「榜单」区块（Global / Featured）却**完全不**经由 `HomeFeed`——它由前端硬编码的 `REGION_CHARTS_BY_SOURCE`（`Home.tsx:55`）驱动，再去调 `metadataToplist(topid)`。
   - 结果是：话题两套命名（`playlists` vs `REGION_CHARTS`）、两套数据来源（home 拉一批 + charts 再拉一批）、两套渲染函数（`SectionTitle`+`SongCard` vs `ChartRow`），插件无法声明「我想展示什么」，只能被动接受固定四字段。

2. **「固定画布 + 插件填充」未完成**
   - 用户诉求：首页应该是**固定画布**，插件作为**内容供应商**声明「我有哪些区块、每个区块叫什么标题、里面是歌单还是歌曲还是艺人或榜单」。某些区块固定（如「最近播放」由平台层填充，`Home.tsx:401`），某些由插件决定（如 QQ 的公告牌/Melon/抖音；kuwo 的经典怀旧/会员飙升）。
   - 当前 QQ 的榜单只能硬编码在前端，切到 kuwo 直接整块消失（N11j 已修成「消失」，但未变成「显示 kuwo 自己的榜单」）。
   - 新增一个源要展示特有内容，必须改前端注册表（`REGION_CHARTS_BY_SOURCE`）——前端承担了本属于插件的内容契约，违反「插件即内容供应商」的设计意图。

---

## 1. 目标

| 目标                   | 说明                                                                                                              |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------- | --------- | ------ | ---------------------------------------------------------------------- |
| 统一内容契约           | 所有首页区块（歌单 / 新歌 / 新碟 / 艺人 / 榜单 / 任意自定义）都由插件 `home()` 返回的**统一 `sections` 数组**声明 |
| 标题由插件给真实字符串 | 区块标题不再依赖前端 i18n key（`home.group_global` 等），改为插件返回 `title`，前端只渲染不做语义假设             |
| 前端零硬编码内容       | `REGION_CHARTS_BY_SOURCE`、`home.playlists                                                                        | new_songs | albums | artists` 四分字段逐步移除，`Home.tsx`改为`sections.map(renderSection)` |
| 保留平台固定层         | 「最近播放」等平台级内容由前端（或 coordinator）注入，与插件 sections 并存，不冲突                                |
| 平滑迁移               | QQ / kuwo / netease / kugou / spotify / ytmusic 各自 `home()` 逐个迁移，不一次性破坏                              |

---

## 2. 核心数据结构

### 2.1 Rust 侧（`apps/desktop/src-tauri/src/metadata.rs`）

新增声明式 Section 模型，**保留** `HomeFeed` 向后兼容一段时间，但新增 `sections` 为主路径：

```rust
/// 区块内单条内容（统一两种卡片类型）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionItem {
    // FeedItem / FeedSong 的公共字段
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub cover: String,
    pub source: String,
    // 表明这一条是「可播放歌曲」还是「可点击卡片」
    pub kind: String, // "song" | "playlist" | "album" | "artist"
    // 仅当 kind=="song" 时有效
    pub artist: Option<String>,
    pub duration_ms: Option<i64>,
    pub engine_hint: Option<String>,
    // 仅当 kind=="playlist"/"album"/"artist" 时，点击行为由前端约定
    // （目前都是跳 search?q=title，后续可扩展 detail_route）
}

/// 首页区块：由插件声明「标题 + 类型 + 条目」
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeSection {
    pub id: String,            // 稳定标识，用于 React key / 缓存分桶
    pub title: String,         // 真实标题（中文也好、英文也好都由插件定）
    pub kind: String,          // "song" | "card" —— 决定渲染组件
    pub items: Vec<SectionItem>,
    // 可选：分组 / 排序 / 样式提示（先留空，后续按需扩展）
    pub hint: Option<String>,  // 副标题小字（如 Top N）
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HomeFeed {
    // —— 旧字段：保留以兼容未迁移的 Rust 插件 / JS 插件，标记 #[serde(default)]
    #[serde(default)]
    pub playlists: Vec<FeedItem>,
    #[serde(default)]
    pub new_songs: Vec<FeedSong>,
    #[serde(default)]
    pub albums: Vec<FeedItem>,
    #[serde(default)]
    pub artists: Vec<FeedItem>,
    // —— 新字段：声明式区块（主路径）
    #[serde(default)]
    pub sections: Vec<HomeSection>,
}
```

> 旧四字段用 `#[serde(default)]` 兜底，未迁移的插件（spotify/ytmusic/netease/kugou）仍可返回旧形状，前端在迁移完成前会做「旧字段 → 合成 section」的兼容层。

### 2.2 JS 插件契约（`apps/desktop/src-tauri/src/js_runtime.rs::home`）

当前 `home()` 已支持 `{"home":{...}}` 与扁平两种形状（`js_runtime.rs:423-444`）。改造后：

- 优先读取 `sections` 数组；若存在且非空，直接映射到 `Vec<HomeSection>`。
- 若不存在，回落到旧四字段兼容（保持向后兼容，避免一次性破坏所有 JS 插件）——这一步是**过渡期**行为，最终稳定后删除。

JS 插件 `homeOp` 返回示例：

```js
// kuwo homeOp：经典怀旧 + 会员飙升 + 热门歌手
function homeOp() {
  const sections = [];
  const sne = bangListOp('classic'); // 返回 FeedSong[]
  if (sne && sne.length)
    sections.push({
      id: 'kw_classic',
      title: '经典怀旧',
      kind: 'song',
      items: sne,
    });
  const rise = bangListOp('rise'); // 会员飙升
  if (rise && rise.length)
    sections.push({
      id: 'kw_rise',
      title: '会员飙升',
      kind: 'song',
      items: rise,
    });
  const artists = artistInfoOp(); // 运行时优先 + baked 兜底
  const aitems = (artists || []).map((a) => ({
    id: a.id,
    title: a.title,
    subtitle: a.subtitle,
    cover: a.cover,
    kind: 'artist',
    source: 'kuwo',
  }));
  if (aitems.length)
    sections.push({
      id: 'kw_artists',
      title: '热门歌手',
      kind: 'card',
      items: aitems,
    });
  return { home: { sections } };
}
```

---

## 3. 前端改造（`apps/desktop/src/pages/Home.tsx`）

### 3.1 删除 / 收敛

- 删除 `REGION_CHARTS_BY_SOURCE` 常量与 `chartsForSource()`（`Home.tsx:55-73`）。
- 删除 `ChartRow` 与 `['global','featured'].map(...)` 的硬编码分组渲染（`Home.tsx:445-476`）。
- 删除 `feed.playlists/new_songs/albums/artists` 四段独立 section（`:478-555`），改为统一的 `feed.sections.map(renderSection)`。

### 3.2 新增统一渲染器

```tsx
function renderSection(sec: HomeSection) {
  if (sec.kind === 'song') {
    return (
      <section key={sec.id} className="mt-8">
        <SectionTitle title={sec.title} hint={sec.hint} />
        <div className="flex gap-4 overflow-x-auto pb-2 [contain:paint]">
          {sec.items.map((it) => (
            <SongCard
              song={itemToSong(it)}
              playing={playingId === it.id}
              onPlay={() => void play(itemToSong(it))}
            />
          ))}
        </div>
      </section>
    );
  }
  // card：playlist / album / artist
  return (
    <section key={sec.id} className="mt-8">
      <SectionTitle title={sec.title} hint={sec.hint} />
      <div className="flex gap-3 overflow-x-auto pb-2 [contain:paint]">
        {sec.items.map((it) => (
          <FeedCard
            item={itemToFeedItem(it)}
            onClick={() =>
              navigate(`/search?q=${encodeURIComponent(it.title)}`)
            }
          />
        ))}
      </div>
    </section>
  );
}
```

### 3.3 平台固定层（最近播放）保持不变

`Home.tsx:401` 的「最近播放」属于平台层，不进 plugin sections，仍由前端 `historyList()` 注入，渲染在 plugin sections 之前。这与「画布固定 + 插件填充」完全契合：**画布 = 最近播放（固定） + 插件 sections（可变）**。

### 3.4 过渡期兼容层（关键，避免一次性破坏）

在 `Home.tsx` 顶部加一个 `feedToSections(feed)`：若 `feed.sections` 为空但四旧字段非空，按固定顺序合成等价 sections（`playlists → 歌单` / `new_songs → 新歌` / `albums → 新碟` / `artists → 艺人`）。这样未迁移的 Rust 插件（spotify/ytmusic/netease/kugou）继续工作，直到逐个迁移。迁移完成后删除兼容层。

---

## 4. Coordinator / 缓存（`coordinator.rs`）

- `home_feed()`（`coordinator.rs:161`）无需改——它返回 `HomeFeed`，新增的 `sections` 字段自动透传。
- 缓存 key `home:<id>` 已按源分桶（`coordinator.rs:163`），无需改动；切源不会串数据（之前 N11f 已修）。

---

## 5. 迁移清单（按源）

| 源                                   | 当前 home()                                                 | 迁移动作                                                                                                                                               |
| ------------------------------------ | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **kuwo (JS)**                        | `homeOp` 返回 `{home:{playlists,new_songs,albums,artists}}` | 改为返回 `sections` 数组：经典怀旧 / 会员飙升（bangList 两个榜单）+ 热门歌手（artistInfoOp 运行时优先+baked 兜底）                                     |
| **qq-music (Rust)**                  | `home()` 返回 4 字段 + 前端 REGION_CHARTS 补 5 个榜         | 把 REGION_CHARTS 的 5 个 topid 直接作为 `sections` 里的 `kind:'song'` 区块（billboard/melon/uk/oricon/douyin）写入 Rust home()；移除前端 REGION_CHARTS |
| **netease (Rust)**                   | home() 返回空/少量                                          | 保持或按其 API 填 sections                                                                                                                             |
| **kugou / spotify / ytmusic (Rust)** | 返回默认空 HomeFeed                                         | 暂不迁移，靠前端兼容层兜底（展示空）                                                                                                                   |

**推荐先迁移 kuwo（JS）——作为样板**，验证 `sections` 契约与 JS 端解析；再迁移 qq-music（Rust），验证「榜单变 section」；其余延后。

---

## 6. 校验与测试

1. **Rust**：`cargo check`（确保 `HomeSection`/`SectionItem` 编译，`#[serde(default)]` 不破坏旧插件）。
2. **JS 契约**：扩 `/tmp/kuwo-section-smoketest.cjs`，断言 `homeOp` 返回 `sections` 数组、每项含 `id/title/kind/items`，且 artists fallback 路径仍 4/4 有效。
3. **前端**：`tsc --noEmit` 0 error；手动 `pnpm tauri dev` 验证 QQ / kuwo 源切换时区块正确替换（切 qq 看到公告牌/Melon，切 kuwo 看到经典怀旧/会员飙升/热门歌手）。
4. **回归**：`cargo test --features js --lib` 保持 20 passed。

---

## 7. 风险与权衡

- **破坏性**：前端 `REGION_CHARTS` 删除后，未迁移源的榜单会暂时消失（靠兼容层保证至少四大块还在）。务必先迁 qq-music 再删前端分组逻辑。
- **i18n 影响**：区块标题改为插件返回的字符串后，不再走 `t('home.xxx')`。若需要多语言，可在插件返回里带 `title_zh/title_en`，前端按 locale 选；本期先用插件给的原始 title，后续再补 i18n 字段。
- **二次扩展**：`HomeSection` 预留 `hint`、`kind` 扩展点；未来若要「区块内再分组」「卡片点进详情页」等，只加字段，不动渲染骨架。

---

## 8. 落地步骤（建议顺序）

1. **A. 定义模型**：`metadata.rs` 加 `HomeSection` / `SectionItem`，`HomeFeed` 加 `sections`（`#[serde(default)]`）。
2. **B. JS 解析**：`js_runtime.rs::home` 优先读 `sections`，回落旧四字段。
3. **C. 前端兼容层**：`feedToSections()` + `renderSection()`；先只加不改，确保旧数据仍渲染。
4. **D. 迁 kuwo**：改 `kuwo/index.js` homeOp 为 sections（样板）。
5. **E. 迁 qq-music**：Rust home() 输出 5 个榜单 section；删除前端 `REGION_CHARTS_BY_SOURCE` 与硬编码分组。
6. **F. 清理**：删除过渡兼容层（旧四字段 → section 的合成），确认仅 sections 主路径；同步 PROGRESS.md / MEMORY.md。

---

## 9. 开放问题（待用户拍板）

1. 区块标题是否需要 i18n？本期直接用插件原始 title 是否可接受？
   答：区块标题是否需要 i18n
2. 是否接受「先迁 kuwo + qq，其余靠兼容层」的渐进策略？还是希望一次性全迁？
   答：一次性全迁
3. `kind` 枚举要不要更细（如区分 `artist` 用 `ArtistTile` 而非 `FeedCard`）——影响渲染组件选择。
   答：要
