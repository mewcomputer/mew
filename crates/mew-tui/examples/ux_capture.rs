//! Render a source-driven UX gallery without a daemon or provider credentials.
//! `cargo run -p mew-tui --example ux_capture -- notes/capture/tui-current`

use std::path::PathBuf;

use mew_tui::App;
use ratatui::{backend::TestBackend, Terminal};

#[path = "ux_capture/activity.rs"]
mod activity;
#[path = "ux_capture/paste.rs"]
mod paste;
#[path = "ux_capture/pickers.rs"]
mod pickers;
#[path = "ux_capture/question.rs"]
mod question;
#[path = "ux_capture/todos.rs"]
mod todos;
#[path = "ux_capture/trace.rs"]
mod trace;
#[path = "ux_capture/windows.rs"]
mod windows;
#[path = "ux_capture/workflows.rs"]
mod workflows;

struct Gallery {
    directory: PathBuf,
    raster: mew_raster::Rasterizer,
    entries: Vec<(String, u16, u16)>,
}

impl Gallery {
    fn capture(&mut self, app: &mut App, name: &str, width: u16, height: u16) {
        // Hide transient alerts so snapshots document the intended surface.
        app.alert = None;
        app.toasts.clear();
        app.git_branch = Some("main".into());
        app.git_branch_resolved = true;
        app.short_cwd = "~/code/mew".into();
        // Keep elapsed-time labels stable across machines and regeneration runs.
        let now = std::time::Instant::now();
        for agent in &mut app.subagents {
            agent.started_at = now - std::time::Duration::from_secs(127);
        }
        for job in &mut app.background_jobs {
            job.started_at = now - std::time::Duration::from_secs(42);
        }
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let frame = terminal
            .draw(|frame| mew_tui::ui::draw(frame, app))
            .unwrap();
        let png = self.raster.to_png(
            frame.buffer,
            &mew_raster::RasterOptions {
                scale: 1.25,
                ..Default::default()
            },
        );
        std::fs::write(self.directory.join(format!("{name}.png")), png).unwrap();
        let text: String = (0..height)
            .map(|y| {
                let row: String = (0..width).map(|x| frame.buffer[(x, y)].symbol()).collect();
                format!("{}\n", row.trim_end())
            })
            .collect();
        std::fs::write(self.directory.join(format!("{name}.frame")), text).unwrap();
        // A later fixture may intentionally replace an earlier overview.
        self.entries.retain(|(entry, _, _)| entry != name);
        self.entries.push((name.into(), width, height));
    }

    fn finish(self) {
        let mut html = String::from(
            "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>mew TUI current captures</title><style>body{background:#1e1e21;color:#eee;font:16px/1.5 system-ui;margin:0;padding:24px}main{max-width:1280px;margin:auto}h1{font-size:26px}h2{font-size:18px;font-weight:500}p,figcaption{color:#aaa}figure{margin:36px 0}img{display:block;max-width:100%;height:auto}a:focus-visible{outline:2px solid #9ac5fa}figcaption{font-size:13px}</style><main><h1>mew TUI current captures</h1><p>Actual TUI renderer, deterministic sample state. Desktop captures use 128 columns × 40 rows, 1280 × 800 pixels and approximately 16 px text. These show layout and state projection, not live terminal compatibility.</p>",
        );
        for (name, width, height) in &self.entries {
            let title = name
                .trim_start_matches("desktop-")
                .trim_start_matches("short-")
                .replace('-', " ");
            let pixels = (u32::from(*width) * 10, u32::from(*height) * 20);
            html.push_str(&format!(
                "<figure><h2>{title}</h2><a href=\"{name}.png\"><img src=\"{name}.png\" alt=\"{title}\" loading=\"lazy\" width=\"{}\" height=\"{}\"></a><figcaption>{width} columns × {height} rows · {} × {} px · <a href=\"{name}.frame\">text frame</a></figcaption></figure>",
                pixels.0, pixels.1, pixels.0, pixels.1
            ));
        }
        html.push_str("</main></html>\n");
        std::fs::write(self.directory.join("index.html"), html).unwrap();
        println!(
            "{} captures: {}",
            self.entries.len(),
            self.directory.display()
        );
    }
}

fn main() {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("notes/capture/tui-current"));
    std::fs::create_dir_all(&directory).unwrap();
    let mut gallery = Gallery {
        directory,
        raster: mew_raster::Rasterizer::new(),
        entries: vec![],
    };
    windows::run(&mut gallery);
    activity::run(&mut gallery);
    trace::run(&mut gallery);
    todos::run(&mut gallery);
    paste::run(&mut gallery);
    question::run(&mut gallery);
    pickers::run(&mut gallery);
    workflows::run(&mut gallery);
    gallery.finish();
}
