use std::ops::Range;

use ratatui::style::{Color, Modifier};
use ratatui_mdstream::highlight::{Highlighter, SyntectHighlighter};

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
    List(Vec<InlineText>),
    Quote(Vec<InlineText>),
    Code {
        language: Option<String>,
        text: String,
    },
    Table(Vec<InlineText>),
    Rule,
    Raw(InlineText),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineText {
    pub text: String,
    pub highlights: Vec<InlineHighlight>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineHighlight {
    pub range: Range<usize>,
    pub style: InlineStyle,
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

pub fn highlight_code_blocks(blocks: &mut [MarkdownRenderBlock]) {
    let mut highlighter = SyntectHighlighter::default();
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
        MarkdownBlock::Table(rows) => rows
            .chunks(VIRTUAL_ROW_ITEMS)
            .enumerate()
            .map(|(index, rows)| MarkdownRenderBlock {
                block: MarkdownBlock::Table(rows.to_vec()),
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
    }
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    split_inline(&InlineText {
        text: text.to_owned(),
        highlights: Vec::new(),
    })
    .into_iter()
    .map(|chunk| chunk.text)
    .collect()
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
            let content = trimmed[level..].trim_start();
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
        mdstream::BlockKind::Table => MarkdownBlock::Table(
            text.lines()
                .filter(|line| !is_table_separator(line))
                .map(|line| parse_inline(line.trim()))
                .collect(),
        ),
        mdstream::BlockKind::ThematicBreak => MarkdownBlock::Rule,
        mdstream::BlockKind::FootnoteDefinition
        | mdstream::BlockKind::HtmlBlock
        | mdstream::BlockKind::MathBlock
        | mdstream::BlockKind::Unknown => MarkdownBlock::Raw(parse_inline(text.trim())),
    }
}

fn list_item_text(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if let Some(item) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        return Some(item);
    }

    let (number, item) = trimmed.split_once(". ")?;
    number
        .chars()
        .all(|character| character.is_ascii_digit())
        .then_some(item)
}

fn parse_list_items(text: &str) -> Vec<InlineText> {
    let mut items = Vec::new();
    for line in text.lines() {
        if let Some(item) = list_item_text(line) {
            items.push(item.to_owned());
        } else if let Some(item) = items.last_mut() {
            let continuation = line.trim();
            if !continuation.is_empty() {
                if item
                    .chars()
                    .last()
                    .is_some_and(|character| !character.is_whitespace())
                {
                    item.push(' ');
                }
                item.push_str(continuation);
            }
        }
    }
    items.into_iter().map(|item| parse_inline(&item)).collect()
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

fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim().trim_matches('|').trim();
    !trimmed.is_empty()
        && trimmed.split('|').all(|cell| {
            cell.trim()
                .chars()
                .all(|character| character == '-' || character == ':')
        })
}

fn parse_inline(input: &str) -> InlineText {
    let mut output = InlineText {
        text: String::new(),
        highlights: Vec::new(),
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
                    index += close_bracket + 2 + close_paren + 1;
                    continue;
                }
            }
        }

        let marker = if rest.starts_with("**") || rest.starts_with("__") {
            Some((2, &rest[..2], InlineStyle::Strong))
        } else if rest.starts_with("~~") {
            Some((2, &rest[..2], InlineStyle::Strikethrough))
        } else if rest.starts_with('`') {
            Some((1, "`", InlineStyle::Code))
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
    }

    #[test]
    fn syntax_highlights_code_fences_with_existing_syntect_assets() {
        let document = parse_document("```rust\nfn main() { let answer = 42; }\n```");
        let mut rows = virtualize_document(&document);
        highlight_code_blocks(&mut rows);

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
            MarkdownBlock::Table(rows) if rows.len() == 2
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
        assert_eq!(items[0].text, item);

        let rows = virtualize_document(&document);
        assert_eq!(rows.len(), 1);
        assert!(matches!(
            &rows[0].block,
            MarkdownBlock::List(items) if items.len() == 1 && items[0].text == item
        ));
    }

    #[test]
    fn preserves_hard_wrapped_list_item_continuations() {
        let document = parse_document("- first line\n  continuation line\n\n- second item");

        let MarkdownBlock::List(items) = &document.blocks[0] else {
            panic!("expected a list")
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "first line continuation line");
        assert_eq!(items[1].text, "second item");
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
