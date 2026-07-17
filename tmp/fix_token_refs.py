#!/usr/bin/env python3
"""Switch generated references from `theme.tokens`/theme.resolve to tokens.tokens/tokens.resolve
because the parameter names stayed as `tokens`."""

from pathlib import Path

FILES = [
    Path("crates/mew-tui/src/ui/overlays.rs"),
    Path("crates/mew-tui/src/ui/status.rs"),
    Path("crates/mew-tui/src/settings.rs"),
]

for path in FILES:
    text = path.read_text()
    text = text.replace("theme.tokens.", "tokens.tokens.")
    text = text.replace("theme.resolve(", "tokens.resolve(")
    path.write_text(text)
    print(f"updated {path}")
