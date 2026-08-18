import { createContext, useContext, useState, useEffect, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ru } from "./ru";
import { en } from "./en";

export type Locale = "ru" | "en";

const dictionaries = { ru, en } as const;

interface I18nContextValue {
  locale: Locale;
  setLocale: (l: Locale) => void;
  t: (key: string, params?: Record<string, string | number>) => string;
}

const I18nContext = createContext<I18nContextValue>(null!);

const STORAGE_KEY = "app_locale";

function getNestedValue(obj: any, path: string): string {
  const parts = path.split(".");
  let current = obj;
  for (const part of parts) {
    if (current == null) return path;
    current = current[part];
  }
  return typeof current === "string" ? current : path;
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(() => {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "en" || saved === "ru") return saved;
    return "en";
  });

  const setLocale = (l: Locale) => {
    setLocaleState(l);
    localStorage.setItem(STORAGE_KEY, l);
    // Sync locale to backend
    if ("__TAURI_INTERNALS__" in window) {
      invoke("set_locale", { locale: l }).catch(() => {});
    }
  };

  // Sync locale to backend on mount
  useEffect(() => {
    if ("__TAURI_INTERNALS__" in window) {
      invoke("set_locale", { locale }).catch(() => {});
    }
  }, []);

  const t = (key: string, params?: Record<string, string | number>): string => {
    let value = getNestedValue(dictionaries[locale], key);
    if (params) {
      for (const [k, v] of Object.entries(params)) {
        value = value.replace(`{${k}}`, String(v));
      }
    }
    return value;
  };

  return (
    <I18nContext.Provider value={{ locale, setLocale, t }}>
      {children}
    </I18nContext.Provider>
  );
}

export function useI18n() {
  return useContext(I18nContext);
}

export function useT() {
  const { t } = useContext(I18nContext);
  return t;
}
