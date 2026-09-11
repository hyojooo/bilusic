/**
 * i18next bootstrap for Bilusic.
 *
 * Two locales ship today (zh-CN default, en-US). The active language is read
 * from the persisted settings store so the very first render already uses the
 * user's choice — no flash of the wrong language.
 */
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zhCN from "./zh-CN";
import enUS from "./en-US";

type Locale = "zh-CN" | "en-US";

/** Read the persisted locale without importing the settings store (avoid cycle). */
function initialLocale(): Locale {
  try {
    const raw = localStorage.getItem("bilusic-settings");
    if (raw) {
      const parsed = JSON.parse(raw) as { state?: { locale?: string } };
      if (parsed?.state?.locale === "en-US" || parsed?.state?.locale === "zh-CN") {
        return parsed.state.locale;
      }
    }
  } catch {
    /* ignore corrupt storage */
  }
  return "zh-CN";
}

void i18n.use(initReactI18next).init({
  resources: {
    "zh-CN": { translation: zhCN },
    "en-US": { translation: enUS },
  },
  lng: initialLocale(),
  fallbackLng: "zh-CN",
  interpolation: { escapeValue: false },
  returnNull: false,
});

export default i18n;
export type { Locale };
