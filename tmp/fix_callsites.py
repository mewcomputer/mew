#!/usr/bin/env python3
"""Update call sites to pass &app.theme instead of &app.theme.tokens."""

from pathlib import Path

FILES = [
    Path("crates/mew-tui/src/ui/mod.rs"),
    Path("crates/mew-tui/src/ui/overlays.rs"),
    Path("crates/mew-tui/src/ui/status.rs"),
]

for path in FILES:
    text = path.read_text()
    text = text.replace("&app.theme.tokens", "&app.theme")
    path.write_text(text)
    print(f"updated {path}")
