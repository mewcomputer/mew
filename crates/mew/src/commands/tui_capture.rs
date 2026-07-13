//! `mew tui-capture` — deterministic TUI screenshots and video.
//!
//! Two modes:
//! - **Script mode** (`--script <file>`): reads a harness script and runs it
//!   to completion. Prints text snapshots to stdout; screenshots/videos write
//!   to files.
//! - **Interactive mode** (`--interactive`): reads verbs from stdin one line
//!   at a time against a persistent harness. After each verb, prints a
//!   `--- frame ---` text dump of the current state AND optionally writes a
//!   PNG screenshot to `--screenshot-dir`. When `--mp4` is given, all frames
//!   are recorded and encoded to video on exit.
//!
//! This enables agent-driven puppet mode — the agent sends an action, sees
//! the text frame + screenshot, decides the next action.

use anyhow::{Context, Result};
use std::io::{self, BufRead, Write};
use std::path::Path;

/// Run the tui-capture command.
pub(crate) fn run(
    script: Option<&Path>,
    interactive: bool,
    screenshot_dir: Option<&Path>,
    mp4: Option<&str>,
    fps: u32,
    width: u16,
    height: u16,
) -> Result<()> {
    if interactive {
        run_interactive(width, height, screenshot_dir, mp4, fps)
    } else {
        let script_path = script.context("either --script or --interactive is required")?;
        run_script_file(script_path, mp4, fps, width, height)
    }
}

/// Script mode: read a file and run it all at once.
fn run_script_file(
    script_path: &Path,
    mp4: Option<&str>,
    fps: u32,
    width: u16,
    height: u16,
) -> Result<()> {
    let script = std::fs::read_to_string(script_path)
        .with_context(|| format!("failed to read script file: {}", script_path.display()))?;

    // If --mp4 is given and the script doesn't already handle recording,
    // wrap it automatically.
    let script = if let Some(mp4_path) = mp4 {
        if script.contains("start_recording") {
            script
        } else {
            format!("start_recording\n{script}\nstop_recording\nrecord \"{mp4_path}\" {fps}")
        }
    } else {
        script
    };

    let output = mew_tui::harness::run_script(&script, width, height);
    print!("{output}");
    Ok(())
}

/// Interactive REPL mode: read verbs from stdin, print frames to stdout.
///
/// Protocol:
/// - Each line of stdin is a verb (same format as script files).
/// - After each verb, stdout gets:
///   1. Any verb-specific output (snapshot text, error messages)
///   2. A `--- frame ---` block with the current text rendering
///   3. If `--screenshot-dir` is set, a `--- screenshot: <path> ---` line
///      with the path to the PNG of this frame
/// - `screenshot <path>` and `record <path>` verbs still work as usual.
/// - `quit` or EOF exits the loop. If `--mp4` is set, encodes video on exit.
fn run_interactive(
    width: u16,
    height: u16,
    screenshot_dir: Option<&Path>,
    mp4: Option<&str>,
    fps: u32,
) -> Result<()> {
    let mut harness = mew_tui::harness::Harness::new(width, height);

    // Start recording if --mp4 is set
    if mp4.is_some() {
        harness.start_recording();
    }

    // Create screenshot dir if needed
    if let Some(dir) = screenshot_dir {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create screenshot dir: {}", dir.display()))?;
    }

    let mut frame_num: u32 = 0;

    // Print initial frame
    print_frame(&mut harness, screenshot_dir, &mut frame_num);

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    for line in lines.by_ref() {
        let line = line.context("failed to read stdin")?;
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Check for quit before executing
        if matches!(trimmed, "quit" | "exit") {
            break;
        }

        // Execute the verb and print any output it produces
        let result = harness.exec_verb(trimmed);
        if !result.is_empty() {
            print!("{result}");
            io::stdout().flush().ok();
        }

        // Print the current frame + optional screenshot
        print_frame(&mut harness, screenshot_dir, &mut frame_num);
    }

    // Stop recording and encode video if --mp4 was set
    if let Some(mp4_path) = mp4 {
        harness.stop_recording();
        match harness.encode_mp4(mp4_path, fps) {
            Ok(_) => eprintln!("--- video saved to {mp4_path} ---"),
            Err(e) => eprintln!("!! video encoding failed: {e}"),
        }
    }

    println!("--- bye ---");
    Ok(())
}

/// Print the current harness state as a text frame.
/// If screenshot_dir is set, also writes a PNG and prints its path.
fn print_frame(
    harness: &mut mew_tui::harness::Harness,
    screenshot_dir: Option<&Path>,
    frame_num: &mut u32,
) {
    println!("--- frame ---");
    print!("{}", harness.render());
    println!("---");

    if let Some(dir) = screenshot_dir {
        *frame_num += 1;
        let filename = format!("frame_{:04}.png", *frame_num);
        let png_path = dir.join(&filename);
        match harness.screenshot(png_path.to_str().unwrap()) {
            Ok(()) => {
                println!("--- screenshot: {} ---", png_path.display());
            }
            Err(e) => {
                eprintln!("!! screenshot failed: {e}");
            }
        }
    }

    io::stdout().flush().ok();
}
