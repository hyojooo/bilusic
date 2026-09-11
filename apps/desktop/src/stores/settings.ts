import { create } from "zustand";
import { persist } from "zustand/middleware";
import { setActiveMetadata, setActiveEngine, setBilibiliCookie } from "@/lib/bili";
import {
  applyTheme,
  DEFAULT_ACCENT,
  type ThemeMode,
} from "@/lib/theme";
import { invalidateHomeCache } from "@/lib/homeCache";
import i18n from "@/i18n";

export type LayoutType = "adaptive" | "compact" | "expanded";
export type Locale = "zh-CN" | "en-US";

export interface SettingsState {
  mode: ThemeMode;
  accent: string;
  layout: LayoutType;
  locale: Locale;
  /** selected metadata source plugin id (mirrors Spotube's "info source") */
  metadataSource: string;
  /** selected audio engine plugin id (mirrors Spotube's "YouTube engine") */
  audioEngine: string;
  /** login cookie (SESSDATA …) for Bili plugins; optional */
  biliCookie: string;
  setMode: (mode: ThemeMode) => void;
  setAccent: (accent: string) => void;
  setLayout: (layout: LayoutType) => void;
  setLocale: (locale: Locale) => void;
  /** push metadata+engine selection to the Rust coordinator (fire-and-forget) */
  setMetadataSource: (id: string) => void;
  setAudioEngine: (id: string) => void;
  setBiliCookie: (cookie: string) => void;
  /** push current settings to the DOM (call once on app start) */
  syncDom: () => void;
}

export const useSettings = create<SettingsState>()(
  persist(
    (set, get) => ({
      mode: "system",
      accent: DEFAULT_ACCENT,
      layout: "adaptive",
      locale: "zh-CN",
      metadataSource: "qq-music",
      audioEngine: "bilibili",
      biliCookie: "",

      setMode: (mode) => {
        set({ mode });
        get().syncDom();
      },
      setAccent: (accent) => {
        set({ accent });
        get().syncDom();
      },
      setLayout: (layout) => set({ layout }),
      setLocale: (locale) => {
        set({ locale });
        void i18n.changeLanguage(locale);
      },
      setMetadataSource: (id) => {
        set({ metadataSource: id });
        // 切源即清掉「新源」的本地首页缓存，强制首页重新拉取该源内容
        // （否则旧缓存会让首页内容看起来没变）。后端 `set_active_metadata`
        // 同时清掉进程内 home/toplist 缓存，双保险。后端`metadata_home`
        // 现在接受 source override（`metadataHome(id)` 必传 id），所以即使
        // 该 IPC 跑赢后端 `setActiveMetadata`，也不会拿到错源的内容。
        invalidateHomeCache(id);
        void setActiveMetadata(id);
      },
      setAudioEngine: (id) => {
        set({ audioEngine: id });
        void setActiveEngine(id);
      },
      setBiliCookie: (biliCookie) => {
        set({ biliCookie });
        void setBilibiliCookie(biliCookie);
      },

      syncDom: () => {
        const { mode, accent } = get();
        applyTheme(mode, accent);
      },
    }),
    {
      name: "bilusic-settings",
      onRehydrateStorage: () => (state) => {
        state?.syncDom();
        // Re-push selection + cookie + locale to the runtime after the store
        // is rehydrated from disk (the previous runtime is gone).
        //
        // Same launch race concern as `setMetadataSource`, but at cold-start:
        // the backend boots with `active_metadata == ""` and its fallback is
        // the first enabled source (qq-music). If React mounts Home and
        // calls `metadataHome()` before this `setActiveMetadata` lands,
        // the backend will respond with QQ's feed under default metadata.
        //
        // The fix that closes the race lives on the data path: Home now
        // passes its own `metadataSource` to `metadataHome(source)`, and
        // the backend `home_feed` honours that override over its
        // currently-active selection. So the response is guaranteed to
        // come from the source the user actually selected, even though
        // the switch-over IPC here is still fire-and-forget.
        if (state) {
          void i18n.changeLanguage(state.locale);
          invalidateHomeCache(state.metadataSource);
          void setActiveMetadata(state.metadataSource);
          void setActiveEngine(state.audioEngine);
          void setBilibiliCookie(state.biliCookie);
        }
      },
    }
  )
);
