use ratatui_mdstream::{
    cache::RenderCache,
    highlight::NoHighlight,
    pending::{ElideHeadAndTail, FullPending},
    render::{render_block_with_metadata, render_pending_with_metadata},
    render_markdown, render_markdown_with_metadata, RenderedLines, Theme,
};

fn block(kind: mdstream::BlockKind, text: &str) -> mdstream::Block {
    mdstream::Block {
        id: mdstream::BlockId(1),
        kind,
        status: mdstream::BlockStatus::Committed,
        raw: text.into(),
        display: None,
    }
}

fn render(kind: mdstream::BlockKind, text: &str, width: u16) -> RenderedLines {
    render_block_with_metadata(
        &block(kind, text),
        width,
        &Theme::default(),
        &mut NoHighlight,
    )
}

fn copy(rendered: &RenderedLines) -> String {
    assert_eq!(rendered.lines.len(), rendered.metadata.len());
    let mut text = String::new();
    for (index, (line, metadata)) in rendered.lines.iter().zip(&rendered.metadata).enumerate() {
        let row: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(metadata.content.start <= metadata.content.end);
        assert!(row.is_char_boundary(metadata.content.start));
        assert!(row.is_char_boundary(metadata.content.end));
        if index > 0 && !metadata.continues_previous {
            text.push('\n');
        }
        text.push_str(&row[metadata.content.clone()]);
    }
    text
}

#[test]
fn paragraph_wraps_keep_source_spaces_and_unicode_at_every_width() {
    let source = "alpha  beta界 e\u{301} gamma";
    for width in [1, 3, 7, 12, 80] {
        let rendered = render(mdstream::BlockKind::Paragraph, source, width);
        assert_eq!(copy(&rendered), source, "width {width}");
        assert!(!rendered.metadata[0].continues_previous);
        assert!(rendered.metadata[1..]
            .iter()
            .all(|row| row.continues_previous));
    }
}

#[test]
fn inline_styles_do_not_add_copy_breaks_between_runs() {
    let rendered = render(mdstream::BlockKind::Paragraph, "alpha **bold界** tail", 5);
    assert_eq!(copy(&rendered), "alpha bold界 tail");
}

#[test]
fn literal_newlines_and_empty_rows_are_hard_breaks() {
    let rendered = render(mdstream::BlockKind::Paragraph, "one\n\nthree long", 6);
    assert_eq!(copy(&rendered), "one\n\nthree long");
    assert!(rendered
        .lines
        .iter()
        .all(|line| { line.spans.iter().all(|span| !span.content.contains('\n')) }));
    assert_eq!(
        rendered
            .metadata
            .iter()
            .filter(|row| !row.continues_previous)
            .count(),
        3
    );
}

#[test]
fn lists_keep_first_markers_but_skip_generated_continuation_indentation() {
    let rendered = render(
        mdstream::BlockKind::List,
        "10. alpha beta gamma\n11. delta epsilon",
        12,
    );
    assert_eq!(copy(&rendered), "10. alpha beta gamma\n11. delta epsilon");
    for row in rendered
        .metadata
        .iter()
        .filter(|row| row.continues_previous)
    {
        assert_eq!(row.content.start, 4);
    }
}

#[test]
fn nested_list_items_keep_source_indentation_without_copying_wrap_padding() {
    let rendered = render(
        mdstream::BlockKind::List,
        "- parent words\n  - child long words",
        10,
    );
    assert_eq!(copy(&rendered), "• parent words\n  • child long words");
}

#[test]
fn quotes_keep_logical_line_markers_without_repeating_them_on_soft_wraps() {
    let rendered = render(
        mdstream::BlockKind::BlockQuote,
        "> alpha beta gamma\n> next line",
        9,
    );
    assert_eq!(copy(&rendered), "> alpha beta gamma\n> next line");
    for row in rendered
        .metadata
        .iter()
        .filter(|row| row.continues_previous)
    {
        assert_eq!(row.content.start, 2);
    }
}

#[test]
fn code_copy_excludes_only_generated_right_padding() {
    let rendered = render(
        mdstream::BlockKind::CodeFence,
        "```text\n  界 = 1;  \n\nlast\n```",
        24,
    );
    assert_eq!(copy(&rendered), "  界 = 1;  \n\nlast");
    assert!(rendered.metadata.iter().all(|row| !row.continues_previous));
    assert_eq!(rendered.metadata[0].content, 0.."  界 = 1;  ".len());
    assert_eq!(rendered.metadata[1].content, 0..0);
}

#[test]
fn markdown_block_separators_are_hard_blank_rows_and_legacy_lines_match() {
    let source = "alpha beta gamma\n\ndelta epsilon";
    let rendered = render_markdown_with_metadata(source, 8, &Theme::default());
    assert_eq!(copy(&rendered), source);
    assert_eq!(
        rendered.lines,
        render_markdown(source, 8, &Theme::default())
    );
}

#[test]
fn list_trailing_blank_and_generated_separator_copy_as_one_blank_row() {
    let source = "- a list item that wraps over multiple rows\n- another item\n\n> a quotation that wraps over multiple rows";
    let rendered = render_markdown_with_metadata(source, 16, &Theme::default());
    assert_eq!(copy(&rendered), source.replace("- ", "• "));

    let mut stream = mdstream::MdStream::new(mdstream::Options::default());
    let mut state = mdstream::DocumentState::new();
    state.apply(stream.append(source));
    state.apply(stream.finalize());
    let mut cache = RenderCache::new();
    cache.extend(state.committed(), 16, &Theme::default(), &mut NoHighlight);
    assert_eq!(
        copy(&cache.collect_with_metadata()),
        source.replace("- ", "• ")
    );
}

#[test]
fn full_pending_uses_the_same_copy_bounds_as_committed_rendering() {
    let text = "- alpha beta gamma";
    let pending = mdstream::PendingBlockRef {
        id: mdstream::BlockId(1),
        kind: mdstream::BlockKind::List,
        raw: text,
        display: None,
    };
    let rendered = render_pending_with_metadata(
        &pending,
        10,
        &Theme::default(),
        &mut NoHighlight,
        &FullPending,
    );
    assert_eq!(copy(&rendered), "• alpha beta gamma");
    assert_eq!(
        rendered.metadata,
        render(mdstream::BlockKind::List, text, 10).metadata
    );
}

#[test]
fn elided_pending_retains_code_bounds_and_marks_its_hint_as_a_hard_row() {
    let pending = mdstream::PendingBlockRef {
        id: mdstream::BlockId(1),
        kind: mdstream::BlockKind::CodeFence,
        raw: "```\n  first\n  second\n  third\n  fourth",
        display: None,
    };
    let rendered = render_pending_with_metadata(
        &pending,
        24,
        &Theme::default(),
        &mut NoHighlight,
        &ElideHeadAndTail { max_lines: 3 },
    );
    let copied = copy(&rendered);
    assert!(copied.starts_with("  first\n… generating more …"));
    assert!(copied.ends_with("\n  fourth"));
    assert_eq!(rendered.metadata[2].content, 0.."  fourth".len());
}

#[test]
fn cache_exposes_matching_metadata_after_growth_and_width_changes() {
    let theme = Theme::default();
    let blocks = vec![
        block(mdstream::BlockKind::List, "- alpha beta gamma"),
        block(mdstream::BlockKind::CodeFence, "```\n  last  \n```"),
    ];
    let mut cache = RenderCache::new();
    for width in [10, 80, 4] {
        cache.extend(&blocks[..1], width, &theme, &mut NoHighlight);
        assert_eq!(copy(&cache.collect_with_metadata()), "• alpha beta gamma");
        cache.extend(&blocks, width, &theme, &mut NoHighlight);
        let rendered = cache.collect_with_metadata();
        assert_eq!(copy(&rendered), "• alpha beta gamma\n\n  last  ");
        assert_eq!(rendered.lines, cache.collect_lines());
        assert_eq!(
            cache.committed[0].metadata.len(),
            cache.committed[0].lines.len()
        );
    }
}
