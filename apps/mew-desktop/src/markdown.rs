use std::ops::Range;

use ratatui::style::{Color, Modifier};
use ratatui_mdstream::highlight::Highlighter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownDocument {
    pub blocks: Vec<MarkdownBlock>,
}

/// Parser state retained for an assistant part while it is streaming. The
/// parser can then consume only the new delta instead of reparsing the entire
/// response on every event.
#[derive(Debug)]
pub struct StreamingMarkdown {
    stream: mdstream::MdStream,
    state: mdstream::DocumentState,
}

impl StreamingMarkdown {
    pub fn new() -> Self {
        Self {
            stream: mdstream::MdStream::new(mdstream::Options::default()),
            state: mdstream::DocumentState::new(),
        }
    }

    pub fn append(&mut self, delta: &str) -> MarkdownDocument {
        self.state.apply(self.stream.append(delta));
        MarkdownDocument {
            blocks: self.state.blocks().map(parse_block).collect(),
        }
    }
}

const VIRTUAL_ROW_BYTES: usize = 1024;
const VIRTUAL_ROW_ITEMS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownRenderBlock {
    pub block: MarkdownBlock,
    pub continuation: bool,
    pub syntax_highlights: Vec<MarkdownSyntaxHighlight>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownSyntaxHighlight {
    pub range: Range<usize>,
    pub color: Option<[u8; 3]>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkdownBlock {
    Paragraph(InlineText),
    Heading {
        level: u8,
        content: InlineText,
    },
    List(Vec<MarkdownListItem>),
    Quote(Vec<InlineText>),
    Code {
        language: Option<String>,
        text: String,
    },
    Table(MarkdownTable),
    Rule,
    Raw(InlineText),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownListItem {
    pub marker: String,
    pub depth: usize,
    pub content: InlineText,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownTable {
    pub alignments: Vec<MarkdownTableAlignment>,
    pub rows: Vec<Vec<InlineText>>,
    pub row_start: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownTableAlignment {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineText {
    pub text: String,
    pub highlights: Vec<InlineHighlight>,
    pub links: Vec<InlineLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineHighlight {
    pub range: Range<usize>,
    pub style: InlineStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineLink {
    pub range: Range<usize>,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineStyle {
    Emphasis,
    Strong,
    Strikethrough,
    Code,
    Link,
}

pub fn virtualize_document(document: &MarkdownDocument) -> Vec<MarkdownRenderBlock> {
    document.blocks.iter().flat_map(virtualize_block).collect()
}

pub fn highlight_code_blocks(
    blocks: &mut [MarkdownRenderBlock],
    highlighter: &mut impl Highlighter,
) {
    let mut in_code_block = false;

    for block in blocks {
        let MarkdownBlock::Code { language, text } = &block.block else {
            if in_code_block {
                highlighter.end_block();
                in_code_block = false;
            }
            continue;
        };

        if !block.continuation || !in_code_block {
            highlighter.begin_block(language.as_deref());
            in_code_block = true;
        }
        let mut offset = 0;
        block.syntax_highlights = highlighter
            .highlight_line(language.as_deref(), text)
            .into_iter()
            .filter_map(|(text, style)| {
                let start = offset;
                offset += text.len();
                let color = match style.fg {
                    Some(Color::Rgb(red, green, blue)) => Some([red, green, blue]),
                    _ => None,
                };
                let modifiers = style.add_modifier;
                (start < offset).then_some(MarkdownSyntaxHighlight {
                    range: start..offset,
                    color,
                    bold: modifiers.contains(Modifier::BOLD),
                    italic: modifiers.contains(Modifier::ITALIC),
                    underline: modifiers.contains(Modifier::UNDERLINED),
                })
            })
            .collect();
    }

    if in_code_block {
        highlighter.end_block();
    }
}

fn virtualize_block(block: &MarkdownBlock) -> Vec<MarkdownRenderBlock> {
    match block {
        MarkdownBlock::Paragraph(inline) | MarkdownBlock::Raw(inline) => split_inline(inline)
            .into_iter()
            .enumerate()
            .map(|(index, inline)| MarkdownRenderBlock {
                block: if matches!(block, MarkdownBlock::Paragraph(_)) {
                    MarkdownBlock::Paragraph(inline)
                } else {
                    MarkdownBlock::Raw(inline)
                },
                continuation: index > 0,
                syntax_highlights: Vec::new(),
            })
            .collect(),
        MarkdownBlock::Code { language, text } => split_lines(text)
            .into_iter()
            .enumerate()
            .map(|(index, text)| MarkdownRenderBlock {
                block: MarkdownBlock::Code {
                    language: language.clone(),
                    text,
                },
                continuation: index > 0,
                syntax_highlights: Vec::new(),
            })
            .collect(),
        MarkdownBlock::List(items) => items
            .chunks(VIRTUAL_ROW_ITEMS)
            .enumerate()
            .map(|(index, items)| MarkdownRenderBlock {
                block: MarkdownBlock::List(items.to_vec()),
                continuation: index > 0,
                syntax_highlights: Vec::new(),
            })
            .collect(),
        MarkdownBlock::Quote(lines) => lines
            .chunks(VIRTUAL_ROW_ITEMS)
            .enumerate()
            .map(|(index, lines)| MarkdownRenderBlock {
                block: MarkdownBlock::Quote(lines.to_vec()),
                continuation: index > 0,
                syntax_highlights: Vec::new(),
            })
            .collect(),
        MarkdownBlock::Table(table) => table
            .rows
            .chunks(VIRTUAL_ROW_ITEMS)
            .enumerate()
            .map(|(index, rows)| MarkdownRenderBlock {
                block: MarkdownBlock::Table(MarkdownTable {
                    alignments: table.alignments.clone(),
                    rows: rows.to_vec(),
                    row_start: table.row_start + index * VIRTUAL_ROW_ITEMS,
                }),
                continuation: index > 0,
                syntax_highlights: Vec::new(),
            })
            .collect(),
        block => vec![MarkdownRenderBlock {
            block: block.clone(),
            continuation: false,
            syntax_highlights: Vec::new(),
        }],
    }
}

fn split_inline(inline: &InlineText) -> Vec<InlineText> {
    if inline.text.len() <= VIRTUAL_ROW_BYTES {
        return vec![inline.clone()];
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    while start < inline.text.len() {
        let remaining = inline.text.len() - start;
        let mut end = inline.text.len().min(start + VIRTUAL_ROW_BYTES);
        while end < inline.text.len() && !inline.text.is_char_boundary(end) {
            end += 1;
        }
        if end < inline.text.len() {
            if let Some(whitespace) = inline.text[start..end].rfind(char::is_whitespace) {
                let candidate = start
                    + whitespace
                    + inline.text[start + whitespace..]
                        .chars()
                        .next()
                        .map_or(0, char::len_utf8);
                if candidate > start {
                    end = candidate;
                }
            }
        }
        if end <= start {
            end = start + remaining.min(VIRTUAL_ROW_BYTES);
        }
        chunks.push(slice_inline(inline, start..end));
        start = end;
    }
    chunks
}

fn slice_inline(inline: &InlineText, range: std::ops::Range<usize>) -> InlineText {
    InlineText {
        text: inline.text[range.clone()].to_owned(),
        highlights: inline
            .highlights
            .iter()
            .filter_map(|highlight| {
                let start = highlight.range.start.max(range.start);
                let end = highlight.range.end.min(range.end);
                (start < end).then(|| InlineHighlight {
                    range: start - range.start..end - range.start,
                    style: highlight.style,
                })
            })
            .collect(),
        links: inline
            .links
            .iter()
            .filter_map(|link| {
                let start = link.range.start.max(range.start);
                let end = link.range.end.min(range.end);
                (start < end).then(|| InlineLink {
                    range: start - range.start..end - range.start,
                    url: link.url.clone(),
                })
            })
            .collect(),
    }
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut end = 0;
    let mut lines = 0;
    for line in text.split_inclusive('\n') {
        if end > start
            && (end - start + line.len() > VIRTUAL_ROW_BYTES || lines == VIRTUAL_ROW_ITEMS)
        {
            chunks.push(text[start..end].to_owned());
            start = end;
            lines = 0;
        }
        end += line.len();
        lines += 1;
    }
    chunks.push(text[start..end].to_owned());
    chunks
}

#[cfg(test)]
mod code_chunk_tests {
    use super::*;

    #[test]
    fn code_chunks_end_at_physical_newlines_and_preserve_the_source() {
        let source = (0..65)
            .map(|index| format!("let line_{index} = \"🙂中 {}\";", "x".repeat(index % 13)))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = split_lines(&source);

        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), source);
        assert!(chunks[..chunks.len() - 1]
            .iter()
            .all(|chunk| chunk.ends_with('\n')));
        assert!(chunks.iter().all(|chunk| chunk.len() <= VIRTUAL_ROW_BYTES));
    }

    #[test]
    fn code_chunks_keep_an_oversized_physical_line_on_one_visual_row() {
        let line = format!("let value = \"{}\";", "🙂中".repeat(400));
        let source = format!("before\n{line}\nafter\n");
        let chunks = split_lines(&source);

        assert_eq!(chunks, ["before\n", &format!("{line}\n"), "after\n"]);
        assert_eq!(chunks.concat(), source);
        assert_eq!(split_lines(""), [""]);
    }

    #[test]
    fn code_chunks_bound_runs_of_empty_lines() {
        let source = "\n".repeat(100);
        let chunks = split_lines(&source);
        assert_eq!(chunks.concat(), source);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.lines().count() <= VIRTUAL_ROW_ITEMS));
        assert!(chunks.len() > 1);
    }
}

pub fn parse_document(text: &str) -> MarkdownDocument {
    let mut stream = mdstream::MdStream::new(mdstream::Options::default());
    let mut state = mdstream::DocumentState::new();
    state.apply(stream.append(text));
    state.apply(stream.finalize());

    MarkdownDocument {
        blocks: state.blocks().map(parse_block).collect(),
    }
}

fn parse_block(block: &mdstream::Block) -> MarkdownBlock {
    let text = block.display_or_raw();
    match block.kind {
        mdstream::BlockKind::Heading => {
            let trimmed = text.trim_start();
            let level = trimmed
                .chars()
                .take_while(|character| *character == '#')
                .count();
            let content = trimmed[level..].trim();
            MarkdownBlock::Heading {
                level: level.clamp(1, 6) as u8,
                content: parse_inline(content),
            }
        }
        mdstream::BlockKind::Paragraph => MarkdownBlock::Paragraph(parse_inline(text.trim())),
        mdstream::BlockKind::List => MarkdownBlock::List(parse_list_items(text)),
        mdstream::BlockKind::BlockQuote => MarkdownBlock::Quote(
            text.lines()
                .map(|line| line.trim_start().trim_start_matches('>').trim_start())
                .map(parse_inline)
                .collect(),
        ),
        mdstream::BlockKind::CodeFence => {
            let mut lines = text.lines();
            let language = lines.next().and_then(code_language).map(str::to_owned);
            let mut code_lines: Vec<&str> = lines.collect();
            if code_lines
                .last()
                .is_some_and(|line| is_fence(line.trim_start()))
            {
                code_lines.pop();
            }
            MarkdownBlock::Code {
                language,
                text: code_lines.join("\n"),
            }
        }
        mdstream::BlockKind::Table => MarkdownBlock::Table(parse_table(&block.raw)),
        mdstream::BlockKind::ThematicBreak => MarkdownBlock::Rule,
        mdstream::BlockKind::FootnoteDefinition
        | mdstream::BlockKind::HtmlBlock
        | mdstream::BlockKind::MathBlock
        | mdstream::BlockKind::Unknown => MarkdownBlock::Raw(parse_inline(text.trim())),
    }
}

#[derive(Clone, Copy)]
enum ListMarker {
    Bullet,
    Ordered { number: u64, delimiter: char },
}

struct ListItemSource<'a> {
    indentation: usize,
    content_indentation: usize,
    marker: ListMarker,
    content: &'a str,
}

fn list_item_source(line: &str) -> Option<ListItemSource<'_>> {
    let trimmed = line.trim_start_matches([' ', '\t']);
    let indent_bytes = line.len() - trimmed.len();
    let bytes = trimmed.as_bytes();
    let (marker, marker_len) = match bytes.first()? {
        b'-' | b'*' | b'+' => (ListMarker::Bullet, 1),
        b'0'..=b'9' => {
            let digits = bytes
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            let delimiter = match bytes.get(digits)? {
                b'.' => '.',
                b')' => ')',
                _ => return None,
            };
            let number = trimmed[..digits].parse().ok()?;
            (ListMarker::Ordered { number, delimiter }, digits + 1)
        }
        _ => return None,
    };
    if !matches!(bytes.get(marker_len), Some(b' ' | b'\t')) {
        return None;
    }
    let content = trimmed[marker_len..].trim_start_matches([' ', '\t']);
    let content_start = line.len() - content.len();
    Some(ListItemSource {
        indentation: prefix_columns(&line[..indent_bytes]),
        content_indentation: prefix_columns(&line[..content_start]),
        marker,
        content,
    })
}

fn prefix_columns(prefix: &str) -> usize {
    prefix.chars().fold(0, |column, character| {
        if character == '\t' {
            column + 4 - column % 4
        } else {
            column + 1
        }
    })
}

fn parse_list_items(text: &str) -> Vec<MarkdownListItem> {
    struct ListLevel {
        indentation: usize,
        content_indentation: usize,
        next_number: Option<(u64, char)>,
        item_index: usize,
    }

    let mut levels: Vec<ListLevel> = Vec::new();
    let mut items: Vec<MarkdownListItem> = Vec::new();
    for line in text.lines() {
        if let Some(source) = list_item_source(line) {
            while levels.len() > 1
                && levels
                    .last()
                    .is_some_and(|level| level.indentation > source.indentation)
            {
                levels.pop();
            }
            if levels
                .last()
                .is_none_or(|level| source.indentation >= level.content_indentation)
            {
                levels.push(ListLevel {
                    indentation: source.indentation,
                    content_indentation: source.content_indentation,
                    next_number: None,
                    item_index: items.len(),
                });
            }
            let depth = levels.len() - 1;
            let level = levels.last_mut().unwrap();
            level.indentation = level.indentation.min(source.indentation);
            let marker = match source.marker {
                ListMarker::Bullet => {
                    level.next_number = None;
                    "•".to_owned()
                }
                ListMarker::Ordered { number, delimiter } => {
                    let number = level
                        .next_number
                        .filter(|(_, previous_delimiter)| *previous_delimiter == delimiter)
                        .map_or(number, |(number, _)| number);
                    level.next_number = Some((number.saturating_add(1), delimiter));
                    format!("{number}{delimiter}")
                }
            };
            level.content_indentation = source.content_indentation;
            level.item_index = items.len();
            items.push(MarkdownListItem {
                marker,
                depth,
                content: InlineText {
                    text: source.content.to_owned(),
                    highlights: Vec::new(),
                    links: Vec::new(),
                },
            });
        } else if !line.trim().is_empty() {
            let trimmed = line.trim_start_matches([' ', '\t']);
            let indentation = prefix_columns(&line[..line.len() - trimmed.len()]);
            let level = levels
                .iter()
                .rev()
                .find(|level| indentation >= level.content_indentation)
                .or_else(|| levels.last());
            if let Some(level) = level {
                let content = &mut items[level.item_index].content.text;
                if !content.is_empty() {
                    content.push(' ');
                }
                content.push_str(line.trim());
            }
        }
    }
    for item in &mut items {
        item.content = parse_inline(&item.content.text);
    }
    items
}

fn code_language(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("```")
        .or_else(|| trimmed.strip_prefix("~~~"))?;
    rest.split_whitespace()
        .next()
        .filter(|language| !language.is_empty())
}

fn is_fence(line: &str) -> bool {
    line.starts_with("```") || line.starts_with("~~~")
}

fn parse_table(text: &str) -> MarkdownTable {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header = table_cells(lines.next().unwrap_or_default());
    let separator = lines.next().map(table_cells).unwrap_or_default();
    let mut alignments = separator
        .iter()
        .map(|cell| match (cell.starts_with(':'), cell.ends_with(':')) {
            (true, true) => MarkdownTableAlignment::Center,
            (_, true) => MarkdownTableAlignment::Right,
            _ => MarkdownTableAlignment::Left,
        })
        .collect::<Vec<_>>();
    let column_count = header.len().max(alignments.len());
    alignments.resize(column_count, MarkdownTableAlignment::Left);
    let rows = std::iter::once(header)
        .chain(lines.map(table_cells))
        .map(|mut cells| {
            cells.resize(column_count, "");
            cells.into_iter().map(parse_inline).collect()
        })
        .collect();
    MarkdownTable {
        alignments,
        rows,
        row_start: 0,
    }
}

fn table_cells(line: &str) -> Vec<&str> {
    let line = line.trim();
    let bytes = line.as_bytes();
    let mut cells = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut last_pipe = None;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                index += 1;
                if let Some(character) = line[index..].chars().next() {
                    index += character.len_utf8();
                }
            }
            b'`' => {
                let delimiter_len = bytes[index..]
                    .iter()
                    .take_while(|byte| **byte == b'`')
                    .count();
                index = code_span_end(line, index, delimiter_len)
                    .map_or(index + delimiter_len, |end| end + delimiter_len);
            }
            b'|' => {
                cells.push(line[start..index].trim());
                last_pipe = Some(index);
                index += 1;
                start = index;
            }
            _ => index += line[index..].chars().next().unwrap().len_utf8(),
        }
    }
    cells.push(line[start..].trim());
    if line.starts_with('|') {
        cells.remove(0);
    }
    if last_pipe.is_some_and(|pipe| pipe + 1 == line.len()) {
        cells.pop();
    }
    cells
}

fn code_span_end(text: &str, start: usize, delimiter_len: usize) -> Option<usize> {
    let mut index = start + delimiter_len;
    while let Some(offset) = text[index..].find('`') {
        index += offset;
        let run = text.as_bytes()[index..]
            .iter()
            .take_while(|byte| **byte == b'`')
            .count();
        if run == delimiter_len {
            return Some(index);
        }
        index += run;
    }
    None
}

fn parse_inline(input: &str) -> InlineText {
    let mut output = InlineText {
        text: String::new(),
        highlights: Vec::new(),
        links: Vec::new(),
    };
    let mut index = 0;

    while index < input.len() {
        let rest = &input[index..];

        if let Some(escaped) = rest.strip_prefix('\\') {
            if let Some(character) = escaped.chars().next() {
                output.text.push(character);
                index += 1 + character.len_utf8();
                continue;
            }
        }

        if rest.starts_with('[') {
            if let Some(close_bracket) = rest.find("](") {
                if let Some(close_paren) = rest[close_bracket + 2..].find(')') {
                    let label = &rest[1..close_bracket];
                    let start = output.text.len();
                    output.text.push_str(label);
                    output.highlights.push(InlineHighlight {
                        range: start..output.text.len(),
                        style: InlineStyle::Link,
                    });
                    let url_start = close_bracket + 2;
                    let url_end = url_start + close_paren;
                    output.links.push(InlineLink {
                        range: start..output.text.len(),
                        url: rest[url_start..url_end].to_owned(),
                    });
                    index += close_bracket + 2 + close_paren + 1;
                    continue;
                }
            }
        }

        if rest.starts_with('`') {
            let delimiter_len = rest.bytes().take_while(|byte| *byte == b'`').count();
            if let Some(end) = code_span_end(rest, 0, delimiter_len) {
                let start = output.text.len();
                output.text.push_str(&rest[delimiter_len..end]);
                output.highlights.push(InlineHighlight {
                    range: start..output.text.len(),
                    style: InlineStyle::Code,
                });
                index += end + delimiter_len;
            } else {
                output.text.push_str(&rest[..delimiter_len]);
                index += delimiter_len;
            }
            continue;
        }

        let marker = if rest.starts_with("**") || rest.starts_with("__") {
            Some((2, &rest[..2], InlineStyle::Strong))
        } else if rest.starts_with("~~") {
            Some((2, &rest[..2], InlineStyle::Strikethrough))
        } else if rest.starts_with('*') || rest.starts_with('_') {
            Some((1, &rest[..1], InlineStyle::Emphasis))
        } else {
            None
        };

        if let Some((marker_len, marker_text, style)) = marker {
            if let Some(end) = rest[marker_len..].find(marker_text) {
                let content = &rest[marker_len..marker_len + end];
                let start = output.text.len();
                output.text.push_str(content);
                output.highlights.push(InlineHighlight {
                    range: start..output.text.len(),
                    style,
                });
                index += marker_len + end + marker_len;
                continue;
            }
        }

        let Some(character) = rest.chars().next() else {
            break;
        };
        output.text.push(character);
        index += character.len_utf8();
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_mdstream::highlight::SyntectHighlighter;

    #[test]
    fn headings_do_not_retain_their_source_line_ending() {
        let document = parse_document("## A readable heading\n\nThe reply continues here.\n");
        let MarkdownBlock::Heading { content, .. } = &document.blocks[0] else {
            panic!("expected heading");
        };
        assert_eq!(content.text, "A readable heading");
    }

    #[test]
    fn parses_blocks_and_inline_styles() {
        let document = parse_document(
            "# Title\n\nA **bold** and *soft* `code` [link](https://example.com).\n\n- one\n- two\n\n> quote",
        );

        assert!(matches!(
            document.blocks[0],
            MarkdownBlock::Heading { level: 1, .. }
        ));
        assert!(matches!(
            document.blocks[2],
            MarkdownBlock::List(ref items) if items.len() == 2
        ));
        assert!(matches!(
            document.blocks[3],
            MarkdownBlock::Quote(ref lines) if lines.len() == 1
        ));

        let MarkdownBlock::Paragraph(inline) = &document.blocks[1] else {
            panic!("expected paragraph")
        };
        assert_eq!(inline.text, "A bold and soft code link.");
        assert_eq!(
            inline
                .highlights
                .iter()
                .map(|highlight| highlight.style)
                .collect::<Vec<_>>(),
            vec![
                InlineStyle::Strong,
                InlineStyle::Emphasis,
                InlineStyle::Code,
                InlineStyle::Link,
            ]
        );
        assert_eq!(inline.links.len(), 1);
        assert_eq!(inline.links[0].url, "https://example.com");
    }

    #[test]
    fn link_targets_survive_virtualized_inline_slices() {
        let document = parse_document("before [a very long link](https://example.com/path) after");
        let MarkdownBlock::Paragraph(inline) = &document.blocks[0] else {
            panic!("expected paragraph")
        };

        let chunks = split_inline(inline);

        assert!(chunks.iter().any(|chunk| {
            chunk
                .links
                .iter()
                .any(|link| link.url == "https://example.com/path")
        }));
    }

    #[test]
    fn syntax_highlights_code_fences_with_existing_syntect_assets() {
        let document = parse_document("```rust\nfn main() { let answer = 42; }\n```");
        let mut rows = virtualize_document(&document);
        highlight_code_blocks(&mut rows, &mut SyntectHighlighter::default());

        assert!(rows.iter().any(|row| {
            matches!(row.block, MarkdownBlock::Code { .. })
                && row
                    .syntax_highlights
                    .iter()
                    .any(|highlight| highlight.color.is_some())
        }));
    }

    #[test]
    fn parses_code_fences_and_tables() {
        let document = parse_document(
            "```rust\nfn main() {}\n```\n\n| name | value |\n| --- | --- |\n| mew | 1 |",
        );

        assert!(matches!(
            &document.blocks[0],
            MarkdownBlock::Code { language: Some(language), text }
                if language == "rust" && text == "fn main() {}"
        ));
        assert!(matches!(
            &document.blocks[1],
            MarkdownBlock::Table(table) if table.rows.len() == 2
        ));
    }

    #[test]
    fn virtualizes_large_blocks_without_dropping_text() {
        let source = "word ".repeat(600);
        let document = parse_document(&source);
        let rows = virtualize_document(&document);
        let rendered = rows
            .iter()
            .filter_map(|row| match &row.block {
                MarkdownBlock::Paragraph(inline) => Some(inline.text.as_str()),
                _ => None,
            })
            .collect::<String>();

        assert!(rows.len() > 1);
        assert_eq!(rendered, source.trim());
        assert!(rows.iter().skip(1).all(|row| row.continuation));
    }

    #[test]
    fn virtualizes_long_unicode_lines_without_splitting_utf8() {
        let source = format!("{}é{}", "a".repeat(VIRTUAL_ROW_BYTES - 1), "a".repeat(8));
        let document = parse_document(&source);
        let rows = virtualize_document(&document);
        let rendered = rows
            .iter()
            .filter_map(|row| match &row.block {
                MarkdownBlock::Paragraph(inline) => Some(inline.text.as_str()),
                _ => None,
            })
            .collect::<String>();

        assert!(rows.len() > 1);
        assert_eq!(rendered, source);
    }

    #[test]
    fn virtualizes_empty_code_blocks_as_one_row() {
        let document = parse_document("```\n```");
        let rows = virtualize_document(&document);

        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0].block, MarkdownBlock::Code { ref text, .. } if text.is_empty()));
    }

    #[test]
    fn preserves_long_list_items_for_layout_wrapping() {
        let item =
            "A long bullet item should stay one logical item while the view wraps it.".repeat(8);
        let document = parse_document(&format!("- {item}"));

        let MarkdownBlock::List(items) = &document.blocks[0] else {
            panic!("expected a list")
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content.text, item);

        let rows = virtualize_document(&document);
        assert_eq!(rows.len(), 1);
        assert!(matches!(
            &rows[0].block,
            MarkdownBlock::List(items) if items.len() == 1 && items[0].content.text == item
        ));
    }

    #[test]
    fn preserves_hard_wrapped_list_item_continuations() {
        let document = parse_document("- first line\n  continuation line\n\n- second item");

        let MarkdownBlock::List(items) = &document.blocks[0] else {
            panic!("expected a list")
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].content.text, "first line continuation line");
        assert_eq!(items[1].content.text, "second item");
    }

    #[test]
    fn preserves_ordered_starts_and_nested_list_markers() {
        let document = parse_document(
            "7. **first** item\n   continuation\n   - child\n     3) [leaf](https://example.com)\n     1) next leaf\n   - sibling\n1. final item\n",
        );
        let MarkdownBlock::List(items) = &document.blocks[0] else {
            panic!("expected a list")
        };

        assert_eq!(
            items
                .iter()
                .map(|item| (item.marker.as_str(), item.depth))
                .collect::<Vec<_>>(),
            [
                ("7.", 0),
                ("•", 1),
                ("3)", 2),
                ("4)", 2),
                ("•", 1),
                ("8.", 0)
            ],
        );
        assert_eq!(items[0].content.text, "first item continuation");
        assert_eq!(items[0].content.highlights[0].style, InlineStyle::Strong);
        assert_eq!(items[2].content.text, "leaf");
        assert_eq!(items[2].content.links[0].url, "https://example.com");
    }

    #[test]
    fn list_continuations_follow_their_parent_across_indentation_changes() {
        let document = parse_document(
            "  4. first\n     - child\n     parent continuation\n1. second\n\t+ tabbed child\n1. final\n",
        );
        let MarkdownBlock::List(items) = &document.blocks[0] else {
            panic!("expected a list")
        };

        assert_eq!(
            items
                .iter()
                .map(|item| (item.marker.as_str(), item.depth))
                .collect::<Vec<_>>(),
            [("4.", 0), ("•", 1), ("5.", 0), ("•", 1), ("6.", 0),]
        );
        assert_eq!(items[0].content.text, "first parent continuation");
        assert_eq!(items[1].content.text, "child");
    }

    #[test]
    fn preserves_table_cells_alignment_and_inline_content() {
        let document = parse_document(
            "| label | code | value |\n| :--- | :---: | ---: |\n| **雪** \\| path | `a|b` | [link](https://example.com) |\n| short |\n| | | | extra |\n",
        );
        let MarkdownBlock::Table(table) = &document.blocks[0] else {
            panic!("expected a table")
        };

        assert_eq!(
            table.alignments,
            [
                MarkdownTableAlignment::Left,
                MarkdownTableAlignment::Center,
                MarkdownTableAlignment::Right,
            ]
        );
        assert_eq!(table.rows.len(), 4);
        assert!(table.rows.iter().all(|row| row.len() == 3));
        assert_eq!(
            table.rows[0]
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<Vec<_>>(),
            ["label", "code", "value"]
        );
        assert_eq!(table.rows[1][0].text, "雪 | path");
        assert_eq!(table.rows[1][0].highlights[0].style, InlineStyle::Strong);
        assert_eq!(table.rows[1][1].text, "a|b");
        assert_eq!(table.rows[1][1].highlights[0].style, InlineStyle::Code);
        assert_eq!(table.rows[1][2].links[0].url, "https://example.com");
        assert_eq!(
            table.rows[2]
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<Vec<_>>(),
            ["short", "", ""]
        );
        assert_eq!(
            table.rows[3]
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<Vec<_>>(),
            ["", "", ""]
        );
        assert_eq!(table.row_start, 0);
    }

    #[test]
    fn table_cell_boundaries_ignore_escaped_pipes_and_matching_code_spans() {
        let document = parse_document(
            "left | right\n:--- | ---:\n``a`|b`` | escaped \\| cell\nunclosed `span | final\n",
        );
        let MarkdownBlock::Table(table) = &document.blocks[0] else {
            panic!("expected a table")
        };

        assert_eq!(table.rows[1][0].text, "a`|b");
        assert_eq!(table.rows[1][1].text, "escaped | cell");
        assert_eq!(table.rows[2][0].text, "unclosed `span");
        assert_eq!(table.rows[2][1].text, "final");
    }

    #[test]
    fn table_and_list_metadata_survive_virtualization_and_streaming() {
        let mut source = String::from("| name | count |\n| :--- | ---: |\n");
        for index in 0..40 {
            source.push_str(&format!("| row {index} | {index} |\n"));
        }
        source.push_str("\n9. first\n   - nested\n");
        for index in 0..20 {
            source.push_str(&format!("1. item {index}\n"));
        }
        source.push('\n');
        let expected = parse_document(&source);
        let mut streaming = StreamingMarkdown::new();
        let mut actual = MarkdownDocument { blocks: Vec::new() };
        for character in source.chars() {
            actual = streaming.append(character.encode_utf8(&mut [0; 4]));
        }
        assert_eq!(actual, expected);

        let rows = virtualize_document(&actual);
        let tables = rows
            .iter()
            .filter_map(|row| match &row.block {
                MarkdownBlock::Table(table) => Some(table),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(tables.len(), 3);
        assert_eq!(
            tables
                .iter()
                .map(|table| table.row_start)
                .collect::<Vec<_>>(),
            [0, 16, 32]
        );
        assert!(tables.iter().all(|table| table.alignments
            == [MarkdownTableAlignment::Left, MarkdownTableAlignment::Right]));
        let list_items = rows
            .iter()
            .filter_map(|row| match &row.block {
                MarkdownBlock::List(items) => Some(items),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(list_items[0].marker, "9.");
        assert_eq!(list_items[1].depth, 1);
        assert_eq!(list_items.last().unwrap().marker, "29.");
    }

    #[test]
    fn streaming_markdown_consumes_only_new_text_and_keeps_the_document() {
        let mut streaming = StreamingMarkdown::new();
        let _ = streaming.append("A **growing");

        let document = streaming.append("** response");
        assert!(matches!(
            document.blocks.as_slice(),
            [MarkdownBlock::Paragraph(inline) | MarkdownBlock::Raw(inline)]
                if inline.text == "A growing response"
        ));
    }
}
