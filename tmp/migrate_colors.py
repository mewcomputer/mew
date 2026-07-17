#!/usr/bin/env python3
"""Bulk-migrate direct Color::X usages in mew-tui UI code to theme token lookups."""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
UI_DIR = ROOT / "crates" / "mew-tui" / "src" / "ui"
SETTINGS = ROOT / "crates" / "mew-tui" / "src" / "settings.rs"

# Files to migrate.
FILES = list(UI_DIR.glob("*.rs")) + [SETTINGS]

# Mapping for standalone Color::X usages.
# The replacement string uses a placeholder `{var}` which will be resolved
# to the theme variable in scope (tokens / app.theme / theme).
COLOR_MAP = {
    "Color::Cyan": '{var}.resolve("text.accent")',
    "Color::Green": '{var}.resolve("text.success")',
    "Color::Yellow": '{var}.resolve("text.warning")',
    "Color::Red": '{var}.resolve("text.error")',
    "Color::DarkGray": '{var}.resolve("text.muted")',
    "Color::Gray": '{var}.resolve("text.placeholder")',
    "Color::White": '{var}.resolve("text.body")',
    "Color::Black": '{var}.resolve("text.inverse")',
}

# Mapping for specific RGB values that correspond to known tokens.
RGB_MAP = {
    "Color::Rgb(30, 30, 35)": '{var}.resolve("markdown.code_fence.bg")',
    "Color::Rgb(22, 22, 26)": '{var}.resolve("panel.background")',
    "Color::Rgb(50, 50, 56)": '{var}.resolve("tool.background")',
    "Color::Rgb(35, 90, 50)": '{var}.resolve("surface.success")',
    "Color::Rgb(90, 35, 35)": '{var}.resolve("surface.error")',
    "Color::Rgb(90, 60, 35)": '{var}.resolve("surface.warning")',
}


def resolve_var(line: str) -> str:
    """Pick the best theme variable name for this line."""
    if "app.theme" in line:
        return "app.theme"
    if "theme.with_persona_accent" in line or "persona_theme" in line:
        return "theme"
    # If the function already binds `theme`, prefer it; otherwise fall back to `app.theme`.
    if re.search(r"\btheme\s*:\s*&(Theme|crate::theme::Theme)\b", line):
        return "theme"
    if "theme: &Theme" in line or "theme: &crate::theme::Theme" in line:
        return "theme"
    if re.search(r"\btokens\s*:\s*&ThemeTokens\b", line):
        return "tokens.theme"
    if re.search(r"\btokens\b", line):
        return "tokens.theme"
    return "app.theme"


def migrate_file(path: Path) -> int:
    text = path.read_text()
    original = text

    # Replace known RGB values first.
    for rgb, tmpl in RGB_MAP.items():
        if rgb in text:
            # Determine var per occurrence by looking at the line.
            lines = text.splitlines()
            new_lines = []
            for line in lines:
                if rgb in line:
                    var = resolve_var(line)
                    line = line.replace(rgb, tmpl.format(var=var))
                new_lines.append(line)
            text = "\n".join(new_lines) + ("\n" if text.endswith("\n") else "")

    # Replace standalone Color::X.
    for color, tmpl in COLOR_MAP.items():
        if color in text:
            lines = text.splitlines()
            new_lines = []
            for line in lines:
                if color in line:
                    var = resolve_var(line)
                    line = line.replace(color, tmpl.format(var=var))
                new_lines.append(line)
            text = "\n".join(new_lines) + ("\n" if text.endswith("\n") else "")

    if text != original:
        path.write_text(text)
        return 1
    return 0


def main():
    changed = 0
    for path in FILES:
        changed += migrate_file(path)
    print(f"Changed {changed} files")


if __name__ == "__main__":
    main()
