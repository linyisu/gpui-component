use std::ops::Range;

use gpui::SharedString;
use markdown::{
    ParseOptions,
    mdast::{self, Node},
};

use crate::{
    highlighter::HighlightTheme,
    text::{
        document::ParsedDocument,
        math::MathNode,
        node::{
            self, BlockNode, CodeBlock, ImageNode, InlineNode, LinkMark, NodeContext, Paragraph,
            Span, Table, TableRow, TextMark,
        },
    },
};

/// Parse Markdown into a tree of nodes.
///
/// TODO: Remove `highlight_theme` option, this should in render stage.
pub(crate) fn parse(
    source: &str,
    cx: &mut NodeContext,
    highlight_theme: &HighlightTheme,
) -> Result<ParsedDocument, SharedString> {
    let options = markdown_parse_options();

    markdown::to_mdast(&source, &options)
        .map(|n| {
            let mut parse_cx = MarkdownParseContext::new(source, cx, highlight_theme);
            ast_to_document(n, &mut parse_cx)
        })
        .map_err(|e| e.to_string().into())
}

fn markdown_parse_options() -> ParseOptions {
    let mut options = ParseOptions::gfm();
    configure_markdown_math(&mut options);
    options
}

#[cfg(feature = "markdown-math")]
fn configure_markdown_math(options: &mut ParseOptions) {
    options.constructs.math_flow = true;
    options.constructs.math_text = true;
}

#[cfg(not(feature = "markdown-math"))]
fn configure_markdown_math(_: &mut ParseOptions) {}

struct MarkdownParseContext<'a> {
    source: &'a str,
    node_cx: &'a mut NodeContext,
    highlight_theme: &'a HighlightTheme,
    in_table_cell: bool,
}

impl<'a> MarkdownParseContext<'a> {
    fn new(
        source: &'a str,
        node_cx: &'a mut NodeContext,
        highlight_theme: &'a HighlightTheme,
    ) -> Self {
        Self {
            source,
            node_cx,
            highlight_theme,
            in_table_cell: false,
        }
    }

    fn span(&self, pos: Option<markdown::unist::Position>) -> Option<Span> {
        let pos = pos?;

        Some(Span {
            start: self.node_cx.offset + pos.start.offset,
            end: self.node_cx.offset + pos.end.offset,
        })
    }

    fn inline_math_source(&self, node: &mdast::InlineMath) -> Option<&'a str> {
        let position = node.position.as_ref()?;
        self.source.get(position.start.offset..position.end.offset)
    }

    fn source_for_position(&self, position: Option<&markdown::unist::Position>) -> Option<&'a str> {
        let position = position?;
        self.source.get(position.start.offset..position.end.offset)
    }
}

fn parse_table_row(table: &mut Table, node: &mdast::TableRow, ctx: &mut MarkdownParseContext<'_>) {
    let mut row = TableRow::default();
    node.children.iter().for_each(|c| {
        match c {
            Node::TableCell(cell) => {
                parse_table_cell(&mut row, cell, ctx);
            }
            _ => {}
        };
    });
    table.children.push(row);
}

fn parse_table_cell(
    row: &mut node::TableRow,
    node: &mdast::TableCell,
    ctx: &mut MarkdownParseContext<'_>,
) {
    let mut paragraph = Paragraph::default();
    let was_in_table_cell = ctx.in_table_cell;
    ctx.in_table_cell = true;
    node.children.iter().for_each(|c| {
        parse_paragraph(&mut paragraph, c, ctx);
    });
    ctx.in_table_cell = was_in_table_cell;
    let table_cell = node::TableCell {
        children: paragraph,
        ..Default::default()
    };
    row.children.push(table_cell);
}

/// Push a text run with its existing `marks` plus `new_mark` across the full
/// run.
///
/// If the last mark already covers the full run, merge into it. Otherwise add a
/// new full-run mark. Empty runs are skipped so callers can flush freely.
fn push_merged(
    paragraph: &mut Paragraph,
    text: String,
    marks: Vec<(Range<usize>, TextMark)>,
    new_mark: TextMark,
) {
    if text.is_empty() {
        return;
    }

    let mut node = InlineNode::new(text).marks(marks);
    let len = node.text.len();
    if let Some(last) = node.marks.last_mut()
        && last.0.start == 0
        && last.0.end == len
    {
        last.1.merge(new_mark);
    } else {
        node.marks.push((0..len, new_mark));
    }
    paragraph.push(node);
}

/// Parse `children` and apply `mark` across each emitted text run.
///
/// Nested child marks are kept and shifted to match the combined text for the
/// current run, which lets nested emphasis like `**_x_**` render as both bold
/// and italic. Inline images and math split the run and are emitted as sibling
/// nodes. The return value is the plain text from all children, for callers that
/// need to pass text back to their parent node.
fn merge_children_with_mark(
    paragraph: &mut Paragraph,
    children: &[mdast::Node],
    mark: TextMark,
    ctx: &mut MarkdownParseContext<'_>,
) -> String {
    let mut text = String::new();
    let mut merged_text = String::new();
    let mut merged_marks = Vec::new();

    for child in children {
        let mut child_paragraph = Paragraph::default();
        let child_text = parse_paragraph(&mut child_paragraph, child, ctx);
        text.push_str(&child_text);

        for mut node in child_paragraph.children {
            if node.line_break {
                push_merged(
                    paragraph,
                    std::mem::take(&mut merged_text),
                    std::mem::take(&mut merged_marks),
                    mark.clone(),
                );
                paragraph.push(InlineNode::line_break());
                continue;
            }

            if let Some(mut image) = node.image.take() {
                if let Some(link_mark) = mark.link.clone() {
                    image.link = Some(link_mark);
                }

                // GPUI InteractiveText does not support inline images, so
                // flush the accumulated text run and emit the image as its
                // own sibling InlineNode.
                push_merged(
                    paragraph,
                    std::mem::take(&mut merged_text),
                    std::mem::take(&mut merged_marks),
                    mark.clone(),
                );
                paragraph.push(InlineNode::image(image));
                continue;
            }

            if let Some(math) = node.math.take() {
                push_merged(
                    paragraph,
                    std::mem::take(&mut merged_text),
                    std::mem::take(&mut merged_marks),
                    mark.clone(),
                );
                let source_len = math.source().len();
                let mut marks = node.marks;
                if marks.is_empty() {
                    marks.push((0..source_len, mark.clone()));
                } else {
                    for (_, child_mark) in &mut marks {
                        child_mark.merge(mark.clone());
                    }
                }
                paragraph.push(InlineNode::math(math).marks(marks));
                continue;
            }

            let merged_offset = merged_text.len();
            merged_text.push_str(&node.text);

            for (range, child_mark) in node.marks {
                merged_marks.push((
                    range.start + merged_offset..range.end + merged_offset,
                    child_mark,
                ));
            }
        }
    }

    push_merged(paragraph, merged_text, merged_marks, mark);
    text
}

fn parse_paragraph(
    paragraph: &mut Paragraph,
    node: &mdast::Node,
    ctx: &mut MarkdownParseContext<'_>,
) -> String {
    let span = node.position().map(|pos| Span {
        start: ctx.node_cx.offset + pos.start.offset,
        end: ctx.node_cx.offset + pos.end.offset,
    });
    if let Some(span) = span {
        paragraph.set_span(span);
    }

    let mut text = String::new();

    match node {
        Node::Paragraph(val) => {
            val.children.iter().for_each(|c| {
                text.push_str(&parse_paragraph(paragraph, c, ctx));
            });
        }
        Node::Text(val) => {
            text = val.value.clone();
            paragraph.push_str(&val.value)
        }
        Node::Emphasis(val) => {
            text = merge_children_with_mark(
                paragraph,
                &val.children,
                TextMark::default().italic(),
                ctx,
            );
        }
        Node::Strong(val) => {
            text =
                merge_children_with_mark(paragraph, &val.children, TextMark::default().bold(), ctx);
        }
        Node::Delete(val) => {
            text = merge_children_with_mark(
                paragraph,
                &val.children,
                TextMark::default().strikethrough(),
                ctx,
            );
        }
        Node::InlineCode(val) => {
            text = val.value.clone();
            paragraph.push(
                InlineNode::new(&text).marks(vec![(0..text.len(), TextMark::default().code())]),
            );
        }
        Node::Link(val) => {
            let link_mark = Some(LinkMark {
                url: val.url.clone().into(),
                title: val.title.clone().map(|s| s.into()),
                ..Default::default()
            });

            text = merge_children_with_mark(
                paragraph,
                &val.children,
                TextMark {
                    link: link_mark,
                    ..Default::default()
                },
                ctx,
            );
        }
        Node::Image(raw) => {
            paragraph.push_image(ImageNode {
                url: raw.url.clone().into(),
                title: raw.title.clone().map(|t| t.into()),
                alt: Some(raw.alt.clone().into()),
                ..Default::default()
            });
        }
        Node::InlineMath(raw) => {
            text = parse_inline_math(paragraph, raw, ctx);
        }
        Node::Break(_) => {
            text = "\n".to_owned();
            paragraph.push(InlineNode::line_break());
        }
        Node::MdxTextExpression(raw) => {
            text = raw.value.clone();
            paragraph
                .push(InlineNode::new(&text).marks(vec![(0..text.len(), TextMark::default())]));
        }
        Node::Html(val) => match super::html::parse(&val.value, &mut *ctx.node_cx) {
            Ok(el) => {
                if el
                    .blocks
                    .first()
                    .map(|node| node.is_break())
                    .unwrap_or(false)
                {
                    text = "\n".to_owned();
                    paragraph.push(InlineNode::line_break());
                } else {
                    if cfg!(debug_assertions) {
                        tracing::warn!("unsupported inline html tag: {:#?}", el);
                    }
                }
            }
            Err(err) => {
                if cfg!(debug_assertions) {
                    tracing::warn!("failed parsing html: {:#?}", err);
                }

                text.push_str(&val.value);
            }
        },
        Node::FootnoteReference(foot) => {
            let prefix = format!("[{}]", foot.identifier);
            paragraph.push(InlineNode::new(&prefix).marks(vec![(
                0..prefix.len(),
                TextMark {
                    italic: true,
                    ..Default::default()
                },
            )]));
        }
        Node::LinkReference(link) => {
            let link_mark = LinkMark {
                url: "".into(),
                title: link.label.clone().map(Into::into),
                identifier: Some(link.identifier.clone().into()),
            };

            text = merge_children_with_mark(
                paragraph,
                &link.children,
                TextMark {
                    link: Some(link_mark),
                    ..Default::default()
                },
                ctx,
            );
        }
        _ => {
            if cfg!(debug_assertions) {
                tracing::warn!("unsupported inline node: {:#?}", node);
            }
        }
    }

    text
}

fn parse_inline_math(
    paragraph: &mut Paragraph,
    raw: &mdast::InlineMath,
    ctx: &MarkdownParseContext<'_>,
) -> String {
    let mut text = raw.value.clone();
    let raw_source = ctx.inline_math_source(raw);
    let raw_display = raw_source.map(|s| s.starts_with("$$")).unwrap_or(false);
    let display = raw_display && !ctx.in_table_cell;
    let math_source = if raw_display {
        raw_source
            .and_then(|s| s.strip_prefix("$$")?.strip_suffix("$$"))
            .map(str::trim)
            .unwrap_or(&text)
    } else {
        text.as_str()
    };

    match MathNode::try_new(math_source, display) {
        Ok(math) => {
            let math = if raw_display && let Some(source) = raw_source {
                math.with_markdown_source(source)
            } else {
                math
            };
            paragraph.push(InlineNode::math(math));
        }
        Err(_) => {
            if let Some(raw_source) = raw_source {
                text = raw_source.to_string();
                paragraph.push_str(raw_source);
            } else {
                let fallback = format!("${text}$");
                paragraph.push(InlineNode::new(&fallback));
            }
        }
    }

    text
}

fn ast_to_document(root: mdast::Node, ctx: &mut MarkdownParseContext<'_>) -> ParsedDocument {
    let root = match root {
        Node::Root(r) => r,
        _ => panic!("expected root node"),
    };

    let blocks = root
        .children
        .into_iter()
        .map(|c| ast_to_node(c, ctx))
        .collect();
    ParsedDocument {
        source: ctx.source.to_string().into(),
        blocks,
    }
}

/// Setext heading false-positive inside list items: the markdown parser may
/// interpret `- [x] text\n  -` as a Heading inside a ListItem because it treats
/// the lone `-` on the continuation line as a setext underline. Only demote
/// that checkbox-prefixed case and recover the checkbox state that the parser
/// missed.
fn fix_setext_false_positive_in_list_item(
    children: &mut Vec<BlockNode>,
    checked: &mut Option<bool>,
) {
    for child in children.iter_mut() {
        if let BlockNode::Heading {
            children: paragraph,
            ..
        } = child
        {
            // Only demote the checkbox-prefixed false-positive; leave other
            // headings inside list items untouched.
            if let Some((is_checked, stripped_len)) = detect_checkbox_prefix(paragraph) {
                if checked.is_none() {
                    *checked = Some(is_checked);
                }
                strip_paragraph_prefix(paragraph, stripped_len);
            } else {
                continue;
            }
            *child = BlockNode::Paragraph(paragraph.take());
        }
    }
}

/// Detect "[x] " or "[ ] " at the start of a paragraph's text.
/// Returns (is_checked, byte_count_to_strip) if found.
fn detect_checkbox_prefix(paragraph: &Paragraph) -> Option<(bool, usize)> {
    let first = paragraph.children.first()?;
    let text = first.text.as_ref();
    if let Some(rest) = text.strip_prefix("[x] ") {
        Some((true, text.len() - rest.len()))
    } else if let Some(rest) = text.strip_prefix("[ ] ") {
        Some((false, text.len() - rest.len()))
    } else {
        None
    }
}

/// Strip `count` bytes from the beginning of the first text node in the
/// paragraph.
fn strip_paragraph_prefix(paragraph: &mut Paragraph, count: usize) {
    let Some(first) = paragraph.children.first_mut() else {
        return;
    };
    let prefix_len = count.min(first.text.len());
    let new_text: SharedString = first.text[prefix_len..].to_string().into();
    first.text = new_text;
    // Shift all marks to account for the removed prefix
    for (range, _) in &mut first.marks {
        range.start = range.start.saturating_sub(prefix_len);
        range.end = range.end.saturating_sub(prefix_len);
    }
    first.marks.retain(|(range, _)| range.end > 0);
}

fn ast_to_node(value: mdast::Node, ctx: &mut MarkdownParseContext<'_>) -> BlockNode {
    match value {
        Node::Root(_) => unreachable!("node::Root should be handled separately"),
        Node::Paragraph(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c, ctx);
            });
            paragraph.span = ctx.span(val.position);
            BlockNode::Paragraph(paragraph)
        }
        Node::Blockquote(val) => {
            let children = val
                .children
                .into_iter()
                .map(|c| ast_to_node(c, ctx))
                .collect();
            BlockNode::Blockquote {
                children,
                span: ctx.span(val.position),
            }
        }
        Node::List(list) => {
            let children = list
                .children
                .into_iter()
                .map(|c| ast_to_node(c, ctx))
                .collect();
            BlockNode::List {
                ordered: list.ordered,
                children,
                span: ctx.span(list.position),
            }
        }
        Node::ListItem(val) => {
            let mut children: Vec<BlockNode> = val
                .children
                .into_iter()
                .map(|c| ast_to_node(c, ctx))
                .collect();

            // Setext heading false-positive: `- [x] text\n  -` can be parsed as
            // a Heading inside a ListItem. Since headings don't belong inside
            // list items, demote them to paragraphs.
            let mut checked = val.checked;
            fix_setext_false_positive_in_list_item(&mut children, &mut checked);

            BlockNode::ListItem {
                children,
                spread: val.spread,
                checked,
                span: ctx.span(val.position),
            }
        }
        Node::Break(val) => BlockNode::Break {
            html: false,
            span: ctx.span(val.position),
        },
        Node::Code(raw) => BlockNode::CodeBlock(CodeBlock::new(
            raw.value.into(),
            raw.lang.map(|s| s.into()),
            ctx.highlight_theme,
            ctx.span(raw.position),
        )),
        Node::Heading(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c, ctx);
            });

            BlockNode::Heading {
                level: val.depth,
                children: paragraph,
                span: ctx.span(val.position),
            }
        }
        Node::Math(val) => {
            let markdown_source = ctx
                .source_for_position(val.position.as_ref())
                .map(SharedString::from)
                .unwrap_or_else(|| format!("$$\n{}\n$$", val.value).into());
            let span = ctx.span(val.position);
            match MathNode::try_new(val.value.clone(), true) {
                Ok(math) => BlockNode::Math(math.with_span(span)),
                Err(_) => BlockNode::Math(
                    MathNode::fallback(val.value, markdown_source, true).with_span(span),
                ),
            }
        }
        Node::Html(val) => match super::html::parse(&val.value, &mut *ctx.node_cx) {
            Ok(el) => BlockNode::Root {
                children: el.blocks,
                span: ctx.span(val.position),
            },
            Err(err) => {
                if cfg!(debug_assertions) {
                    tracing::warn!("error parsing html: {:#?}", err);
                }

                BlockNode::Paragraph(Paragraph::new(val.value))
            }
        },
        Node::MdxFlowExpression(val) => BlockNode::CodeBlock(CodeBlock::new(
            val.value.into(),
            Some("mdx".into()),
            ctx.highlight_theme,
            ctx.span(val.position),
        )),
        Node::Yaml(val) => BlockNode::CodeBlock(CodeBlock::new(
            val.value.into(),
            Some("yml".into()),
            ctx.highlight_theme,
            ctx.span(val.position),
        )),
        Node::Toml(val) => BlockNode::CodeBlock(CodeBlock::new(
            val.value.into(),
            Some("toml".into()),
            ctx.highlight_theme,
            ctx.span(val.position),
        )),
        Node::MdxJsxTextElement(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c, ctx);
            });
            paragraph.span = ctx.span(val.position);
            BlockNode::Paragraph(paragraph)
        }
        Node::MdxJsxFlowElement(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c, ctx);
            });
            paragraph.span = ctx.span(val.position);
            BlockNode::Paragraph(paragraph)
        }
        Node::ThematicBreak(val) => BlockNode::HorizontalRule {
            span: ctx.span(val.position),
        },
        Node::Table(val) => {
            let mut table = Table::default();
            table.column_aligns = val
                .align
                .clone()
                .into_iter()
                .map(|align| align.into())
                .collect();
            val.children.iter().for_each(|c| {
                if let Node::TableRow(row) = c {
                    parse_table_row(&mut table, row, ctx);
                }
            });
            table.span = ctx.span(val.position);

            BlockNode::Table(table)
        }
        Node::FootnoteDefinition(def) => {
            let mut paragraph = Paragraph::default();
            let prefix = format!("[{}]: ", def.identifier);
            paragraph.push(InlineNode::new(&prefix).marks(vec![(
                0..prefix.len(),
                TextMark {
                    italic: true,
                    ..Default::default()
                },
            )]));

            def.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c, ctx);
            });
            paragraph.span = ctx.span(def.position);
            BlockNode::Paragraph(paragraph)
        }
        Node::Definition(def) => {
            ctx.node_cx.add_ref(
                def.identifier.clone().into(),
                LinkMark {
                    url: def.url.clone().into(),
                    identifier: Some(def.identifier.clone().into()),
                    title: def.title.clone().map(Into::into),
                },
            );

            BlockNode::Definition {
                identifier: def.identifier.clone().into(),
                url: def.url.clone().into(),
                title: def.title.clone().map(|s| s.into()),
                span: ctx.span(def.position),
            }
        }
        _ => {
            if cfg!(debug_assertions) {
                tracing::warn!("unsupported node: {:#?}", value);
            }
            BlockNode::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nested_emphasis_merges_text_marks() {
        let mut cx = NodeContext::default();
        let document = parse(
            "This has **_bold and italic_** text.",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("expected paragraph");
        };

        let bold_italic = paragraph
            .children
            .iter()
            .find(|child| child.text.as_ref() == "bold and italic")
            .expect("expected emphasized text");

        assert!(
            bold_italic
                .marks
                .iter()
                .any(|(_, mark)| mark.bold && mark.italic),
            "nested emphasis should produce a bold and italic mark"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_inline_math_parses_to_math_node() {
        let mut cx = NodeContext::default();
        let document = parse(
            "This has inline math $x^2 + y^2 = z^2$.",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("expected paragraph");
        };

        assert!(
            paragraph.children.iter().any(|child| child.math.is_some()),
            "inline math should produce a math inline node"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_inline_math_keeps_surrounding_text_nodes() {
        let mut cx = NodeContext::default();
        let source = concat!(
            "每个测试用例包含一行，有三个整数 $n, a, b$",
            "（$1 \\le n, a, b \\le 10^8$）——学生人数、个人密钥的价格和团体密钥的价格。",
        );
        let document = parse(source, &mut cx, &HighlightTheme::default_light()).unwrap();

        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("expected paragraph");
        };

        assert_eq!(paragraph.children.len(), 5);
        assert_eq!(
            paragraph.children[0].text.as_ref(),
            "每个测试用例包含一行，有三个整数 "
        );
        assert_eq!(
            paragraph.children[1]
                .math
                .as_ref()
                .unwrap()
                .source()
                .as_ref(),
            "n, a, b"
        );
        assert_eq!(paragraph.children[2].text.as_ref(), "（");
        let math = paragraph.children[3].math.as_ref().unwrap();
        assert_eq!(math.source().as_ref(), "1 \\le n, a, b \\le 10^8");
        assert!(!math.is_display());
        assert_eq!(
            paragraph.children[4].text.as_ref(),
            "）——学生人数、个人密钥的价格和团体密钥的价格。"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_inline_math_soft_break_is_explicit_line_break() {
        let mut cx = NodeContext::default();
        let document = parse(
            "This has inline math $x^2$ before\n$y^2$ after.",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("expected paragraph");
        };

        let break_ix = paragraph
            .children
            .iter()
            .position(|child| child.line_break)
            .expect("expected an explicit line break node");

        assert!(
            paragraph.children[break_ix + 1].math.is_some(),
            "math after a source newline should start after the explicit break"
        );
        assert!(
            paragraph
                .children
                .iter()
                .filter(|child| !child.line_break)
                .all(|child| !child.text.contains('\n')),
            "source newlines should not remain inside text boxes when inline math is present"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_nested_inline_math_stays_math_node() {
        let mut cx = NodeContext::default();
        let document = parse(
            "This has *inline math $x^2$*.",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("expected paragraph");
        };

        assert!(
            paragraph.children.iter().any(|child| child.math.is_some()),
            "nested inline math should stay a math node"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_double_dollar_math_in_paragraph_renders_as_display_math() {
        let mut cx = NodeContext::default();
        let document = parse(
            "This has $$x^2$$ text.",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("expected paragraph");
        };

        assert!(
            paragraph.children.iter().any(|child| child.math.is_some()),
            "double-dollar math in a paragraph should produce a display math node"
        );
        assert_eq!(document.to_markdown(), "This has $$x^2$$ text.");
        let math = paragraph
            .children
            .iter()
            .find_map(|child| child.math.as_ref())
            .expect("expected display math");
        math.select_all_for_test();
        assert_eq!(paragraph.selected_text(), "$$x^2$$");

        let round_tripped = parse(
            &document.to_markdown(),
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();
        let BlockNode::Paragraph(paragraph) = &round_tripped.blocks[0] else {
            panic!("expected paragraph");
        };
        assert!(
            paragraph.children.iter().any(|child| child
                .math
                .as_ref()
                .map(|math| math.is_display())
                == Some(true)),
            "double-dollar math in a paragraph should round-trip as display math"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_double_dollar_math_in_table_cell_renders_inline() {
        let mut cx = NodeContext::default();
        let document = parse(
            "| Name | Formula |\n|---|---|\n| Inline | $$x^2+y^2=z^2$$ |",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::Table(table) = &document.blocks[0] else {
            panic!("expected table");
        };
        let math = table.children[1].children[1]
            .children
            .children
            .iter()
            .find_map(|child| child.math.as_ref())
            .expect("expected table cell math");

        assert!(
            !math.is_display(),
            "double-dollar math in a table cell should render inline"
        );
        assert_eq!(math.markdown_source().as_ref(), "$$x^2+y^2=z^2$$");
        assert_eq!(
            document.to_markdown(),
            "Name | Formula\n:-- | :--\nInline | $$x^2+y^2=z^2$$"
        );
    }

    #[test]
    fn test_escaped_dollar_text_does_not_parse_as_math() {
        let mut cx = NodeContext::default();
        let document = parse(
            "Price is \\$5 and \\$10 today.",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("expected paragraph");
        };

        assert!(
            paragraph.children.iter().all(|child| child.math.is_none()),
            "escaped dollar text should not produce math nodes"
        );
        assert_eq!(
            paragraph.children[0].text.as_ref(),
            "Price is $5 and $10 today."
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_unescaped_dollar_amounts_follow_math_dialect() {
        let mut cx = NodeContext::default();
        let source = "Price is $5 and $10 today.";
        let document = parse(source, &mut cx, &HighlightTheme::default_light()).unwrap();

        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("expected paragraph");
        };

        assert_eq!(paragraph.children.len(), 3);
        assert_eq!(paragraph.children[0].text.as_ref(), "Price is ");
        assert_eq!(
            paragraph.children[1]
                .math
                .as_ref()
                .map(|math| math.source().as_ref()),
            Some("5 and ")
        );
        assert_eq!(paragraph.children[2].text.as_ref(), "10 today.");
        assert_eq!(document.to_markdown(), source);
    }

    #[cfg(not(feature = "markdown-math"))]
    #[test]
    fn test_dollar_math_source_stays_text_when_math_feature_disabled() {
        let mut cx = NodeContext::default();
        let source = "$x$ and $$y^2$$";
        let document = parse(source, &mut cx, &HighlightTheme::default_light()).unwrap();

        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("expected paragraph");
        };

        assert!(
            paragraph.children.iter().all(|child| child.math.is_none()),
            "math delimiters should stay plain text when markdown-math is disabled"
        );
        assert_eq!(paragraph.text(), source);
        assert_eq!(document.to_markdown(), source);
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_linked_inline_math_keeps_link_mark() {
        let mut cx = NodeContext::default();
        let document = parse(
            "[See $x^2$](https://example.com).",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("expected paragraph");
        };

        let math = paragraph
            .children
            .iter()
            .find(|child| child.math.is_some())
            .expect("expected inline math");
        let link = math
            .marks
            .iter()
            .find_map(|(_, mark)| mark.link.as_ref())
            .expect("expected inline math to inherit the link mark");

        assert_eq!(link.url.as_ref(), "https://example.com");
        assert_eq!(document.to_markdown(), "[See $x^2$](https://example.com).");

        let document = parse(
            "*[See $x^2$](https://example.com).*",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("expected paragraph");
        };

        let math = paragraph
            .children
            .iter()
            .find(|child| child.math.is_some())
            .expect("expected inline math");
        let mark = math
            .marks
            .iter()
            .map(|(_, mark)| mark)
            .next()
            .expect("expected inline math marks");

        assert!(mark.italic, "expected outer emphasis to survive");
        assert_eq!(
            mark.link.as_ref().map(|link| link.url.as_ref()),
            Some("https://example.com")
        );
        assert_eq!(
            document.to_markdown(),
            "*[See $x^2$](https://example.com).*"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_styled_inline_math_to_markdown_keeps_style() {
        let mut cx = NodeContext::default();
        let document = parse(
            "This has *$x^2$* and **$y^2$**.",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        assert_eq!(document.to_markdown(), "This has *$x^2$* and **$y^2$**.");
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_inline_math_to_markdown_does_not_duplicate_source() {
        let mut cx = NodeContext::default();
        let document = parse(
            "This has $x^2$ inline math.",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        assert_eq!(document.to_markdown(), "This has $x^2$ inline math.");
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_block_math_parses_to_math_node() {
        let mut cx = NodeContext::default();
        let document = parse(
            "$$\n\\begin{aligned}\nx^2 + y^2 &= z^2 \\\\\nx^3 + y^3 &= z^3\n\\end{aligned}\n$$",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        assert!(
            matches!(document.blocks[0], BlockNode::Math(_)),
            "block math should produce a math block node"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_unsupported_block_math_preserves_markdown_source() {
        let mut cx = NodeContext::default();
        let source = "$$\n\\frac{1\n$$";
        assert!(
            MathNode::try_new("\\frac{1", true).is_err(),
            "test input should exercise the unsupported math fallback"
        );

        let document = parse(source, &mut cx, &HighlightTheme::default_light()).unwrap();

        let BlockNode::Math(math) = &document.blocks[0] else {
            panic!("expected unsupported block math to remain a math node");
        };

        assert_eq!(math.markdown_source().as_ref(), source);
        assert_eq!(document.to_markdown(), source);

        let round_tripped = parse(
            &document.to_markdown(),
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();
        assert!(
            matches!(round_tripped.blocks[0], BlockNode::Math(_)),
            "unsupported block math should not round-trip as a code block"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_list_item_block_math_parses_to_math_node() {
        let mut cx = NodeContext::default();
        let document = parse(
            "1. A list item\n\n   $$\n   x^2 + y^2 = z^2\n   $$",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::List { children, .. } = &document.blocks[0] else {
            panic!("expected list");
        };
        let BlockNode::ListItem { children, .. } = &children[0] else {
            panic!("expected list item");
        };

        assert!(
            children
                .iter()
                .any(|child| matches!(child, BlockNode::Math(_))),
            "list item block math should parse to a math block node"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_list_item_block_math_to_markdown_keeps_list_scope() {
        let mut cx = NodeContext::default();
        let source = "1. A list item\n\n   $$\n   x^2 + y^2 = z^2\n   $$";
        let document = parse(source, &mut cx, &HighlightTheme::default_light()).unwrap();

        assert_eq!(document.to_markdown(), source);

        let round_tripped = parse(
            &document.to_markdown(),
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();
        let BlockNode::List { children, .. } = &round_tripped.blocks[0] else {
            panic!("expected list");
        };
        let BlockNode::ListItem { children, .. } = &children[0] else {
            panic!("expected list item");
        };

        assert!(
            children
                .iter()
                .any(|child| matches!(child, BlockNode::Math(_))),
            "round-tripped block math should remain inside the list item"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_list_item_starting_with_block_math_to_markdown_keeps_list_scope() {
        let mut cx = NodeContext::default();
        let source = "- $$\n  x^2 + y^2 = z^2\n  $$";
        let document = parse(source, &mut cx, &HighlightTheme::default_light()).unwrap();

        assert_eq!(document.to_markdown(), source);

        let round_tripped = parse(
            &document.to_markdown(),
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();
        let BlockNode::List { children, .. } = &round_tripped.blocks[0] else {
            panic!("expected list");
        };
        let BlockNode::ListItem { children, .. } = &children[0] else {
            panic!("expected list item");
        };

        assert!(
            children
                .iter()
                .any(|child| matches!(child, BlockNode::Math(_))),
            "round-tripped block math should remain inside the list item"
        );
    }

    #[test]
    fn test_todo_list_parses_with_nested_list() {
        let mut cx = NodeContext::default();
        let document = parse(
            "- [x] foefewigweg\n  - subtask",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::List {
            children, ordered, ..
        } = &document.blocks[0]
        else {
            panic!("expected list, got {:?}", document.blocks[0]);
        };
        assert!(!ordered);

        let BlockNode::ListItem {
            children: item1_children,
            checked,
            ..
        } = &children[0]
        else {
            panic!("expected list item");
        };
        assert_eq!(*checked, Some(true));

        assert_eq!(item1_children.len(), 2);

        let BlockNode::List {
            children: nested, ..
        } = &item1_children[1]
        else {
            panic!("expected nested list as second child");
        };
        assert_eq!(nested.len(), 1);

        let BlockNode::ListItem {
            checked: nested_checked,
            ..
        } = &nested[0]
        else {
            panic!("expected nested list item");
        };
        assert_eq!(*nested_checked, None);
    }

    #[test]
    fn test_setext_heading_false_positive_demoted_to_paragraph() {
        let mut cx = NodeContext::default();
        // "- [x] text\n  -" triggers setext heading false-positive.
        // The Heading should be demoted back to Paragraph.
        let document = parse(
            "- [x] foefewigweg\n  -",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::List { children, .. } = &document.blocks[0] else {
            panic!("expected list");
        };
        let BlockNode::ListItem {
            children: item_children,
            checked,
            ..
        } = &children[0]
        else {
            panic!("expected list item");
        };

        // The checkbox should still be recognized
        assert_eq!(*checked, Some(true));

        // Children should contain a Paragraph (demoted from Heading), not a Heading
        assert!(
            item_children
                .iter()
                .all(|c| !matches!(c, BlockNode::Heading { .. })),
            "heading should have been demoted to paragraph"
        );
        assert!(
            item_children
                .iter()
                .any(|c| matches!(c, BlockNode::Paragraph(_))),
            "should have a paragraph child"
        );
    }

    #[test]
    fn test_atx_heading_in_list_item_stays_heading() {
        let mut cx = NodeContext::default();
        let document = parse(
            "- # Heading in list",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::List { children, .. } = &document.blocks[0] else {
            panic!("expected list");
        };
        let BlockNode::ListItem {
            children: item_children,
            checked,
            ..
        } = &children[0]
        else {
            panic!("expected list item");
        };

        assert_eq!(*checked, None);
        assert!(
            item_children
                .iter()
                .any(|c| matches!(c, BlockNode::Heading { level: 1, .. })),
            "ATX heading inside a list item should stay a heading"
        );
    }

    #[test]
    fn test_nested_heading_in_list_item_stays_heading() {
        let mut cx = NodeContext::default();
        let document = parse(
            "- item\n\n  ## Nested heading",
            &mut cx,
            &HighlightTheme::default_light(),
        )
        .unwrap();

        let BlockNode::List { children, .. } = &document.blocks[0] else {
            panic!("expected list");
        };
        let BlockNode::ListItem {
            children: item_children,
            checked,
            ..
        } = &children[0]
        else {
            panic!("expected list item");
        };

        assert_eq!(*checked, None);
        assert!(
            item_children
                .iter()
                .any(|c| matches!(c, BlockNode::Heading { level: 2, .. })),
            "nested heading inside a list item should stay a heading"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_common_math_inputs_parse() {
        for source in [
            "\\begin{aligned}x&=1\\\\y&=2\\end{aligned}",
            "\\begin{equation}e^{i\\pi}+1=0\\end{equation}",
            "\\begin{cases}x^2,&x\\ge0\\\\-x,&x<0\\end{cases}",
            "\\begin{bmatrix}1&0\\\\0&1\\end{bmatrix}",
            "\\color{red}{x} + \\textcolor{blue}{y}",
        ] {
            MathNode::try_new(source, true).unwrap_or_else(|err| {
                panic!("expected ratex to parse `{source}`, got {err}");
            });
        }
    }
}
