//! Terminal title management — sets the terminal tab/window title to show
//! streaming status so users can tell at a glance when mew is done thinking.

/// Set the terminal title via the xterm escape sequence `\x1b]0;{title}\x07`.
/// This works on macOS Terminal, iTerm2, and most Linux terminals that
/// support the OSC 0 sequence. Terminals that don't support it silently
/// ignore it.
pub fn set_terminal_title(title: &str) {
    // Use stderr so it doesn't interfere with ratatui's stdout rendering.
    eprint!("\x1b]0;{}\x07", title);
}

/// Compute the title string for the current app state.
/// Returns "mew — thinking…" while streaming, "mew" when idle.
pub fn title_for_streaming(streaming: bool) -> &'static str {
    if streaming {
        "mew — thinking…"
    } else {
        "mew"
    }
}