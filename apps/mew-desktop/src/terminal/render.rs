use gpui::*;

use super::data::RowData;
use super::metrics::CellMetrics;

pub fn build_row_elements(
    rows: &[RowData],
    metrics: &CellMetrics,
    default_bg: Hsla,
) -> Vec<AnyElement> {
    let row_h = px(metrics.height);
    let cell_w = metrics.width;

    rows.iter()
        .map(|row| {
            let mut styled = StyledText::new(SharedString::from(row.text.clone()));
            if !row.highlights.is_empty() {
                styled = styled.with_highlights(row.highlights.clone());
            }

            let mut row_div = div()
                .relative()
                .h(row_h)
                .w_full()
                .overflow_hidden()
                .bg(default_bg);

            for bg_run in &row.bg_runs {
                let x = px(bg_run.start_col as f32 * cell_w);
                let w = px((bg_run.end_col - bg_run.start_col) as f32 * cell_w);
                row_div = row_div.child(
                    div()
                        .absolute()
                        .left(x)
                        .top(px(0.0))
                        .w(w)
                        .h(row_h)
                        .bg(bg_run.color),
                );
            }

            row_div.child(styled).into_any_element()
        })
        .collect()
}
