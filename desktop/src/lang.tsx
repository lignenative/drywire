// The language of the interface, chosen from the system's and changeable
// in the header. The choice is remembered in this window's storage.
import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";
import { detectLanguage, translate, type Key, type Lang } from "./i18n";
import { isAppError } from "./api";

const STORAGE_KEY = "drywire.language";

function initialLanguage(): Lang {
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved === "en" || saved === "fr") return saved;
  return detectLanguage(navigator.languages ?? [navigator.language]);
}

type Translate = (key: Key, vars?: Record<string, string | number>) => string;

interface LanguageContext {
  lang: Lang;
  setLang: (lang: Lang) => void;
  t: Translate;
}

const Context = createContext<LanguageContext | null>(null);

export function LanguageProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(initialLanguage);
  const setLang = useCallback((next: Lang) => {
    localStorage.setItem(STORAGE_KEY, next);
    document.documentElement.lang = next;
    setLangState(next);
  }, []);
  const value = useMemo<LanguageContext>(
    () => ({ lang, setLang, t: (key, vars) => translate(lang, key, vars) }),
    [lang, setLang],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useLanguage(): LanguageContext {
  const context = useContext(Context);
  if (!context) throw new Error("useLanguage outside LanguageProvider");
  return context;
}

/** A failure as a sentence in the interface's language, then the detail. */
export function errorText(t: Translate, error: unknown): string {
  if (!isAppError(error)) return `${t("error.unknown")} ${String(error)}`;
  const known = `error.${error.code}` as Key;
  const sentence = t(known);
  return sentence === known ? `${t("error.unknown")} ${error.message}` : `${sentence} ${error.message}`;
}
