<p align="center">
  <img src="./docs/assets/icon-128.png" width="120" alt="Bilusic logo">
</p>

<h1 align="center">Bilusic</h1>

<p align="center">
  A lightweight, open-source desktop music player.<br>
  Plug in your own audio sources, stream across services, and enjoy a clean, cross-platform experience.
</p>

<p align="center">
  <a href="https://github.com/hyojooo/bilusic/releases/latest">
    <img src="https://img.shields.io/github/v/release/hyojooo/bilusic" alt="release">
  </a>
  <a href="https://github.com/hyojooo/bilusic/releases">
    <img src="https://img.shields.io/github/downloads/hyojooo/bilusic/total" alt="downloads">
  </a>
  <a href="./LICENSE">
    <img src="https://img.shields.io/github/license/hyojooo/bilusic" alt="license">
  </a>
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey" alt="platform">
</p>

<p align="center">
  <strong>English</strong> · <a href="./README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <img src="./docs/assets/screenshot-light.jpg" width="720" alt="Bilusic screenshot">
</p>

## ✨ Features

- 🎵 **Pluggable Audio Sources** — Switch between multiple built-in source plugins as your playback backend.
- 🔌 **Plugin System** — Extend metadata providers and audio engines with declarative, JS, or WASM plugins.
- 🌗 **Dark / Light Themes** — Follow the system appearance or switch manually; UI adapts automatically.
- 📜 **Lyrics & Metadata** — Auto-match track info and synced lyrics; UI supports multiple languages.
- 🖥️ **Cross-Platform Native Feel** — Built with Tauri 2 (Rust + Web frontend). Small, fast, and native.
- 🔎 **Global Search** — Search songs, albums, playlists, and artists across sources.

## 🚀 Getting Started

### Development

```bash
cd apps/desktop
pnpm install
pnpm tauri dev        # Launch the desktop app (also starts Vite dev server)
```

To verify only the web frontend (no Rust toolchain needed):

```bash
cd apps/desktop
pnpm install
pnpm dev              # Open http://localhost:1420
```

### Build Release Artifacts

```bash
pnpm tauri build      # Outputs .dmg / .msi / .AppImage
```

> Windows requires [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/).
> Linux needs `libwebkit2gtk-4.1-dev` and other system dependencies (see the CI workflow).

## 🧱 Tech Stack

Tauri 2 (Rust) + React 18 + TypeScript + Vite + Tailwind CSS + Zustand · Package manager **pnpm**

## 📁 Project Structure

```
bilusic/
├── apps/desktop/            # Main Tauri project (frontend + Rust backend)
└── docs/                    # Plans, design docs, and screenshots
```

## 📄 License

Licensed under the [MIT License](./LICENSE).

## ⚠️ Disclaimer

This project is for **personal study and research only**. Please comply with the terms of service of the music services you connect to. Do not use this software for any commercial purpose or any act that infringes third-party rights. The project does not bundle or host any copyrighted content; all audio streams come from audio sources configured by the user.
