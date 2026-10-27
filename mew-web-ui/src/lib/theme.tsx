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
  baseText: string;
  preview: string[];
}

export const THEMES: ThemeDef[] = themes as ThemeDef[];

export const FONT_CHOICES = [
  { id: "system", name: "System", description: "Use the platform font." },
  { id: "mi-sans", name: "Mi Sans", description: "mew's default sans-serif." },
  { id: "junicode", name: "Junicode", description: "A literary serif face." },
  { id: "goudy", name: "OFL Goudy", description: "A warm classic serif face." },
] as const;

export type FontChoiceId = (typeof FONT_CHOICES)[number]["id"];

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
  fontId: FontChoiceId;
  setFontId: (id: FontChoiceId) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

const STORAGE_KEY = "mew-theme-id";
const DEFAULT_THEME = "codex-dark";
const APPEARANCE_STORAGE_KEY = "mew-appearance";
const FONT_STORAGE_KEY = "mew-font-choice";
const DEFAULT_FONT: FontChoiceId = "system";

function hostThemeDefinition(value: HostTheme): ThemeDef {
  const builtin = THEMES.find((theme) => theme.id === value.id);
  if (builtin) return builtin;
  const color = (name: string, fallback: string) => value.css.match(new RegExp(`--${name}:\\s*(#[a-fA-F0-9]{6})`))?.[1] ?? fallback;
  return { id: value.id, name: value.id, description: "Installed theme", category: "Custom", baseText: color("foreground", "#ffffff"), preview: [color("background", "#181818"), color("primary", "#aaaaaa"), color("text-success", "#888888")] };
}

function getInitialFontId(): FontChoiceId {
  if (typeof window === "undefined") return DEFAULT_FONT;
  const stored = localStorage.getItem(FONT_STORAGE_KEY);
  return FONT_CHOICES.some((font) => font.id === stored)
    ? (stored as FontChoiceId)
    : DEFAULT_FONT;
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const host = getHostBootstrap();
  const availableThemes = useMemo(() => host?.themes.length ? host.themes.map(hostThemeDefinition) : THEMES, [host]);
  const [preferences, setPreferences] = useState(() => {
    const stored = localStorage.getItem(STORAGE_KEY);
    const validStored = availableThemes.find((theme) => theme.id === stored);
    const storedMode = validStored && luminance(validStored.preview[0] ?? "#000000") < 0.5 ? "dark" : "light";
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
  const [fontId, setFontIdState] = useState<FontChoiceId>(getInitialFontId);
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
      ?? (luminance(selected.preview[0] ?? "#000000") < 0.5 ? "dark" : "light");
    savePreferences({ ...preferences, themeMode: selectedMode, [selectedMode === "dark" ? "darkTheme" : "lightTheme"]: id });
  };
  const setThemeMode = (themeMode: "system" | ResolvedMode) => savePreferences({ ...preferences, themeMode });

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

  const setFontId = (id: FontChoiceId) => {
    setFontIdState(id);
    localStorage.setItem(FONT_STORAGE_KEY, id);
  };

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme.id);
    document.documentElement.classList.toggle("dark", mode === "dark");
    document.documentElement.style.colorScheme = mode;
  }, [theme.id, mode]);

  useEffect(() => {
    document.documentElement.setAttribute("data-font", fontId);
  }, [fontId]);

  return (
    <ThemeContext.Provider value={{ themeId: theme.id, setThemeId, theme, mode, themes: availableThemes, themeMode: preferences.themeMode, setThemeMode, fontId, setFontId }}>
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
