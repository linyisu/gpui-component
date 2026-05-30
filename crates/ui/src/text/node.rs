use std::{
    cell::RefCell,
    collections::HashMap,
    ops::Range,
    sync::{Arc, Mutex},
};

use gpui::{
    AnyElement, App, DefiniteLength, Div, ElementId, FontWeight, HighlightStyle,
    InteractiveElement as _, IntoElement, Length, ParentElement, SharedString, SharedUri, Styled,
    Window, div, prelude::FluentBuilder as _, px, relative, rems,
};
use markdown::mdast;
use ropey::Rope;

use crate::{
    ActiveTheme as _, StyledExt,
    highlighter::{HighlightTheme, LanguageRegistry, SyntaxHighlighter},
    input::{InputEdit, Point, RopeExt as _},
    text::{
        CodeBlockActionsFn,
        document::{ListItemPrefix, NodeRenderOptions},
        inline::{InlineState, ParagraphInlineLayout},
        math::MathNode,
    },
    v_flex,
};

use super::TextViewStyle;

thread_local! {
    static CODE_BLOCK_HIGHLIGHTERS: RefCell<HashMap<SharedString, SyntaxHighlighter>> =
        RefCell::new(HashMap::new());
}

/// The block-level nodes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BlockNode {
    /// Something like a Div container in HTML.
    Root {
        children: Vec<BlockNode>,
        span: Option<Span>,
    },
    Paragraph(Paragraph),
    Heading {
        level: u8,
        children: Paragraph,
        span: Option<Span>,
    },
    Blockquote {
        children: Vec<BlockNode>,
        span: Option<Span>,
    },
    List {
        /// Only contains ListItem, others will be ignored
        children: Vec<BlockNode>,
        ordered: bool,
        span: Option<Span>,
    },
    ListItem {
        children: Vec<BlockNode>,
        spread: bool,
        /// Whether the list item is checked, if None, it's not a checkbox
        checked: Option<bool>,
        span: Option<Span>,
    },
    CodeBlock(CodeBlock),
    Math(MathNode),
    Table(Table),
    Break {
        html: bool,
        span: Option<Span>,
    },
    HorizontalRule {
        span: Option<Span>,
    },
    /// Link reference definition retained for markdown serialization.
    Definition {
        identifier: SharedString,
        url: SharedString,
        title: Option<SharedString>,
        span: Option<Span>,
    },
    Unknown,
}

#[derive(Clone, Copy)]
enum BlockTextKind {
    All,
    Selected,
}

impl BlockNode {
    pub(super) fn is_list_item(&self) -> bool {
        matches!(self, Self::ListItem { .. })
    }

    pub(super) fn is_break(&self) -> bool {
        matches!(self, Self::Break { .. })
    }

    /// Combine all children, omitting the empt parent nodes.
    pub(super) fn compact(self) -> BlockNode {
        match self {
            Self::Root { mut children, .. } if children.len() == 1 => children.remove(0).compact(),
            _ => self,
        }
    }

    /// Get the span of the node.
    pub(crate) fn span(&self) -> Option<Span> {
        match self {
            BlockNode::Root { span, .. } => *span,
            BlockNode::Paragraph(paragraph) => paragraph.span,
            BlockNode::Heading { span, .. } => *span,
            BlockNode::Blockquote { span, .. } => *span,
            BlockNode::List { span, .. } => *span,
            BlockNode::ListItem { span, .. } => *span,
            BlockNode::CodeBlock(code_block) => code_block.span,
            BlockNode::Math(math) => math.span(),
            BlockNode::Table(table) => table.span,
            BlockNode::Break { span, .. } => *span,
            BlockNode::HorizontalRule { span, .. } => *span,
            BlockNode::Definition { span, .. } => *span,
            BlockNode::Unknown { .. } => None,
        }
    }

    pub(super) fn text(&self) -> String {
        self.text_by_kind(BlockTextKind::All)
    }

    pub(super) fn selected_text(&self) -> String {
        self.text_by_kind(BlockTextKind::Selected)
    }

    fn text_by_kind(&self, kind: BlockTextKind) -> String {
        let mut text = String::new();
        match self {
            BlockNode::Root { children, .. } => {
                let block_text = Self::children_text(children, kind);
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            BlockNode::Paragraph(paragraph) => {
                let block_text = match kind {
                    BlockTextKind::All => paragraph.text(),
                    BlockTextKind::Selected => paragraph.selected_text(),
                };
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            BlockNode::Heading { children, .. } => {
                let block_text = match kind {
                    BlockTextKind::All => children.text(),
                    BlockTextKind::Selected => children.selected_text(),
                };
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            BlockNode::List { children, .. } | BlockNode::ListItem { children, .. } => {
                text.push_str(&Self::children_text(children, kind));
            }
            BlockNode::Blockquote { children, .. } => {
                let block_text = Self::children_text(children, kind);

                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            BlockNode::Table(table) => {
                let mut block_text = String::new();
                for row in table.children.iter() {
                    let mut row_texts = vec![];
                    for cell in row.children.iter() {
                        let cell_text = match kind {
                            BlockTextKind::All => cell.children.text(),
                            BlockTextKind::Selected => cell.children.selected_text(),
                        };
                        if matches!(kind, BlockTextKind::All) || !cell_text.is_empty() {
                            row_texts.push(cell_text);
                        }
                    }
                    if !row_texts.is_empty() {
                        block_text.push_str(&row_texts.join(" "));
                        block_text.push('\n');
                    }
                }

                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            BlockNode::CodeBlock(code_block) => {
                let block_text = match kind {
                    BlockTextKind::All => code_block.text(),
                    BlockTextKind::Selected => code_block.selected_text(),
                };
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            BlockNode::Math(math) => {
                let block_text = match kind {
                    BlockTextKind::All => math.markdown_source().to_string(),
                    BlockTextKind::Selected => math.selected_text(),
                };
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            BlockNode::Definition { .. }
            | BlockNode::Break { .. }
            | BlockNode::HorizontalRule { .. }
            | BlockNode::Unknown { .. } => {}
        }

        text
    }

    fn children_text(children: &[BlockNode], kind: BlockTextKind) -> String {
        let mut text = String::new();
        for child in children.iter() {
            text.push_str(&child.text_by_kind(kind));
        }
        text
    }
}

#[allow(unused)]
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LinkMark {
    pub url: SharedString,
    /// Optional identifier for footnotes.
    pub identifier: Option<SharedString>,
    pub title: Option<SharedString>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TextMark {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: bool,
    pub code: bool,
    pub link: Option<LinkMark>,
}

impl TextMark {
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    pub fn code(mut self) -> Self {
        self.code = true;
        self
    }

    pub fn link(mut self, link: impl Into<LinkMark>) -> Self {
        self.link = Some(link.into());
        self
    }

    pub fn merge(&mut self, other: TextMark) {
        self.bold |= other.bold;
        self.italic |= other.italic;
        self.strikethrough |= other.strikethrough;
        self.underline |= other.underline;
        self.code |= other.code;
        if let Some(link) = other.link {
            self.link = Some(link);
        }
    }
}

/// The bytes
#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl From<Span> for ElementId {
    fn from(value: Span) -> Self {
        ElementId::Name(format!("md-{}:{}", value.start, value.end).into())
    }
}

#[allow(unused)]
#[derive(Debug, Default, Clone)]
pub struct ImageNode {
    pub url: SharedUri,
    pub link: Option<LinkMark>,
    pub title: Option<SharedString>,
    pub alt: Option<SharedString>,
    pub width: Option<DefiniteLength>,
    pub height: Option<DefiniteLength>,
}

impl ImageNode {
    pub fn title(&self) -> String {
        self.title
            .clone()
            .unwrap_or_else(|| self.alt.clone().unwrap_or_default())
            .to_string()
    }

    pub(crate) fn plain_text(&self) -> SharedString {
        self.alt.clone().unwrap_or_default()
    }

    pub(crate) fn markdown_source(&self) -> String {
        let alt = self.alt.clone().unwrap_or_default();
        let title = self
            .title
            .clone()
            .map_or(String::new(), |title| format!(" \"{}\"", title));
        let image = format!("![{}]({}{})", alt, self.url, title);
        if let Some(link) = &self.link {
            format!("[{}]({})", image, link.url)
        } else {
            image
        }
    }
}

impl PartialEq for ImageNode {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.link == other.link
            && self.title == other.title
            && self.alt == other.alt
            && self.width == other.width
            && self.height == other.height
    }
}

#[derive(Default, Clone, Debug)]
pub(crate) struct InlineNode {
    /// The text content.
    pub(crate) text: SharedString,
    pub(crate) image: Option<ImageNode>,
    pub(crate) math: Option<MathNode>,
    pub(crate) line_break: bool,
    /// The text styles, each tuple contains the range of the text and the style.
    pub(crate) marks: Vec<(Range<usize>, TextMark)>,

    pub(crate) state: Arc<Mutex<InlineState>>,
}

impl PartialEq for InlineNode {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.image == other.image
            && self.math == other.math
            && self.line_break == other.line_break
            && self.marks == other.marks
    }
}

impl InlineNode {
    pub(crate) fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            image: None,
            math: None,
            line_break: false,
            marks: vec![],
            state: Arc::new(Mutex::new(InlineState::default())),
        }
    }

    pub(crate) fn image(image: ImageNode) -> Self {
        let mut this = Self::new("");
        this.image = Some(image);
        this
    }

    pub(crate) fn math(math: MathNode) -> Self {
        let mut this = Self::new(math.source().clone());
        this.math = Some(math);
        this
    }

    pub(crate) fn line_break() -> Self {
        let mut this = Self::new("\n");
        this.line_break = true;
        this
    }

    pub(crate) fn marks(mut self, marks: Vec<(Range<usize>, TextMark)>) -> Self {
        self.marks = marks;
        self
    }
}

/// The paragraph element, contains multiple text nodes.
///
/// Unlike other Element, this is cloneable, because it is used in the Node AST.
/// We are keep the selection state inside this AST Nodes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Paragraph {
    pub(super) span: Option<Span>,
    pub(super) children: Vec<InlineNode>,
    /// The link references in this paragraph, used for reference links.
    ///
    /// The key is the identifier, the value is the url.
    pub(super) link_refs: HashMap<SharedString, SharedString>,
}

impl PartialEq for Paragraph {
    fn eq(&self, other: &Self) -> bool {
        self.span == other.span
            && self.children == other.children
            && self.link_refs == other.link_refs
    }
}

impl Paragraph {
    pub(crate) fn new(text: String) -> Self {
        let mut paragraph = Self {
            span: None,
            children: vec![],
            link_refs: HashMap::new(),
        };
        paragraph.push_str(&text);
        paragraph
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();
        let mut pending_line_breaks = 0;
        let mut saw_selected_text = false;
        let mut saw_non_break_content_since_last_output = false;

        for c in self.children.iter() {
            let mut selected = String::new();
            let mut selected_starts_at_zero = false;

            if let Ok(state) = c.state.lock() {
                if let Some(selection) = &state.selection {
                    let part_text = state.text.clone();
                    selected.push_str(&part_text[selection.start..selection.end]);
                    selected_starts_at_zero = selection.start == 0;
                }
            }

            if let Some(math) = &c.math {
                selected.push_str(&math.selected_text());
                selected_starts_at_zero = true;
            }

            if !selected.is_empty() {
                if pending_line_breaks > 0 {
                    if saw_selected_text
                        || (!saw_non_break_content_since_last_output && selected_starts_at_zero)
                    {
                        for _ in 0..pending_line_breaks {
                            text.push('\n');
                        }
                    }
                    pending_line_breaks = 0;
                }
                text.push_str(&selected);
                saw_selected_text = true;
                saw_non_break_content_since_last_output = false;
                continue;
            }

            if c.line_break {
                pending_line_breaks += 1;
            } else {
                if pending_line_breaks > 0 {
                    pending_line_breaks = 0;
                }
                saw_non_break_content_since_last_output = true;
            }
        }

        if !text.is_empty() && pending_line_breaks > 0 {
            for _ in 0..pending_line_breaks {
                text.push('\n');
            }
        }

        text
    }

    pub(super) fn text(&self) -> String {
        let mut text = String::new();
        for node in self.children.iter() {
            if node.line_break {
                text.push('\n');
            } else if let Some(image) = &node.image {
                text.push_str(&image.plain_text());
            } else if let Some(math) = &node.math {
                text.push_str(math.markdown_source().as_ref());
            } else {
                text.push_str(&node.text);
            }
        }
        text
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Table {
    pub(crate) children: Vec<TableRow>,
    pub(crate) column_aligns: Vec<ColumnAlign>,
    pub(crate) span: Option<Span>,
}

impl Table {
    pub(crate) fn column_align(&self, index: usize) -> ColumnAlign {
        self.column_aligns.get(index).copied().unwrap_or_default()
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub(crate) enum ColumnAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl From<mdast::AlignKind> for ColumnAlign {
    fn from(value: mdast::AlignKind) -> Self {
        match value {
            mdast::AlignKind::None => ColumnAlign::Left,
            mdast::AlignKind::Left => ColumnAlign::Left,
            mdast::AlignKind::Center => ColumnAlign::Center,
            mdast::AlignKind::Right => ColumnAlign::Right,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TableRow {
    pub children: Vec<TableCell>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TableCell {
    pub children: Paragraph,
    pub width: Option<DefiniteLength>,
}

impl Paragraph {
    pub(crate) fn take(&mut self) -> Paragraph {
        std::mem::replace(
            self,
            Paragraph {
                span: None,
                children: vec![],
                link_refs: Default::default(),
            },
        )
    }

    pub(crate) fn is_image(&self) -> bool {
        false
    }

    pub(crate) fn set_span(&mut self, span: Span) {
        self.span = Some(span);
    }

    pub(crate) fn push_str(&mut self, text: &str) {
        let mut start = 0;

        for (ix, ch) in text.char_indices() {
            if ch == '\n' {
                if start < ix {
                    self.push_text_segment(&text[start..ix]);
                }
                self.children.push(InlineNode::line_break());
                start = ix + ch.len_utf8();
            }
        }

        if start < text.len() {
            self.push_text_segment(&text[start..]);
        }
    }

    pub(crate) fn push(&mut self, text: InlineNode) {
        self.children.push(text);
    }

    pub(crate) fn push_image(&mut self, image: ImageNode) {
        self.children.push(InlineNode::image(image));
    }

    fn push_text_segment(&mut self, text: &str) {
        self.children.push(
            InlineNode::new(text.to_string()).marks(vec![(0..text.len(), TextMark::default())]),
        );
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.children.is_empty()
            || self.children.iter().all(|node| {
                node.text.is_empty()
                    && node.image.is_none()
                    && node.math.is_none()
                    && !node.line_break
            })
    }

    /// Return length of children text.
    pub(crate) fn text_len(&self) -> usize {
        self.children
            .iter()
            .map(|node| node.text.len())
            .sum::<usize>()
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.children.extend(other.children);
    }
}

#[derive(Debug, Clone)]
pub struct CodeBlock {
    lang: Option<SharedString>,
    styles: Arc<Mutex<Option<Vec<(Range<usize>, HighlightStyle)>>>>,
    highlight_theme: Arc<HighlightTheme>,
    state: Arc<Mutex<InlineState>>,
    pub span: Option<Span>,
}

impl PartialEq for CodeBlock {
    fn eq(&self, other: &Self) -> bool {
        self.lang == other.lang && self.code() == other.code() && self.span == other.span
    }
}

impl CodeBlock {
    /// Get the language of the code block.
    pub fn lang(&self) -> Option<SharedString> {
        self.lang.clone()
    }

    /// Get the code content of the code block.
    pub fn code(&self) -> SharedString {
        self.state
            .lock()
            .map(|state| state.text.clone())
            .unwrap_or_default()
    }

    pub(crate) fn new(
        code: SharedString,
        lang: Option<SharedString>,
        highlight_theme: &HighlightTheme,
        span: Option<impl Into<Span>>,
    ) -> Self {
        let state = Arc::new(Mutex::new(InlineState::default()));
        if let Ok(mut state) = state.lock() {
            state.set_text(code);
        }

        Self {
            lang,
            styles: Arc::new(Mutex::new(None)),
            highlight_theme: Arc::new(highlight_theme.clone()),
            state,
            span: span.map(|s| s.into()),
        }
    }

    pub(crate) fn styles(&self) -> Vec<(Range<usize>, HighlightStyle)> {
        let Some(lang) = &self.lang else {
            return Vec::new();
        };

        let Ok(mut styles) = self.styles.lock() else {
            return Vec::new();
        };

        if let Some(styles) = styles.as_ref() {
            return styles.clone();
        }

        let code = self.code();
        let computed_styles = CODE_BLOCK_HIGHLIGHTERS.with(|cache| {
            let mut cache = cache.borrow_mut();
            let highlighter = cache
                .entry(lang.clone())
                .or_insert_with(|| SyntaxHighlighter::new(lang));

            if let Some(config) = LanguageRegistry::singleton().language(lang)
                && highlighter.language() != &config.name
            {
                *highlighter = SyntaxHighlighter::new(lang);
            }

            let old_end_byte = highlighter.text().len();
            let old_end_position = highlighter.text().offset_to_point(old_end_byte);
            let code_rope = Rope::from_str(code.as_str());

            let edit = InputEdit {
                start_byte: 0,
                old_end_byte,
                new_end_byte: code.len(),
                start_position: Point::new(0, 0),
                old_end_position,
                new_end_position: code_rope.offset_to_point(code.len()),
            };

            highlighter.update(Some(edit), &code_rope, None);
            highlighter.styles(&(0..code.len()), &self.highlight_theme)
        });
        *styles = Some(computed_styles.clone());
        computed_styles
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();
        if let Ok(state) = self.state.lock()
            && let Some(selection) = &state.selection
        {
            let part_text = state.text.clone();
            text.push_str(&part_text[selection.start..selection.end]);
        }
        text
    }

    pub(super) fn text(&self) -> String {
        self.state
            .lock()
            .map(|state| state.text.to_string())
            .unwrap_or_default()
    }

    fn render(
        &self,
        options: &NodeRenderOptions,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let style = &node_cx.style;

        div()
            .when(!options.is_last, |this| this.pb(style.paragraph_gap))
            .child(
                div()
                    .id(("codeblock", options.ix))
                    .p_3()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .relative()
                    .refine_style(&style.code_block)
                    .child(ParagraphInlineLayout::highlighted_text(
                        ElementId::Name(format!("code-inline-{}", options.ix).into()),
                        self.code(),
                        self.state.clone(),
                        self.styles(),
                        node_cx.clone(),
                        NodeRenderOptions {
                            is_last: true,
                            list_prefix: None,
                            ..*options
                        },
                    ))
                    .when_some(node_cx.code_block_actions.clone(), |this, actions| {
                        this.child(
                            div()
                                .id("actions")
                                .absolute()
                                .top_2()
                                .right_2()
                                .bg(cx.theme().muted)
                                .rounded(cx.theme().radius)
                                .child(actions(&self, window, cx)),
                        )
                    }),
            )
            .into_any_element()
    }
}

/// A context for rendering nodes, contains link references.
#[derive(Default, Clone)]
pub(crate) struct NodeContext {
    /// The byte offset of the node in the original markdown text.
    /// Used for incremental updates.
    pub(crate) offset: usize,
    pub(crate) link_refs: HashMap<SharedString, LinkMark>,
    pub(crate) style: TextViewStyle,
    pub(crate) code_block_actions: Option<Arc<CodeBlockActionsFn>>,
}

impl NodeContext {
    pub(super) fn add_ref(&mut self, identifier: SharedString, link: LinkMark) {
        self.link_refs.insert(identifier, link);
    }
}

impl PartialEq for NodeContext {
    fn eq(&self, other: &Self) -> bool {
        self.link_refs == other.link_refs && self.style == other.style
        // Note: code_block_buttons is intentionally not compared (closures can't be compared)
    }
}

#[derive(Clone, Default, PartialEq)]
struct InlineMarkdownStyle {
    bold: bool,
    italic: bool,
    strikethrough: bool,
    code: bool,
}

impl InlineMarkdownStyle {
    fn from_mark(mark: &TextMark) -> Self {
        Self {
            bold: mark.bold,
            italic: mark.italic,
            strikethrough: mark.strikethrough,
            code: mark.code,
        }
    }

    fn apply(&self, mut text: String) -> String {
        if self.code {
            text = format!("`{}`", text);
        }
        if self.bold {
            text = format!("**{}**", text);
        }
        if self.italic {
            text = format!("*{}*", text);
        }
        if self.strikethrough {
            text = format!("~~{}~~", text);
        }
        text
    }
}

fn full_range_mark(node: &InlineNode) -> Option<&TextMark> {
    node.marks
        .iter()
        .find(|(range, _)| range.start == 0 && range.end == node.text.len())
        .map(|(_, mark)| mark)
}

struct PendingMarkdown {
    style: InlineMarkdownStyle,
    text: String,
    link: Option<(LinkMark, String)>,
}

impl PendingMarkdown {
    fn new(style: InlineMarkdownStyle) -> Self {
        Self {
            style,
            text: String::new(),
            link: None,
        }
    }

    fn push(&mut self, segment: String, link: Option<LinkMark>) {
        if let Some(link) = link {
            if let Some((pending_link, pending_text)) = self.link.as_mut()
                && *pending_link == link
            {
                pending_text.push_str(&segment);
                return;
            }

            self.flush_link();
            self.link = Some((link, segment));
        } else {
            self.flush_link();
            self.text.push_str(&segment);
        }
    }

    fn flush_link(&mut self) {
        if let Some((link, pending_text)) = self.link.take() {
            self.text
                .push_str(&format!("[{}]({})", pending_text, link.url));
        }
    }

    fn finish(mut self) -> String {
        self.flush_link();
        self.style.apply(self.text)
    }
}

fn push_markdown_segment(
    text: &mut String,
    pending: &mut Option<PendingMarkdown>,
    segment: String,
    link: Option<LinkMark>,
    style: InlineMarkdownStyle,
) {
    if let Some(pending) = pending.as_mut()
        && pending.style == style
    {
        pending.push(segment, link);
        return;
    }

    if let Some(pending) = pending.take() {
        text.push_str(&pending.finish());
    }

    let mut next = PendingMarkdown::new(style);
    next.push(segment, link);
    *pending = Some(next);
}

impl Paragraph {
    fn render(
        &self,
        options: NodeRenderOptions,
        node_cx: &NodeContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        ParagraphInlineLayout::new(
            self.span.map(Into::into).unwrap_or_else(|| {
                ElementId::Name(format!("paragraph-inline-{}", options.ix).into())
            }),
            self.children.clone(),
            node_cx.clone(),
            options,
        )
        .into_any_element()
    }
}

impl Paragraph {
    fn to_inline_markdown(&self) -> String {
        let mut text = String::new();
        let mut pending = None;

        for text_node in &self.children {
            let (segment, link, style) = {
                if let Some(math) = &text_node.math {
                    let text = math.markdown_source().to_string();
                    let (link, style) = full_range_mark(text_node)
                        .map(|mark| (mark.link.clone(), InlineMarkdownStyle::from_mark(mark)))
                        .unwrap_or_default();
                    (text, link, style)
                } else {
                    let mut text = text_node.text.to_string();
                    let mut link = None;
                    let mut markdown_style = InlineMarkdownStyle::default();
                    for (range, style) in &text_node.marks {
                        if range.start == 0 && range.end == text_node.text.len() {
                            if let Some(mark) = &style.link {
                                markdown_style = InlineMarkdownStyle::from_mark(style);
                                link = Some(mark.clone());
                                text = text_node.text[range.clone()].to_string();
                                continue;
                            }
                            if text_node.marks.len() == 1 {
                                markdown_style = InlineMarkdownStyle::from_mark(style);
                                text = text_node.text[range.clone()].to_string();
                                continue;
                            }
                        }

                        if style.bold {
                            text = format!("**{}**", &text_node.text[range.clone()]);
                        }
                        if style.italic {
                            text = format!("*{}*", &text_node.text[range.clone()]);
                        }
                        if style.strikethrough {
                            text = format!("~~{}~~", &text_node.text[range.clone()]);
                        }
                        if style.code {
                            text = format!("`{}`", &text_node.text[range.clone()]);
                        }
                        if let Some(mark) = &style.link {
                            link = Some(mark.clone());
                            text = text_node.text[range.clone()].to_string();
                        }
                    }

                    if let Some(image) = &text_node.image {
                        text.push_str(&image.markdown_source())
                    }

                    (text, link, markdown_style)
                }
            };

            push_markdown_segment(&mut text, &mut pending, segment, link, style);
        }

        if let Some(pending) = pending {
            text.push_str(&pending.finish());
        }

        text
    }

    fn to_markdown(&self) -> String {
        let mut text = self.to_inline_markdown();
        text.push_str("\n\n");
        text
    }
}

fn indent_markdown_continuation_lines(markdown: &str, indent: &str) -> String {
    let mut lines = markdown.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };

    let mut text = first.to_string();
    for line in lines {
        text.push('\n');
        if !line.is_empty() {
            text.push_str(indent);
        }
        text.push_str(line);
    }
    text
}

impl BlockNode {
    /// Converts the node back to markdown-like source.
    ///
    /// This preserves markdown source for round-trip checks and copied content.
    #[allow(dead_code)]
    pub(crate) fn to_markdown(&self) -> String {
        match self {
            BlockNode::Root { children, .. } => children
                .iter()
                .map(|child| child.to_markdown())
                .collect::<Vec<_>>()
                .join("\n\n"),
            BlockNode::Paragraph(paragraph) => paragraph.to_markdown(),
            BlockNode::Heading {
                level, children, ..
            } => {
                let hashes = "#".repeat(*level as usize);
                format!("{} {}", hashes, children.to_markdown())
            }
            BlockNode::Blockquote { children, .. } => {
                let content = children
                    .iter()
                    .map(|child| child.to_markdown())
                    .collect::<Vec<_>>()
                    .join("\n\n");

                content
                    .lines()
                    .map(|line| format!("> {}", line))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            BlockNode::List {
                children, ordered, ..
            } => children
                .iter()
                .enumerate()
                .map(|(i, child)| {
                    let prefix = if *ordered {
                        format!("{}. ", i + 1)
                    } else {
                        "- ".to_string()
                    };
                    let indent = " ".repeat(prefix.len());
                    let content = indent_markdown_continuation_lines(&child.to_markdown(), &indent);
                    format!("{}{}", prefix, content)
                })
                .collect::<Vec<_>>()
                .join("\n"),
            BlockNode::ListItem {
                children, checked, ..
            } => {
                let checkbox = if let Some(checked) = checked {
                    if *checked { "[x] " } else { "[ ] " }
                } else {
                    ""
                };
                format!(
                    "{}{}",
                    checkbox,
                    children
                        .iter()
                        .map(|child| child.to_markdown())
                        .collect::<Vec<_>>()
                        .join("\n\n")
                )
            }
            BlockNode::CodeBlock(code_block) => {
                format!(
                    "```{}\n{}\n```",
                    code_block.lang.clone().unwrap_or_default(),
                    code_block.code()
                )
            }
            BlockNode::Math(math) => math.markdown_source().to_string(),
            BlockNode::Table(table) => {
                let header = table
                    .children
                    .first()
                    .map(|row| {
                        row.children
                            .iter()
                            .map(|cell| cell.children.to_inline_markdown())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .unwrap_or_default();
                let alignments = table
                    .column_aligns
                    .iter()
                    .map(|align| {
                        match align {
                            ColumnAlign::Left => ":--",
                            ColumnAlign::Center => ":-:",
                            ColumnAlign::Right => "--:",
                        }
                        .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                let rows = table
                    .children
                    .iter()
                    .skip(1)
                    .map(|row| {
                        row.children
                            .iter()
                            .map(|cell| cell.children.to_inline_markdown())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{}\n{}\n{}", header, alignments, rows)
            }
            BlockNode::Break { html, .. } => {
                if *html {
                    "<br>".to_string()
                } else {
                    "\n".to_string()
                }
            }
            BlockNode::HorizontalRule { .. } => "---".to_string(),
            BlockNode::Definition {
                identifier,
                url,
                title,
                ..
            } => {
                if let Some(title) = title {
                    format!("[{}]: {} \"{}\"", identifier, url, title)
                } else {
                    format!("[{}]: {}", identifier, url)
                }
            }
            BlockNode::Unknown { .. } => "".to_string(),
        }
        .trim()
        .to_string()
    }
}

impl BlockNode {
    fn render_list_prefix_line(
        prefix: ListItemPrefix,
        options: NodeRenderOptions,
        child_ix: usize,
        node_cx: &NodeContext,
    ) -> Div {
        div().w_full().min_w_0().overflow_hidden().child(
            ParagraphInlineLayout::new(
                ElementId::Name(format!("list-prefix-{}-{}", options.ix, child_ix).into()),
                vec![],
                node_cx.clone(),
                NodeRenderOptions {
                    depth: options.depth + 1,
                    in_list: true,
                    is_last: true,
                    list_prefix: Some(prefix),
                    ..options
                },
            )
            .into_any_element(),
        )
    }

    fn render_list_item(
        item: &BlockNode,
        ix: usize,
        options: NodeRenderOptions,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        match item {
            BlockNode::ListItem {
                children,
                spread,
                checked,
                ..
            } => v_flex()
                .id(("li", options.ix))
                .w_full()
                .min_w_0()
                .when(*spread, |this| this.child(div()))
                .children({
                    let mut items: Vec<Div> = Vec::with_capacity(children.len());
                    let item_prefix = match *checked {
                        Some(checked) => Some(ListItemPrefix::Todo {
                            checked,
                            visible: true,
                        }),
                        None => Some(ListItemPrefix::Marker {
                            ix,
                            ordered: options.ordered,
                            depth: options.depth,
                            visible: true,
                        }),
                    };
                    let mut list_prefix = item_prefix;
                    let list_indent = item_prefix.map(ListItemPrefix::hidden);

                    for (child_ix, child) in children.iter().enumerate() {
                        match child {
                            BlockNode::Paragraph(_) => {
                                let prefix = list_prefix.take().or(list_indent);

                                let text = child.render_block(
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        in_list: true,
                                        is_last: true,
                                        list_prefix: prefix,
                                        ..options
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                );

                                items.push(div().w_full().min_w_0().overflow_hidden().child(text));
                            }
                            BlockNode::List { .. } => {
                                if let Some(prefix) = list_prefix.take() {
                                    items.push(Self::render_list_prefix_line(
                                        prefix, options, child_ix, node_cx,
                                    ));
                                }

                                items.push(div().ml(rems(1.)).child(child.render_block(
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        in_list: true,
                                        is_last: true,
                                        list_prefix: None,
                                        ..options
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                )));
                            }
                            BlockNode::CodeBlock(_)
                            | BlockNode::Table(_)
                            | BlockNode::Blockquote { .. }
                            | BlockNode::HorizontalRule { .. } => {
                                // Keep the list marker at the list-item boundary and
                                // do not leak it into descendant block content.
                                if let Some(prefix) = list_prefix.take() {
                                    items.push(Self::render_list_prefix_line(
                                        prefix, options, child_ix, node_cx,
                                    ));
                                }

                                items.push(div().ml(rems(1.)).child(child.render_block(
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        in_list: true,
                                        is_last: true,
                                        list_prefix: None,
                                        ..options
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                )));
                            }
                            BlockNode::Math(math) => {
                                let prefix = list_prefix.take().or(list_indent);
                                let text = ParagraphInlineLayout::new(
                                    math.span().map(Into::into).unwrap_or_else(|| {
                                        ElementId::Name(
                                            format!("list-math-{}-{}", options.ix, child_ix).into(),
                                        )
                                    }),
                                    vec![InlineNode::math(math.clone())],
                                    node_cx.clone(),
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        in_list: true,
                                        is_last: true,
                                        list_prefix: prefix,
                                        ..options
                                    },
                                )
                                .into_any_element();

                                items.push(div().w_full().min_w_0().overflow_hidden().child(text));
                            }
                            _ => {
                                let prefix = list_prefix.take().or(list_indent);
                                let text = child.render_block(
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        in_list: true,
                                        is_last: true,
                                        list_prefix: prefix,
                                        ..options
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                );
                                items.push(div().w_full().min_w_0().overflow_hidden().child(text));
                            }
                        }
                    }
                    items
                })
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    fn render_table(
        item: &BlockNode,
        options: &NodeRenderOptions,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        const DEFAULT_LENGTH: usize = 5;
        const MAX_LENGTH: usize = 150;
        let col_lens = match item {
            BlockNode::Table(table) => {
                let mut col_lens = vec![];
                for row in table.children.iter() {
                    for (ix, cell) in row.children.iter().enumerate() {
                        if col_lens.len() <= ix {
                            col_lens.push(DEFAULT_LENGTH);
                        }

                        let len = cell.children.text_len();
                        if len > col_lens[ix] {
                            col_lens[ix] = len;
                        }
                    }
                }
                col_lens
            }
            _ => vec![],
        };

        match item {
            BlockNode::Table(table) => div()
                .pb(rems(1.))
                .w_full()
                .child(
                    div()
                        .id(("table", options.ix))
                        .w_full()
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(cx.theme().radius)
                        .overflow_hidden()
                        .children({
                            let mut rows = Vec::with_capacity(table.children.len());
                            for (row_ix, row) in table.children.iter().enumerate() {
                                rows.push(
                                    div()
                                        .id("row")
                                        .w_full()
                                        .when(row_ix < table.children.len() - 1, |this| {
                                            this.border_b_1()
                                        })
                                        .border_color(cx.theme().border)
                                        .flex()
                                        .flex_row()
                                        .children({
                                            let mut cells = Vec::with_capacity(row.children.len());
                                            for (ix, cell) in row.children.iter().enumerate() {
                                                let align = table.column_align(ix);
                                                let is_last_col = ix == row.children.len() - 1;
                                                let len = col_lens
                                                    .get(ix)
                                                    .copied()
                                                    .unwrap_or(MAX_LENGTH)
                                                    .min(MAX_LENGTH);

                                                cells.push(
                                                    div()
                                                        .id(("cell", ix))
                                                        .overflow_hidden()
                                                        .when(
                                                            align == ColumnAlign::Center,
                                                            |this| this.text_center(),
                                                        )
                                                        .when(align == ColumnAlign::Right, |this| {
                                                            this.text_right()
                                                        })
                                                        .min_w_16()
                                                        .w(Length::Definite(relative(len as f32)))
                                                        .px_2()
                                                        .py_1()
                                                        .when(!is_last_col, |this| {
                                                            this.border_r_1()
                                                                .border_color(cx.theme().border)
                                                        })
                                                        .child(cell.children.render(
                                                            NodeRenderOptions {
                                                                column_align: align,
                                                                list_prefix: None,
                                                                ..*options
                                                            },
                                                            node_cx,
                                                            window,
                                                            cx,
                                                        )),
                                                )
                                            }
                                            cells
                                        }),
                                )
                            }
                            rows
                        }),
                )
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    pub(crate) fn render_block(
        &self,
        options: NodeRenderOptions,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let ix = options.ix;
        let mb = if options.in_list || options.is_last {
            rems(0.)
        } else {
            node_cx.style.paragraph_gap
        };

        match self {
            BlockNode::Root { children, .. } => div()
                .id(("div", ix))
                .children(children.into_iter().enumerate().map(move |(ix, node)| {
                    node.render_block(NodeRenderOptions { ix, ..options }, node_cx, window, cx)
                }))
                .into_any_element(),
            BlockNode::Paragraph(paragraph) => div()
                .id(("p", ix))
                .w_full()
                .min_w_0()
                .pb(mb)
                .child(paragraph.render(options, node_cx, window, cx))
                .into_any_element(),
            BlockNode::Heading {
                level, children, ..
            } => {
                let (text_size, font_weight) = match level {
                    1 => (rems(2.), FontWeight::BOLD),
                    2 => (rems(1.5), FontWeight::SEMIBOLD),
                    3 => (rems(1.25), FontWeight::SEMIBOLD),
                    4 => (rems(1.125), FontWeight::SEMIBOLD),
                    5 => (rems(1.), FontWeight::SEMIBOLD),
                    6 => (rems(1.), FontWeight::MEDIUM),
                    _ => (rems(1.), FontWeight::NORMAL),
                };

                let mut text_size = text_size.to_pixels(node_cx.style.heading_base_font_size);
                if let Some(f) = node_cx.style.heading_font_size.as_ref() {
                    text_size = (f)(*level, node_cx.style.heading_base_font_size);
                }

                div()
                    .id(SharedString::from(format!("h{}-{}", level, ix)))
                    .w_full()
                    .min_w_0()
                    .pb(rems(0.3))
                    .whitespace_normal()
                    .text_size(text_size)
                    .font_weight(font_weight)
                    .child(children.render(options, node_cx, window, cx))
                    .into_any_element()
            }
            BlockNode::Blockquote { children, .. } => div()
                .w_full()
                .pb(mb)
                .child(
                    div()
                        .id(("blockquote", ix))
                        .w_full()
                        .text_color(cx.theme().muted_foreground)
                        .border_l_3()
                        .border_color(cx.theme().secondary_active)
                        .px_4()
                        .children({
                            let children_len = children.len();
                            children.into_iter().enumerate().map(move |(index, c)| {
                                let is_last = index == children_len - 1;
                                c.render_block(
                                    NodeRenderOptions {
                                        list_prefix: None,
                                        ..options.is_last(is_last)
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                )
                            })
                        }),
                )
                .into_any_element(),
            BlockNode::List {
                children, ordered, ..
            } => v_flex()
                .id((if *ordered { "ol" } else { "ul" }, ix))
                .w_full()
                .min_w_0()
                .pb(mb)
                .children({
                    let mut items = Vec::with_capacity(children.len());
                    let mut item_index = 0;
                    for (ix, item) in children.into_iter().enumerate() {
                        let is_item = item.is_list_item();

                        items.push(Self::render_list_item(
                            item,
                            item_index,
                            NodeRenderOptions {
                                ix,
                                ordered: *ordered,
                                ..options
                            },
                            node_cx,
                            window,
                            cx,
                        ));

                        if is_item {
                            item_index += 1;
                        }
                    }
                    items
                })
                .into_any_element(),
            BlockNode::CodeBlock(code_block) => code_block.render(&options, node_cx, window, cx),
            BlockNode::Math(math) => div()
                .id(("math", ix))
                .w_full()
                .pb(mb)
                .flex()
                .justify_center()
                .child(math.render())
                .into_any_element(),
            BlockNode::Table { .. } => {
                Self::render_table(self, &options, node_cx, window, cx).into_any_element()
            }
            BlockNode::HorizontalRule { .. } => div()
                .pb(mb)
                .child(div().id("horizontal-rule").bg(cx.theme().border).h(px(2.)))
                .into_any_element(),
            BlockNode::Break { .. } => div().id("break").into_any_element(),
            BlockNode::Unknown { .. } | BlockNode::Definition { .. } => div().into_any_element(),
            _ => {
                if cfg!(debug_assertions) {
                    tracing::warn!("unknown implementation: {:?}", self);
                }

                div().into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_block_equality_includes_code_content() {
        let theme = HighlightTheme::default_light();
        let first = CodeBlock::new(
            "let value = 1;".into(),
            Some("rust".into()),
            &theme,
            None::<Span>,
        );
        let second = CodeBlock::new(
            "let value = 2;".into(),
            Some("rust".into()),
            &theme,
            None::<Span>,
        );

        assert_ne!(first, second);
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn code_block_highlighter_cache_refreshes_after_language_registration() {
        let lang = SharedString::from("json-cache-test");
        let theme = HighlightTheme::default_light();

        CODE_BLOCK_HIGHLIGHTERS.with(|cache| {
            cache.borrow_mut().remove(&lang);
        });

        let unknown_block = CodeBlock::new(
            "{\"value\": 1}".into(),
            Some(lang.clone()),
            &theme,
            None::<Span>,
        );
        _ = unknown_block.styles();

        let cached_language = CODE_BLOCK_HIGHLIGHTERS.with(|cache| {
            cache
                .borrow()
                .get(&lang)
                .map(|highlighter| highlighter.language().clone())
        });
        assert_eq!(cached_language.as_deref(), Some("text"));

        LanguageRegistry::singleton().register(
            lang.as_ref(),
            &crate::highlighter::LanguageConfig::new(
                lang.clone(),
                tree_sitter_json::LANGUAGE.into(),
                vec![],
                r#"
                    (string) @string
                    (number) @number
                    (pair key: (string) @property)
                "#,
                "",
                "",
            ),
        );

        let registered_block = CodeBlock::new(
            "{\"value\": 2}".into(),
            Some(lang.clone()),
            &theme,
            None::<Span>,
        );
        _ = registered_block.styles();

        let cached_language = CODE_BLOCK_HIGHLIGHTERS.with(|cache| {
            cache
                .borrow()
                .get(&lang)
                .map(|highlighter| highlighter.language().clone())
        });
        assert_eq!(cached_language.as_deref(), Some(lang.as_ref()));
    }

    #[test]
    fn image_markdown_source_preserves_outer_link() {
        let image = ImageNode {
            url: "https://example.com/badge.svg".into(),
            alt: Some("Build Status".into()),
            link: Some(LinkMark {
                url: "https://example.com/ci".into(),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(
            image.markdown_source(),
            "[![Build Status](https://example.com/badge.svg)](https://example.com/ci)"
        );
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_selected_text_preserves_text_before_inline_math() {
        let math = MathNode::try_new("x^2", false).unwrap();
        math.select_all_for_test();

        let mut paragraph = Paragraph::default();
        paragraph.push_str("before ");
        paragraph.push(InlineNode::math(math));
        paragraph.children[0]
            .state
            .lock()
            .unwrap()
            .set_text("before ".into());
        paragraph.children[0].state.lock().unwrap().selection = Some((0.."before ".len()).into());

        assert_eq!(paragraph.selected_text(), "before $x^2$");
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_selected_text_preserves_line_break_around_inline_math() {
        let math = MathNode::try_new("x^2", false).unwrap();
        math.select_all_for_test();

        let mut paragraph = Paragraph::default();
        paragraph.push_str("before");
        paragraph.push(InlineNode::math(math));
        paragraph.push(InlineNode::line_break());
        paragraph.push_str("after");

        paragraph.children[0]
            .state
            .lock()
            .unwrap()
            .set_text("before".into());
        paragraph.children[0].state.lock().unwrap().selection = Some((0.."before".len()).into());
        paragraph.children[1]
            .math
            .as_ref()
            .unwrap()
            .select_all_for_test();

        paragraph.children[2]
            .state
            .lock()
            .unwrap()
            .set_text("".into());

        paragraph.children[3]
            .state
            .lock()
            .unwrap()
            .set_text("after".into());
        paragraph.children[3].state.lock().unwrap().selection = Some((0.."after".len()).into());

        assert_eq!(paragraph.selected_text(), "before$x^2$\nafter");
    }

    #[test]
    fn test_selected_text_preserves_consecutive_line_breaks_and_trailing_breaks() {
        let mut paragraph = Paragraph::default();
        paragraph.push_str("before");
        paragraph.push(InlineNode::line_break());
        paragraph.push(InlineNode::line_break());
        paragraph.push_str("after");

        paragraph.children[0]
            .state
            .lock()
            .unwrap()
            .set_text("before".into());
        paragraph.children[0].state.lock().unwrap().selection = Some((0.."before".len()).into());

        paragraph.children[3]
            .state
            .lock()
            .unwrap()
            .set_text("after".into());
        paragraph.children[3].state.lock().unwrap().selection = Some((0.."after".len()).into());

        assert_eq!(paragraph.selected_text(), "before\n\nafter");

        let mut trailing = Paragraph::default();
        trailing.push_str("foo");
        trailing.push(InlineNode::line_break());
        trailing.children[0]
            .state
            .lock()
            .unwrap()
            .set_text("foo".into());
        trailing.children[0].state.lock().unwrap().selection = Some((0.."foo".len()).into());

        assert_eq!(trailing.selected_text(), "foo\n");

        let mut leading = Paragraph::default();
        leading.push(InlineNode::line_break());
        leading.push_str("a");
        leading.children[1]
            .state
            .lock()
            .unwrap()
            .set_text("a".into());
        leading.children[1].state.lock().unwrap().selection = Some((0.."a".len()).into());

        assert_eq!(leading.selected_text(), "\na");
    }

    #[test]
    fn test_selected_text_does_not_copy_break_before_unselected_content() {
        let mut paragraph = Paragraph::default();
        paragraph.push_str("a");
        paragraph.push(InlineNode::line_break());
        paragraph.push_str("b");

        paragraph.children[0]
            .state
            .lock()
            .unwrap()
            .set_text("a".into());
        paragraph.children[0].state.lock().unwrap().selection = Some((0..1).into());

        paragraph.children[2]
            .state
            .lock()
            .unwrap()
            .set_text("b".into());

        assert_eq!(paragraph.selected_text(), "a");
    }

    #[test]
    fn test_table_selected_text_skips_unselected_cells_and_rows() {
        let mut selected = Paragraph::default();
        selected.push_str("selected");
        selected.children[0]
            .state
            .lock()
            .unwrap()
            .set_text("selected".into());
        selected.children[0].state.lock().unwrap().selection = Some((0.."selected".len()).into());

        let table = BlockNode::Table(Table {
            children: vec![
                TableRow {
                    children: vec![
                        TableCell {
                            children: Paragraph::new("header a".into()),
                            ..Default::default()
                        },
                        TableCell {
                            children: Paragraph::new("header b".into()),
                            ..Default::default()
                        },
                    ],
                },
                TableRow {
                    children: vec![
                        TableCell {
                            children: Paragraph::new("left".into()),
                            ..Default::default()
                        },
                        TableCell {
                            children: selected,
                            ..Default::default()
                        },
                        TableCell {
                            children: Paragraph::new("right".into()),
                            ..Default::default()
                        },
                    ],
                },
                TableRow {
                    children: vec![TableCell {
                        children: Paragraph::new("footer".into()),
                        ..Default::default()
                    }],
                },
            ],
            ..Default::default()
        });

        assert_eq!(table.selected_text(), "selected\n\n");
    }

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_selected_text_preserves_text_around_inline_math() {
        let math = MathNode::try_new("x^2", false).unwrap();
        math.select_all_for_test();

        let mut paragraph = Paragraph::default();
        paragraph.push_str("text（");
        paragraph.push(InlineNode::math(math));
        paragraph.push_str("）after");

        paragraph.children[0]
            .state
            .lock()
            .unwrap()
            .set_text("text（".into());
        paragraph.children[0].state.lock().unwrap().selection = Some((0.."text（".len()).into());
        paragraph.children[1]
            .math
            .as_ref()
            .unwrap()
            .select_all_for_test();

        paragraph.children[2]
            .state
            .lock()
            .unwrap()
            .set_text("）after".into());
        paragraph.children[2].state.lock().unwrap().selection = Some((0.."）after".len()).into());

        assert_eq!(paragraph.selected_text(), "text（$x^2$）after");
    }
}
