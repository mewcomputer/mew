import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import themes from "../themes.json";
import { getHostBootstrap, hostRequest, type HostTheme } from "./host";

// ---------------------------------------------------------------------------
// Theme definitions — loaded from themes.json
// ---------------------------------------------------------------------------

export interface ThemeDef {
  id: string;
  name: string;
  description: string;
  category: string;
  mode: ResolvedMode;
  baseText: string;
  preview: string[];
}

export const THEMES: ThemeDef[] = themes as ThemeDef[];

export const TITLE_FONT_CHOICES = [
  { id: "sentient", name: "Sentient", description: "Editorial display serif from the website.", family: '"Sentient", ui-serif, Georgia, serif' },
  { id: "instrument-sans", name: "Instrument Sans", description: "Clean humanist sans-serif.", family: '"Instrument Sans", "Noto Sans SC", ui-sans-serif, system-ui, sans-serif' },
  { id: "banga", name: "Banga", description: "Expressive display face.", family: '"Banga", "MiSans", ui-sans-serif, system-ui, sans-serif' },
  { id: "mi-sans", name: "Mi Sans", description: "Clean geometric sans-serif.", family: '"MiSans", ui-sans-serif, system-ui, sans-serif' },
  { id: "system", name: "System", description: "Use the platform font.", family: "ui-sans-serif, system-ui, sans-serif" },
  { id: "junicode", name: "Junicode", description: "A literary serif face.", family: '"Junicode", ui-serif, Georgia, serif' },
  { id: "goudy", name: "OFL Goudy", description: "A warm classic serif face.", family: '"OFLGoudy", ui-serif, Georgia, serif' },
] as const;

export const BODY_FONT_CHOICES = [
  { id: "instrument-sans", name: "Instrument Sans", description: "mew's default editorial sans-serif.", family: '"Instrument Sans", "Noto Sans SC", ui-sans-serif, system-ui, sans-serif' },
  { id: "system", name: "System", description: "Use the platform font.", family: "ui-sans-serif, system-ui, sans-serif" },
  { id: "mi-sans", name: "Mi Sans", description: "Clean geometric sans-serif.", family: '"MiSans", ui-sans-serif, system-ui, sans-serif' },
  { id: "sentient", name: "Sentient", description: "Warm editorial serif.", family: '"Sentient", ui-serif, Georgia, serif' },
  { id: "junicode", name: "Junicode", description: "A literary serif face.", family: '"Junicode", ui-serif, Georgia, serif' },
  { id: "goudy", name: "OFL Goudy", description: "A warm classic serif face.", family: '"OFLGoudy", ui-serif, Georgia, serif' },
] as const;

export const FONT_CHOICES = BODY_FONT_CHOICES;

export type TitleFontChoiceId = (typeof TITLE_FONT_CHOICES)[number]["id"];
export type BodyFontChoiceId = (typeof BODY_FONT_CHOICES)[number]["id"];
export type FontChoiceId = BodyFontChoiceId;

// ---------------------------------------------------------------------------
// Color utilities (for light/dark detection only)
// ---------------------------------------------------------------------------

/** Relative luminance for determining light vs dark. */
function luminance(hex: string): number {
  const h = hex.replace("#", "");
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  const toLin = (v: number) => {
    v /= 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * toLin(r) + 0.7152 * toLin(g) + 0.0722 * toLin(b);
}

// ---------------------------------------------------------------------------
// Theme context
// ---------------------------------------------------------------------------

export type ResolvedMode = "light" | "dark";

interface ThemeContextValue {
  /** The selected theme id (e.g. "catppuccin-mocha"). */
  themeId: string;
  setThemeId: (id: string) => void;
  /** The resolved ThemeDef object. */
  theme: ThemeDef;
  /** Whether the current theme is light or dark. */
  mode: ResolvedMode;
  themes: ThemeDef[];
  themeMode: "system" | ResolvedMode;
  setThemeMode: (mode: "system" | ResolvedMode) => void;
  lightTheme: string;
  darkTheme: string;
  setLightTheme: (id: string) => void;
  setDarkTheme: (id: string) => void;
  titleFontId: TitleFontChoiceId;
  setTitleFontId: (id: TitleFontChoiceId) => void;
  bodyFontId: BodyFontChoiceId;
  setBodyFontId: (id: BodyFontChoiceId) => void;
  fontId: FontChoiceId;
  setFontId: (id: FontChoiceId) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

const STORAGE_KEY = "mew-theme-id";
const DEFAULT_THEME = "codex-dark";
const APPEARANCE_STORAGE_KEY = "mew-appearance";
const FONT_STORAGE_KEY = "mew-font-choice";
const TITLE_FONT_STORAGE_KEY = "mew-title-font-choice";
const BODY_FONT_STORAGE_KEY = "mew-body-font-choice";
const DEFAULT_TITLE_FONT: TitleFontChoiceId = "sentient";
const DEFAULT_BODY_FONT: BodyFontChoiceId = "instrument-sans";

function hostThemeDefinition(value: HostTheme): ThemeDef {
  const builtin = THEMES.find((theme) => theme.id === value.id);
  if (builtin) return builtin;
  const color = (name: string, fallback: string) => value.css.match(new RegExp(`--${name}:\\s*(#[a-fA-F0-9]{6})`))?.[1] ?? fallback;
  return { id: value.id, name: value.id, description: "Installed theme", category: "Custom", mode: value.mode, baseText: color("foreground", "#ffffff"), preview: [color("background", "#181818"), color("primary", "#aaaaaa"), color("text-success", "#888888")] };
}

function getInitialTitleFontId(): TitleFontChoiceId {
  if (typeof window === "undefined") return DEFAULT_TITLE_FONT;
  const stored = localStorage.getItem(TITLE_FONT_STORAGE_KEY);
  if (TITLE_FONT_CHOICES.some((font) => font.id === stored)) {
    return stored as TitleFontChoiceId;
  }
  const legacy = localStorage.getItem(FONT_STORAGE_KEY);
  if (legacy === "mi-sans") return "banga";
  if (legacy && TITLE_FONT_CHOICES.some((font) => font.id === legacy)) {
    return legacy as TitleFontChoiceId;
  }
  return DEFAULT_TITLE_FONT;
}

function getInitialBodyFontId(): BodyFontChoiceId {
  if (typeof window === "undefined") return DEFAULT_BODY_FONT;
  const stored = localStorage.getItem(BODY_FONT_STORAGE_KEY) ?? localStorage.getItem(FONT_STORAGE_KEY);
  if (BODY_FONT_CHOICES.some((font) => font.id === stored)) {
    return stored as BodyFontChoiceId;
  }
  return DEFAULT_BODY_FONT;
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const host = getHostBootstrap();
  const availableThemes = useMemo(() => host?.themes.length ? host.themes.map(hostThemeDefinition) : THEMES, [host]);
  const [preferences, setPreferences] = useState(() => {
    const stored = localStorage.getItem(STORAGE_KEY);
    const validStored = availableThemes.find((theme) => theme.id === stored);
    const storedMode = validStored?.mode ?? (validStored && luminance(validStored.preview[0] ?? "#000000") < 0.5 ? "dark" : "light");
    const defaults = {
      themeMode: host?.themeMode ?? (validStored ? storedMode : "system") as "system" | ResolvedMode,
      lightTheme: host?.lightTheme ?? (validStored && storedMode === "light" ? validStored.id : "codex-light"),
      darkTheme: host?.darkTheme ?? (validStored && storedMode === "dark" ? validStored.id : DEFAULT_THEME),
    };
    if (host) return defaults;
    try {
      const saved: unknown = JSON.parse(localStorage.getItem(APPEARANCE_STORAGE_KEY) ?? "null");
      if (saved && typeof saved === "object" && "themeMode" in saved && "lightTheme" in saved && "darkTheme" in saved
        && ["system", "light", "dark"].includes(String(saved.themeMode))
        && typeof saved.lightTheme === "string" && typeof saved.darkTheme === "string") {
        return { themeMode: saved.themeMode as "system" | ResolvedMode, lightTheme: saved.lightTheme, darkTheme: saved.darkTheme };
      }
    } catch { /* Use the last valid preference after malformed browser storage. */ }
    return defaults;
  });
  const [systemDark, setSystemDark] = useState(() => window.matchMedia("(prefers-color-scheme: dark)").matches);
  const [titleFontId, setTitleFontIdState] = useState<TitleFontChoiceId>(getInitialTitleFontId);
  const [bodyFontId, setBodyFontIdState] = useState<BodyFontChoiceId>(getInitialBodyFontId);
  const mode: ResolvedMode = preferences.themeMode === "system" ? (systemDark ? "dark" : "light") : preferences.themeMode;
  const themeId = mode === "dark" ? preferences.darkTheme : preferences.lightTheme;
  const theme = availableThemes.find((item) => item.id === themeId) ?? availableThemes[0]!;

  const savePreferences = (next: typeof preferences) => {
    setPreferences(next);
    localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(next));
    localStorage.setItem(STORAGE_KEY, next.themeMode === "dark" ? next.darkTheme : next.lightTheme);
    if (host) void hostRequest({ type: "set_theme", ...next }).catch((error: unknown) => {
      console.error("Could not save appearance preferences", error);
    });
  };
  const setThemeId = (id: string) => {
    const selected = availableThemes.find((item) => item.id === id);
    if (!selected) return;
    const selectedMode = host?.themes.find((item) => item.id === id)?.mode
      ?? selected.mode
      ?? (luminance(selected.preview[0] ?? "#000000") < 0.5 ? "dark" : "light");
    savePreferences({ ...preferences, themeMode: selectedMode, [selectedMode === "dark" ? "darkTheme" : "lightTheme"]: id });
  };
  const setThemeMode = (themeMode: "system" | ResolvedMode) => savePreferences({ ...preferences, themeMode });
  const setLightTheme = (id: string) => {
    if (!availableThemes.some((item) => item.id === id)) return;
    savePreferences({ ...preferences, lightTheme: id });
  };
  const setDarkTheme = (id: string) => {
    if (!availableThemes.some((item) => item.id === id)) return;
    savePreferences({ ...preferences, darkTheme: id });
  };

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const update = () => setSystemDark(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  useEffect(() => {
    if (!host) return;
    const style = document.createElement("style");
    style.dataset.mewHostThemes = "";
    style.textContent = host.themes.map((item) => `[data-theme="${CSS.escape(item.id)}"] { ${item.css} }`).join("\n");
    document.head.append(style);
    return () => style.remove();
  }, [host]);

  const setTitleFontId = (id: TitleFontChoiceId) => {
    setTitleFontIdState(id);
    localStorage.setItem(TITLE_FONT_STORAGE_KEY, id);
  };

  const setBodyFontId = (id: BodyFontChoiceId) => {
    setBodyFontIdState(id);
    localStorage.setItem(BODY_FONT_STORAGE_KEY, id);
    localStorage.setItem(FONT_STORAGE_KEY, id);
  };

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme.id);
    document.documentElement.classList.toggle("dark", mode === "dark");
    document.documentElement.style.colorScheme = mode;
  }, [theme.id, mode]);

  useEffect(() => {
    document.documentElement.setAttribute("data-title-font", titleFontId);
    document.documentElement.setAttribute("data-body-font", bodyFontId);
    document.documentElement.setAttribute("data-font", bodyFontId);
  }, [titleFontId, bodyFontId]);

  return (
    <ThemeContext.Provider value={{
      themeId: theme.id,
      setThemeId,
      theme,
      mode,
      themes: availableThemes,
      themeMode: preferences.themeMode,
      setThemeMode,
      lightTheme: preferences.lightTheme,
      darkTheme: preferences.darkTheme,
      setLightTheme,
      setDarkTheme,
      titleFontId,
      setTitleFontId,
      bodyFontId,
      setBodyFontId,
      fontId: bodyFontId,
      setFontId: setBodyFontId,
    }}>
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) {
    throw new Error("useTheme must be used within a ThemeProvider");
  }
  return ctx;
}
