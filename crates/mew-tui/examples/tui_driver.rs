//! Headless TUI driver. Runs a script against the mew TUI and prints the
//! rendered frames as text — for agents and humans to exercise the interface
//! without a real terminal.
//!
//! Usage:
//!   cargo run -p mew-tui --example tui_driver -- <script-file>
//!   cargo run -p mew-tui --example tui_driver           # reads a script from stdin
//!
//! See `crates/mew-tui/examples/demo.tuiscript` for a sample, and the
//! `mew_tui::harness` module docs for the verb list.

use std::io::Read;

fn main() {
    let script = match std::env::args().nth(1) {
        Some(path) => std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("error: read {path}: {e}");
            std::process::exit(1);
        }),
        None => {
            let mut s = String::new();
            if std::io::stdin().read_to_string(&mut s).is_err() {
                eprintln!("error: failed to read script from stdin");
                std::process::exit(1);
            }
            s
        }
    };

    print!("{}", mew_tui::harness::run_script(&script, 80, 24));
}
