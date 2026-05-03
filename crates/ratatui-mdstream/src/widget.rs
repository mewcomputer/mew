use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, StatefulWidget, Widget, Wrap},
};

use crate::{
    cache::RenderCache,
    highlight::NoHighlight,
    pending::ElideHeadAndTail,
    render::render_pending,
    theme::Theme,
};

/// State for `StreamView`, holding the render cache and scroll position.
pub struct StreamViewState {
    pub cache: RenderCache,
    pub scroll: u16,
    pub follow_tail: bool,
    pub theme: Theme,
    highlighter: Box<dyn crate::highlight::Highlighter>,
    pending_policy: Box<dyn crate::pending::PendingPolicy>,
}

impl StreamViewState {
    pub fn new() -> Self {
        Self {
            cache: RenderCache::new(),
            scroll: 0,
            follow_tail: true,
            theme: Theme::default(),
            highlighter: Box::new(NoHighlight),
            pending_policy: Box::new(ElideHeadAndTail::default()),
        }
    }

    pub fn with_highlighter<H: crate::highlight::Highlighter + 'static>(mut self, h: H) -> Self {
        self.highlighter = Box::new(h);
        self
    }

    pub fn scroll_up(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
        self.follow_tail = false;
    }

    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_add(n);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll = 0;
        self.follow_tail = false;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.follow_tail = true;
    }

    pub fn toggle_follow_tail(&mut self) {
        self.follow_tail = !self.follow_tail;
    }

    pub fn set_follow_tail(&mut self, on: bool) {
        self.follow_tail = on;
    }

    /// Notify the state that an update was applied to the document.
    pub fn notify_applied(&mut self, applied: &mdstream::AppliedUpdate) {
        if applied.reset {
            self.cache.invalidate();
        }
    }
}

impl Default for StreamViewState {
    fn default() -> Self {
        Self::new()
    }
}

/// A ratatui widget that renders an `mdstream::DocumentState`.
pub struct StreamView<'a> {
    state: &'a mdstream::DocumentState,
    theme: Option<&'a Theme>,
    follow_tail: bool,
}

impl<'a> StreamView<'a> {
    pub fn new(state: &'a mdstream::DocumentState) -> Self {
        Self {
            state,
            theme: None,
            follow_tail: false,
        }
    }

    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn follow_tail(mut self, follow: bool) -> Self {
        self.follow_tail = follow;
        self
    }
}

impl StatefulWidget for StreamView<'_> {
    type State = StreamViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let theme = self.theme.unwrap_or(&state.theme);
        let width = area.width;

        // Update cache with any new committed blocks.
        state.cache.extend(
            self.state.committed(),
            width,
            theme,
            &mut *state.highlighter,
        );

        // Collect committed lines.
        let mut all_lines = state.cache.collect_lines();

        // Append pending block if present.
        if let Some(pending) = self.state.pending() {
            let pending_ref = mdstream::PendingBlockRef {
                id: pending.id,
                kind: pending.kind,
                raw: &pending.raw,
                display: pending.display.as_deref(),
            };
            let pending_lines = render_pending(
                &pending_ref,
                width,
                theme,
                &mut *state.highlighter,
                &*state.pending_policy,
            );
            if !all_lines.is_empty() && !pending_lines.is_empty() {
                all_lines.push(Line::from(""));
            }
            all_lines.extend(pending_lines);
        }

        // Compute scroll.
        let content_height = all_lines.len() as u16;
        let max_scroll = content_height.saturating_sub(area.height);

        if self.follow_tail || state.follow_tail {
            state.scroll = max_scroll;
        } else {
            state.scroll = state.scroll.min(max_scroll);
        }

        let paragraph = Paragraph::new(all_lines)
            .wrap(Wrap { trim: false })
            .scroll((state.scroll, 0));

        paragraph.render(area, buf);
    }
}
