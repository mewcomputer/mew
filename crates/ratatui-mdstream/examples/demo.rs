use std::io;

use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use mdstream::{MdStream, Options};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders},
    Terminal,
};
use ratatui_mdstream::{StreamView, StreamViewState, Theme};

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut stream = MdStream::new(Options::default());
    let mut doc_state = mdstream::DocumentState::new();
    let mut view_state = StreamViewState::new()
        .with_highlighter(ratatui_mdstream::highlight::SyntectHighlighter::default());

    // Feed some sample markdown in chunks to simulate streaming.
    let chunks = vec![
        "# Hello, mdstream!\n\nThis is a **streaming** markdown ",
        "example for *ratatui*.\n\n## Features\n\n",
        "- Inline parsing: **bold**, *italic*, `code`\n",
        "- Word-aware wrapping\n",
        "- Styled code fences\n\n",
        "```rust\nfn main() {\n    println!(\"hello\");\n}\n```\n\n",
        "### WGSL Shader\n\n",
        "```wgsl\n@vertex\nfn vs_main(@location(0) pos: vec2<f32>) -> @builtin(position) vec4<f32> {\n    return vec4<f32>(pos, 0.0, 1.0);\n}\n\n@fragment\nfn fs_main() -> @location(0) vec4<f32> {\n    return vec4<f32>(1.0, 0.5, 0.2, 1.0);\n}\n```\n\n",
        "More text after the code block. ~~strikethrough~~ works too!",
        "### Table Support\n\n| Language | Highlighting | Notes |\n| :--- | :---: | ---: |\n| Rust | ✓ | via syntect |\n| WGSL | ✓ | via two-face |\n| GDScript | ✓ | via two-face |\n| Zig | ✓ | via two-face |\n\n"
    ];

    let mut chunk_iter = chunks.into_iter();

    let res = run(
        &mut terminal,
        &mut stream,
        &mut doc_state,
        &mut view_state,
        &mut chunk_iter,
    );

    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    stream: &mut MdStream,
    doc_state: &mut mdstream::DocumentState,
    view_state: &mut StreamViewState,
    chunks: &mut dyn Iterator<Item = &str>,
) -> io::Result<()>
where
    io::Error: From<<B as ratatui::backend::Backend>::Error>,
{
    loop {
        terminal.draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(area);

            let block = Block::default()
                .title("ratatui-mdstream demo")
                .borders(Borders::ALL);
            let inner = block.inner(chunks[0]);
            f.render_widget(block, chunks[0]);

            let theme = ratatui_mdstream::theme::neutral();
            let view = StreamView::new(doc_state).theme(&theme).follow_tail(true);
            f.render_stateful_widget(view, inner, view_state);

            let status = ratatui::widgets::Paragraph::new(
                "q: quit | j/k: scroll | g/G: top/bottom | f: toggle follow",
            );
            f.render_widget(status, chunks[1]);
        })?;

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('j') => view_state.scroll_down(1),
                    KeyCode::Char('k') => view_state.scroll_up(1),
                    KeyCode::Char('g') => view_state.scroll_to_top(),
                    KeyCode::Char('G') => view_state.scroll_to_bottom(),
                    KeyCode::Char('f') => view_state.toggle_follow_tail(),
                    _ => {}
                }
            }
        }

        // Feed next chunk if available.
        if let Some(chunk) = chunks.next() {
            // split on spaces
            for chr in chunk.chars() {
                let update = stream.append(&format!("{}", chr));
                let applied = doc_state.apply(update);
                view_state.notify_applied(&applied);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    Ok(())
}
