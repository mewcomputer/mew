//! Frame-time benchmark for the chat render path.
//!
//! Simulates wheel-scrolling through transcripts of increasing size and
//! reports per-frame draw times, across several content mixes (plain text,
//! tool calls with output+diff, reasoning blocks, and active selection).
//!
//! Run with:
//!
//! ```sh
//! cargo run -p mew-tui --release --example draw_bench
//! ```

use std::time::Instant;

use mew_tui::harness::Harness;

const MD_BODY: &str = r#"Here's a look at the **render path** and how it scales.

- the drain loop coalesces rapid input events
- markdown is cached per message keyed by width
- tool blocks pad to the full paragraph width

```rust
fn wrapped_height(text: &Text, width: u16) -> u16 {
    text.lines
        .iter()
        .map(|line| (line.width().div_ceil(width as usize)).max(1) as u16)
        .sum()
}
```

The scroll ceiling must be derived from the *wrapped* height, otherwise any
line that wraps makes the bottom unreachable. That is the classic "can't
scroll to the last line" bug, and it is why the paragraph is measured before
rendering rather than after.
"#;

/// Realistic tool output: a few dozen lines of file content with ANSI color.
const TOOL_OUTPUT: &str = "    1\tuse std::collections::HashMap;\n    2\t\n    3\tpub struct App {\n    4\t    pub messages: Vec<Message>,\n    5\t    pub input: String,\n    6\t    pub cursor: usize,\n    7\t    pub scroll: u16,\n    8\t}\n    9\t\n   10\timpl App {\n   11\t    pub fn new() -> Self {\n   12\t        Self { messages: vec![], input: String::new(), cursor: 0, scroll: 0 }\n   13\t    }\n   14\t}\n";

/// A realistic diff: added/removed/context lines, enough to exercise the
/// diff-coloring path in the tool block renderer.
const TOOL_DIFF: &str = "--- a/src/app.rs\n+++ b/src/app.rs\n@@ -1,7 +1,9 @@\n use std::collections::HashMap;\n \n pub struct App {\n     pub messages: Vec<Message>,\n+    pub chat_dirty: Option<u64>,\n+    pub rendered_chat: Option<RenderedChat>,\n     pub input: String,\n-    pub scroll: u16,\n+    pub scroll: u16,\n }\n";

const REASONING: &str = "Let me think about this. The render path rebuilds the entire Text every frame, which is O(total transcript). Even though markdown parsing is cached, the line construction and chat_rows building walk every line. If I cache the built Text and only rebuild when something changes, idle scroll frames become O(visible) instead of O(total). The key insight is that scroll doesn't change the rendered output, only which slice of it is visible. So the cache invalidation should be tied to content mutations, not viewport position.";

/// Build a transcript that mixes assistant text, tool calls (with output +
/// diff), and reasoning blocks — closer to a real agentic session than
/// plain text alone.
fn seed_mixed(h: &mut Harness, n_turns: usize) {
    for i in 0..n_turns {
        // Reasoning block (collapsed by default — header line only).
        h.say_reasoning(&format!("turn {i}: {REASONING}"));
        // Assistant text.
        h.say(&format!("turn {i}\n\n{MD_BODY}"));
        // A tool call with output + diff.
        h.say_tool_call("read", TOOL_OUTPUT, Some(TOOL_DIFF));
        // A bash call with longer output (collapsed by default).
        h.say_tool_call(
            "bash",
            &format!("running tests...\nok. 12 passed.\n{TOOL_OUTPUT}"),
            None,
        );
    }
}

fn bench_scroll_text(n_messages: usize, frames: u32) -> f64 {
    let mut h = Harness::new(200, 50);
    for i in 0..n_messages {
        h.say(&format!("message {i}\n\n{MD_BODY}"));
    }
    h.render();
    h.app.scroll_up(1);
    measure(&mut h, frames, |h| {
        h.app.scroll_up(1);
    })
}

fn bench_scroll_mixed(n_turns: usize, frames: u32) -> f64 {
    let mut h = Harness::new(200, 50);
    seed_mixed(&mut h, n_turns);
    h.render();
    h.app.scroll_up(1);
    measure(&mut h, frames, |h| {
        h.app.scroll_up(1);
    })
}

fn bench_scroll_with_selection(n_turns: usize, frames: u32) -> f64 {
    let mut h = Harness::new(200, 50);
    seed_mixed(&mut h, n_turns);
    h.render();
    h.app.scroll_up(1);
    // Start a selection in the middle of the visible window — selection
    // changes span styling, so this exercises the apply_selection path.
    h.app.sel_anchor_row = Some(5);
    h.app.sel_anchor_col = Some(2);
    h.app.sel_end_row = Some(5);
    h.app.sel_end_col = Some(10);
    h.app.mark_chat_dirty();
    measure(&mut h, frames, |h| {
        h.app.scroll_up(1);
    })
}

fn bench_rebuild_text(n_messages: usize, frames: u32) -> f64 {
    let mut h = Harness::new(200, 50);
    for i in 0..n_messages {
        h.say(&format!("message {i}\n\n{MD_BODY}"));
    }
    h.render();
    measure(&mut h, frames, |h| {
        h.app.mark_chat_dirty();
    })
}

fn bench_rebuild_mixed(n_turns: usize, frames: u32) -> f64 {
    let mut h = Harness::new(200, 50);
    seed_mixed(&mut h, n_turns);
    h.render();
    measure(&mut h, frames, |h| {
        h.app.mark_chat_dirty();
    })
}

/// Run `frames` renders, applying `step` before each, return the average
/// frame time in milliseconds.
fn measure(h: &mut Harness, frames: u32, step: impl Fn(&mut Harness)) -> f64 {
    let mut times = Vec::with_capacity(frames as usize);
    for _ in 0..frames {
        step(h);
        let t0 = Instant::now();
        h.render();
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times.iter().sum::<f64>() / times.len() as f64
}

fn main() {
    println!("scroll (idle, cached):");
    println!(
        "  {:>10} {:>14} {:>14} {:>14}",
        "size", "text ms", "mixed ms", "w/ select ms"
    );
    for (n, label) in [(10, "10"), (50, "50"), (200, "200"), (500, "500")] {
        let t = bench_scroll_text(n, 200);
        let m = bench_scroll_mixed(n / 4 + 1, 200);
        let s = bench_scroll_with_selection(n / 4 + 1, 200);
        println!("  {label:>10} {t:>14.3} {m:>14.3} {s:>14.3}");
    }
    println!();
    println!("rebuild (chat_dirty bumped):");
    println!("  {:>10} {:>14} {:>14}", "size", "text ms", "mixed ms");
    for (n, label) in [(10, "10"), (50, "50"), (200, "200"), (500, "500")] {
        let t = bench_rebuild_text(n, 50);
        let m = bench_rebuild_mixed(n / 4 + 1, 50);
        println!("  {label:>10} {t:>14.3} {m:>14.3}");
    }
}
