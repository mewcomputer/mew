#!/usr/bin/env python3
"""Generate theme_manifest.json from existing web CSS and TUI themes."""

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WEB_CSS = ROOT / "mew-web-ui" / "src" / "index.css"
TUI_RESOURCES = ROOT / "crates" / "mew-tui" / "resources" / "themes"
OUT = ROOT / "crates" / "mew-tui" / "resources" / "theme_manifest.json"


def hsl_to_hex(h: float, s: float, l: float) -> str:
    """Convert HSL (h in deg, s/l in %) to #rrggbb hex."""
    s /= 100.0
    l /= 100.0
    c = (1 - abs(2 * l - 1)) * s
    x = c * (1 - abs((h / 60) % 2 - 1))
    m = l - c / 2
    if h < 60:
        r1, g1, b1 = c, x, 0
    elif h < 120:
        r1, g1, b1 = x, c, 0
    elif h < 180:
        r1, g1, b1 = 0, c, x
    elif h < 240:
        r1, g1, b1 = 0, x, c
    elif h < 300:
        r1, g1, b1 = x, 0, c
    else:
        r1, g1, b1 = c, 0, x
    r = round((r1 + m) * 255)
    g = round((g1 + m) * 255)
    b = round((b1 + m) * 255)
    return f"#{max(0, min(255, r)):02x}{max(0, min(255, g)):02x}{max(0, min(255, b)):02x}"


def parse_hsl(value: str) -> str:
    """Parse hsl(...) or hsla(...) to hex."""
    m = re.match(r"hsla?\(\s*([\d.]+)\s+([\d.]+)%\s+([\d.]+)%", value)
    if not m:
        raise ValueError(f"Cannot parse HSL: {value}")
    return hsl_to_hex(float(m.group(1)), float(m.group(2)), float(m.group(3)))


def css_var_to_token(name: str) -> str:
    """Convert --some-name to a manifest token key."""
    key = name.removeprefix("--")
    return key.replace("-", "_")


# ---------------------------------------------------------------------------
# Base tokens and aliases
# ---------------------------------------------------------------------------

base_tokens = {
    # Core shadcn — exact values from TUI Theme::dark()
    "background": "#1e1e21",
    "foreground": "#ffffff",
    "card": "#323238",
    "card_foreground": "@foreground",
    "popover": "#1e1e21",
    "popover_foreground": "@foreground",
    "primary": "#00ffff",
    "primary_foreground": "#1e1e21",
    "secondary": "#323238",
    "secondary_foreground": "@foreground",
    "muted": "#28282c",
    "muted_foreground": "#a9a9a9",
    "accent": "#323238",
    "accent_foreground": "@foreground",
    "destructive": "#8c1e1e",
    "destructive_foreground": "@foreground",
    "border": "#323237",
    "input": "#1e1e21",
    "ring": "#00ffff",
    "sidebar": "#1c1c1f",
    "sidebar_foreground": "@foreground",
    "sidebar_primary": "#00ffff",
    "sidebar_primary_foreground": "#1c1c1f",
    "sidebar_accent": "#323238",
    "sidebar_accent_foreground": "@foreground",
    "sidebar_border": "@border",
    "sidebar_ring": "@ring",
    "radius": "0.625rem",
    "chart_1": "#96e6a0",
    "chart_2": "#96bef0",
    "chart_3": "#f5c850",
    "chart_4": "#c8aaf0",

    # mew surface extensions
    "panel.background": "@background",
    "panel.overlay": "@card",
    "panel.overlay_hover": "@background",
    "tool.background": "@card",
    "tool.border": "@border",
    "status_bar.background": "@background",
    "sidebar.background": "@sidebar",
    "divider": "@border",
    "surface.success": "@green.bg",
    "surface.error": "@red.bg",
    "surface.inverse": "@foreground",

    # Selection
    "selection.foreground": "@background",
    "selection.background": "@foreground",

    # Semantic text
    "text.body": "@foreground",
    "text.muted": "@muted_foreground",
    "text.placeholder": "@muted_foreground",
    "text.disabled": "@muted_foreground",
    "text.accent": "@primary",
    "text.success": "@green.fg",
    "text.warning": "@yellow.fg",
    "text.error": "@red.fg",
    "text.inverse": "@background",

    # Color scales — exact values from TUI Theme::dark()
    "red.fg": "#fff0f0",
    "red.med": "#b42828",
    "red.bg": "#8c1e1e",
    "orange.fg": "#ffb347",
    "orange.med": "#cc8400",
    "orange.bg": "#4a2e00",
    "green.fg": "#96e6a0",
    "green.med": "#287832",
    "green.bg": "#194623",
    "yellow.fg": "#f5d26e",
    "yellow.med": "#c8a028",
    "yellow.bg": "#4b3c14",
    "blue.fg": "#96bef0",
    "blue.med": "#325a8c",
    "blue.bg": "#1e375a",
    "purple.fg": "#f0e6fa",
    "purple.med": "#5f3282",
    "purple.bg": "#37234b",
    "cyan.fg": "#00ffff",
    "cyan.med": "#287878",
    "cyan.bg": "#143232",

    # ANSI palette (dark defaults from Ayu Dark, a sensible terminal palette)
    "terminal.black": "#0d1016",
    "terminal.red": "#ef7177",
    "terminal.green": "#aad84c",
    "terminal.yellow": "#feb454",
    "terminal.blue": "#5ac1fe",
    "terminal.magenta": "#39bae5",
    "terminal.cyan": "#95e5cb",
    "terminal.white": "#bfbdb6",
    "terminal.bright_black": "#545557",
    "terminal.bright_red": "#83353b",
    "terminal.bright_green": "#567627",
    "terminal.bright_yellow": "#92582b",
    "terminal.bright_blue": "#27618c",
    "terminal.bright_magenta": "#205a78",
    "terminal.bright_cyan": "#4c806f",
    "terminal.bright_white": "#fafafa",
    "terminal.dim_black": "#bfbdb6",
    "terminal.dim_red": "#febab9",
    "terminal.dim_green": "#d8eca8",
    "terminal.dim_yellow": "#ffd9aa",
    "terminal.dim_blue": "#b7dffe",
    "terminal.dim_magenta": "#addcf3",
    "terminal.dim_cyan": "#cbf2e4",
    "terminal.dim_white": "#787876",

    # Pills
    "pill.auto.fg": "@purple.fg",
    "pill.auto.bg": "@purple.bg",
    "pill.permissive.fg": "@green.fg",
    "pill.permissive.bg": "@green.bg",
    "pill.dangerous.fg": "@red.fg",
    "pill.dangerous.bg": "@red.bg",
    "pill.model.fg": "@green.fg",
    "pill.model.bg": "@green.bg",
    "pill.thinking.fg": "@yellow.fg",
    "pill.thinking.bg": "@yellow.bg",
    "pill.cwd.fg": "@blue.fg",
    "pill.cwd.bg": "@blue.bg",
    "pill.git.fg": "@orange.fg",
    "pill.git.bg": "@orange.bg",
    "pill.attention.fg": "@yellow.fg",
    "pill.attention.bg": "@yellow.bg",
    "pill.persona.fg": "@persona.accent.fg",
    "pill.persona.bg": "@persona.accent.bg",
    "pill.custom.fg": "@text.body",
    "pill.custom.bg": "@muted",

    # Markdown
    "markdown.paragraph": "@text.body",
    "markdown.heading.foreground": "@text.body",
    "markdown.heading.h1": "@text.body",
    "markdown.heading.h2": "@text.body",
    "markdown.heading.h3": "@text.body",
    "markdown.heading.h4": "@text.body",
    "markdown.heading.h5": "@text.body",
    "markdown.heading.h6": "@text.body",
    "markdown.emphasis": "@text.body",
    "markdown.strong": "@text.body",
    "markdown.strikethrough": "@text.muted",
    "markdown.inline_code.fg": "@text.body",
    "markdown.inline_code.bg": "@muted",
    "markdown.link_text": "@primary",
    "markdown.link_url": "@text.muted",
    "markdown.list_bullet": "@text.muted",
    "markdown.block_quote": "@text.muted",
    "markdown.thematic_break": "@divider",
    "markdown.table_header": "@text.body",
    "markdown.table_cell": "@text.body",
    "markdown.table_border": "@divider",
    "markdown.code_fence.fg": "@text.body",
    "markdown.code_fence.bg": "#1e1e23",
    "markdown.code_fence.border": "@border",
    "markdown.pending_indicator": "@text.warning",

    # Syntax (TextMate scopes)
    "syntax.comment": "@text.muted",
    "syntax.string": "@green.fg",
    "syntax.keyword": "@red.fg",
    "syntax.function": "@blue.fg",
    "syntax.number": "@purple.fg",
    "syntax.type": "@yellow.fg",
    "syntax.variable": "@text.body",
    "syntax.operator": "@text.body",
    "syntax.constant": "@purple.fg",

    # Persona accent placeholders (overridden at runtime by with_persona_accent)
    "persona.accent.fg": "@primary_foreground",
    "persona.accent.bg": "@primary",
}

# Light overrides — exact values from TUI Theme::light()
light_overrides = {
    "background": "#ffffff",
    "foreground": "#0f0f14",
    "card": "#f5f5f8",
    "popover": "#ffffff",
    "primary": "#1e1e28",
    "primary_foreground": "#f5f5fa",
    "secondary": "#f0f0f4",
    "muted": "#ebebef",
    "muted_foreground": "#64646e",
    "accent": "#e6e6ec",
    "destructive": "#c84040",
    "border": "#d2d2d8",
    "input": "#f5f5f8",
    "ring": "@primary",
    "sidebar": "#f8f8fa",
    "sidebar_primary": "@primary",
    "sidebar_primary_foreground": "@sidebar",
    "sidebar_accent": "@accent",
    "sidebar_border": "@border",
    "sidebar_ring": "@ring",
    "red.fg": "#8b2020",
    "red.med": "#d85c5c",
    "red.bg": "#f9e3e3",
    "green.fg": "#1f6e2e",
    "green.med": "#4caf50",
    "green.bg": "#e8f5e9",
    "yellow.fg": "#7a5c00",
    "yellow.med": "#d4ac0d",
    "yellow.bg": "#fcf3cf",
    "blue.fg": "#1e4a8c",
    "blue.med": "#4a90e2",
    "blue.bg": "#e3f2fd",
    "purple.fg": "#6a2c85",
    "purple.med": "#9b59b6",
    "purple.bg": "#f3e5f5",
    "cyan.fg": "#00695c",
    "cyan.med": "#26a69a",
    "cyan.bg": "#e0f2f1",
    "markdown.code_fence.bg": "@background",
}

# ---------------------------------------------------------------------------
# Parse web CSS themes
# ---------------------------------------------------------------------------


def parse_css_themes(css_path: Path) -> dict:
    text = css_path.read_text()
    pattern = r'\[data-theme="([^"]+)"\]\s*\{([^}]*)\}'
    themes = {}
    for name, body in re.findall(pattern, text, re.DOTALL):
        tokens = {}
        for line in body.splitlines():
            line = line.strip()
            if not line or line.startswith("/*"):
                continue
            if ":" not in line or not line.endswith(";"):
                continue
            var, raw = line[:-1].split(":", 1)
            var = var.strip()
            raw = raw.strip()
            if not var.startswith("--"):
                continue
            key = css_var_to_token(var)
            try:
                if raw.startswith("hsl"):
                    value = parse_hsl(raw)
                elif raw.startswith("#"):
                    if len(raw) == 7:
                        value = raw
                    elif len(raw) == 4:
                        value = f"#{raw[1]*2}{raw[2]*2}{raw[3]*2}"
                    else:
                        continue
                elif raw.endswith("rem"):
                    # e.g. radius
                    value = raw
                else:
                    continue
            except ValueError as e:
                print(f"Skipping {name} {var}: {e}")
                continue
            tokens[key] = value
        if tokens:
            themes[name] = tokens
    return themes


# ---------------------------------------------------------------------------
# Build themes as sparse overrides
# ---------------------------------------------------------------------------


def resolve_flat(tokens: dict) -> dict:
    """Resolve aliases in a token table to a flat hex table."""
    flat = {}
    for k, v in tokens.items():
        if isinstance(v, str) and v.startswith("@"):
            seen = set()
            cur = v
            while cur.startswith("@"):
                if cur in seen:
                    raise ValueError(f"Cycle involving {k}")
                seen.add(cur)
                cur = tokens.get(cur[1:], cur)
            flat[k] = cur
        else:
            flat[k] = v
    return flat


def sparse_override(theme_tokens: dict, base_tokens: dict) -> dict:
    """Return only the tokens that differ from base after alias resolution."""
    base_flat = resolve_flat(base_tokens)
    theme_flat = resolve_flat(theme_tokens)
    diff = {}
    for k, v in theme_flat.items():
        if base_flat.get(k) != v:
            diff[k] = theme_tokens.get(k, v)
    return diff


def build_manifest():
    css_themes = parse_css_themes(WEB_CSS)

    manifest = {
        "version": 1,
        "tokens": base_tokens,
        "themes": {},
    }

    manifest["themes"]["dark"] = {"mode": "dark", "tokens": {}}

    light_tokens = dict(base_tokens)
    light_tokens.update(light_overrides)
    manifest["themes"]["light"] = {
        "mode": "light",
        "tokens": sparse_override(light_tokens, base_tokens),
    }

    theme_id_to_base = {
        "catppuccin-latte": "light",
        "catppuccin-frappe": "dark",
        "catppuccin-macchiato": "dark",
        "catppuccin-mocha": "dark",
        "evergarden-fall": "dark",
        "evergarden-spring": "dark",
        "evergarden-summer": "light",
        "evergarden-winter": "dark",
        "rose-pine": "dark",
        "rose-pine-moon": "dark",
        "rose-pine-dawn": "light",
        "tokyo-night": "dark",
        "tokyo-night-storm": "dark",
        "tokyo-night-light": "light",
        "ayu-dark": "dark",
        "ayu-mirage": "dark",
        "ayu-light": "light",
        "nord": "dark",
        "gruvbox-dark": "dark",
        "github-dark": "dark",
        "dracula": "dark",
    }

    for theme_id, base_name in theme_id_to_base.items():
        css_tokens = css_themes.get(theme_id, {})
        if not css_tokens:
            print(f"Warning: no CSS tokens found for {theme_id}")
            continue
        merged = dict(base_tokens)
        merged.update(css_tokens)
        diff = sparse_override(merged, base_tokens)
        manifest["themes"][theme_id] = {
            "mode": manifest["themes"][base_name]["mode"],
            "base": base_name,
            "tokens": diff,
        }

    # Override overlap themes with TUI JSON data if present.
    for tui_name in ["tokyo-night", "catppuccin-mocha", "catppuccin-latte"]:
        path = TUI_RESOURCES / f"{tui_name}.json"
        if path.exists():
            data = json.loads(path.read_text())
            tui_tokens = data.get("tokens", {})
            flat_tui = {}
            for k, v in tui_tokens.items():
                if isinstance(v, dict) and set(v.keys()) == {"fg", "med", "bg"}:
                    flat_tui[f"{k}.fg"] = v["fg"]
                    flat_tui[f"{k}.med"] = v["med"]
                    flat_tui[f"{k}.bg"] = v["bg"]
                else:
                    flat_tui[k] = v
            merged = dict(base_tokens)
            merged.update(flat_tui)
            diff = sparse_override(merged, base_tokens)
            manifest["themes"][tui_name]["tokens"] = diff

    OUT.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Wrote {OUT}")
    print(f"Themes: {list(manifest['themes'].keys())}")


if __name__ == "__main__":
    build_manifest()
