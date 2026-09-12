# 发版指南（Release Guide）

本指南说明如何在本地改动代码后，手动发布一个新版本到 GitHub Releases。

## 概述：CI 是怎么工作的

仓库已配置 `.github/workflows/build.yml`：

| 触发事件 | CI 行为 |
|---|---|
| `push` 到 `main` 分支 | 仅三平台（macOS / Windows / Linux）**构建验证**，不发布 Release |
| `push` 一个 `v*` tag（如 `v0.2.0`） | 三平台构建 **+ 自动创建 GitHub Release + 上传安装包** |

也就是说：**打 tag 即发版**。日常提交推到 `main` 只会验证能否构建成功，不会发布。

> 产物当前为**未签名**构建，用户下载 macOS `.dmg` / Windows `.msi` 时会遇到 Gatekeeper / SmartScreen 安全提示，但可正常安装。如需消除提示需另行配置代码签名（进阶）。

## 前置条件

- 本地已 `git clone` 且 `origin` 指向你的 GitHub 仓库（本工程：`git@github.com:hyojooo/bilusic.git`）
- 本机 SSH key 已加入 GitHub 账户（`ssh -T git@github.com` 能成功）
- 已安装：Node 22 + pnpm 9、Rust stable、系统依赖（Linux 需 `libwebkit2gtk-4.1-dev` 等）
- 当前分支为 `main`

## 手动发版步骤

```bash
# 0. 进入工程目录
cd /Users/guho/Desktop/Developer/bilusic

# 1. 改完代码后，提交你的改动
git add -A
git commit -m "feat: 你的改动说明"

# 2. 升级版本号（⚠️ 下面三个文件必须保持一致！）
#      apps/desktop/src-tauri/Cargo.toml          → [package] version
#      apps/desktop/src-tauri/tauri.conf.json     → "version"
#      apps/desktop/package.json                  → "version"
#    例如 0.1.0 → 0.2.0（新功能）或 0.1.1（修 bug）
git add apps/desktop/src-tauri/Cargo.toml \
        apps/desktop/src-tauri/tauri.conf.json \
        apps/desktop/package.json
git commit -m "chore: bump version to 0.2.0"

# 3. 推送到 main（触发普通构建，确认能通过）
git push origin main

# 4. 打 tag（⚠️ 必须用 v 前缀，CI 只监听 tags: ['v*']）
git tag v0.2.0

# 5. 推送 tag（触发发版构建 → 自动创建 Release + 上传 .dmg/.msi/.AppImage）
git push --tags
```

完成后去 **GitHub → Releases** 等三平台构建结束（约 15–40 分钟），即可看到新版本与三个安装包。

## 关键提醒

| 项 | 说明 |
|---|---|
| **tag 命名** | 必须是 `vX.Y.Z`（带 `v` 前缀），否则 CI 不会发版，只当普通构建 |
| **三处版本号一致** | 漏改任何一处会导致版本显示错乱或产物命名不一致 |
| **先 push main 再 push tag** | 若有未推送的 commit，必须先 `git push origin main`，否则 tag 指向的 commit 不在远程 |
| **只发版不验证？** | 可跳过第 3 步直接 `git push --tags`，但建议先推 main 确认普通构建绿了，避免发版构建中途失败 |
| **tag 不要随便移动** | 已发布的 tag 不要 `git tag -d` + 重建（会触发重复发版）。要重新发版请升新版本号打新 tag |

## 发版前本地自测（可选但推荐）

不想每次都等远程 40 分钟，可先本地构建验证：

```bash
cd apps/desktop
pnpm install
pnpm tauri build
```

本地产物位于 `apps/desktop/src-tauri/target/release/bundle/`。能正常出包即说明代码与配置无误，再走上面的远程发版流程。

## 版本号怎么选（语义化版本 SemVer）

- **PATCH `0.1.1`** — 修 bug、改文案、小修复（不新增功能）
- **MINOR `0.2.0`** — 新增功能（向后兼容）
- **MAJOR `1.0.0`** — 首个被认为稳定的正式版，或含不兼容改动

## 常见问题排查

- **Releases 页面为空 / 没生成** → 检查是否忘了 `tagName`/`releaseName` 配置（见 `build.yml` 的 `Build with Tauri` 步骤），或 tag 没带 `v` 前缀导致 CI 没进发版分支。
- **构建秒失败（<1 分钟）** → 多半是 CI 漏了 `pnpm install`。确保 `build.yml` 有 `pnpm install --frozen-lockfile` 步骤。
- **Actions 报 `Unexpected input(s) 'release'`** → tauri-action v0 没有 `release` 输入参数；用 `tagName`+`releaseName` 控制发版，而非 `release: true`。
- **macOS/Windows 下载后打不开** → 是正常的未签名提示。macOS 右键 → 打开可绕过；彻底解决需配置 Apple / Windows 代码签名证书 secret。
