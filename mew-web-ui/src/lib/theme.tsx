import {
  createContext,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import themes from "../themes.json";

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
  fontId: FontChoiceId;
  setFontId: (id: FontChoiceId) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

const STORAGE_KEY = "mew-theme-id";
const DEFAULT_THEME = "catppuccin-mocha";
const FONT_STORAGE_KEY = "mew-font-choice";
const DEFAULT_FONT: FontChoiceId = "mi-sans";

function getInitialThemeId(): string {
  if (typeof window === "undefined") return DEFAULT_THEME;
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored && THEMES.some((t) => t.id === stored)) return stored;
  return DEFAULT_THEME;
}

function getInitialFontId(): FontChoiceId {
  if (typeof window === "undefined") return DEFAULT_FONT;
  const stored = localStorage.getItem(FONT_STORAGE_KEY);
  return FONT_CHOICES.some((font) => font.id === stored)
    ? (stored as FontChoiceId)
    : DEFAULT_FONT;
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [themeId, setThemeIdState] = useState<string>(getInitialThemeId);
  const [fontId, setFontIdState] = useState<FontChoiceId>(getInitialFontId);

  const theme = THEMES.find((t) => t.id === themeId) ?? THEMES[0]!;
  const mode: ResolvedMode =
    luminance(theme.preview[0] ?? "#000000") < 0.5 ? "dark" : "light";

  const setThemeId = (id: string) => {
    setThemeIdState(id);
    localStorage.setItem(STORAGE_KEY, id);
  };

  const setFontId = (id: FontChoiceId) => {
    setFontIdState(id);
    localStorage.setItem(FONT_STORAGE_KEY, id);
  };

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme.id);
  }, [theme]);

  useEffect(() => {
    document.documentElement.setAttribute("data-font", fontId);
  }, [fontId]);

  return (
    <ThemeContext.Provider value={{ themeId, setThemeId, theme, mode, fontId, setFontId }}>
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
