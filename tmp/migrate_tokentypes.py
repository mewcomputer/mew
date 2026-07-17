#!/usr/bin/env python3
"""Migrate ThemeTokens parameters to Theme and replace direct colors."""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FILES = [
    ROOT / "crates" / "mew-tui" / "src" / "ui" / "overlays.rs",
    ROOT / "crates" / "mew-tui" / "src" / "ui" / "status.rs",
    ROOT / "crates" / "mew-tui" / "src" / "settings.rs",
]

COLOR_MAP = {
    "Color::Cyan": '"text.accent"',
    "Color::Green": '"text.success"',
    "Color::Yellow": '"text.warning"',
    "Color::Red": '"text.error"',
    "Color::DarkGray": '"text.muted"',
    "Color::Gray": '"text.placeholder"',
    "Color::White": '"text.body"',
    "Color::Black": '"text.inverse"',
}

RGB_MAP = {
    "Color::Rgb(30, 30, 35)": '"markdown.code_fence.bg"',
    "Color::Rgb(22, 22, 26)": '"panel.background"',
    "Color::Rgb(50, 50, 56)": '"tool.background"',
    "Color::Rgb(35, 90, 50)": '"surface.success"',
    "Color::Rgb(90, 35, 35)": '"surface.error"',
    "Color::Rgb(90, 60, 35)": '"surface.warning"',
    # Settings constants
    "Color::Rgb(26, 26, 38)": '"sidebar.background"',
    "Color::Rgb(30, 30, 46)": '"card"',
    "Color::Rgb(17, 17, 27)": '"status_bar.background"',
    "Color::Rgb(24, 24, 37)": '"popover"',
}


def migrate_file(path: Path) -> int:
    text = path.read_text()
    original = text

    # Update imports.
    text = text.replace("use crate::theme::ThemeTokens;", "use crate::theme::Theme;")

    # Update parameter types. Two patterns:
    #   tokens: &crate::theme::ThemeTokens
    #   tokens: &ThemeTokens
    text = re.sub(
        r"(\w+):\s*&crate::theme::ThemeTokens",
        lambda m: f"{m.group(1)}: &crate::theme::Theme",
        text,
    )
    text = re.sub(
        r"(\w+):\s*&ThemeTokens",
        lambda m: f"{m.group(1)}: &Theme",
        text,
    )

    # Rename parameter references from `tokens.` to `theme.tokens.`.
    # We assume the parameter was named `tokens` in all these files.
    # Use a regex that only matches `tokens.` not preceded by another identifier or `theme.`.
    def rename_tokens(match: re.Match) -> str:
        prefix = match.group(1)
        if prefix == "theme":
            return match.group(0)
        return f"{prefix}theme.tokens."

    text = re.sub(r"(?<![A-Za-z0-9_])tokens\.", "theme.tokens.", text)

    # Replace direct Color::X usages on a per-line basis using the renamed theme var.
    lines = text.splitlines()
    new_lines = []
    for line in lines:
        new_line = line
        # Determine the theme variable name used on this line.
        if "theme.tokens." in new_line or "theme.resolve(" in new_line:
            var = "theme"
        else:
            var = None

        if var:
            for color, token in COLOR_MAP.items():
                new_line = new_line.replace(color, f"{var}.resolve({token})")
            for rgb, token in RGB_MAP.items():
                new_line = new_line.replace(rgb, f"{var}.resolve({token})")
        new_lines.append(new_line)
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
