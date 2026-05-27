use std::{
    collections::HashMap,
    ops::Range,
    sync::{Arc, Mutex},
};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, DefiniteLength, Div, Element, ElementId, FontStyle,
    FontWeight, GlobalElementId, Half, HighlightStyle, Hitbox, HitboxBehavior, InspectorElementId,
    InteractiveElement as _, IntoElement, LayoutId, Length, MouseUpEvent, ObjectFit, ParentElement,
    Pixels, Point, ShapedLine, SharedString, SharedUri, Size, StatefulInteractiveElement, Style,
    Styled, StyledImage as _, TextAlign, TextRun, Window, div, fill, img, point,
    prelude::FluentBuilder as _, px, relative, rems,
};
use markdown::mdast;
use ropey::Rope;

use crate::{
    ActiveTheme as _, Icon, IconName, StyledExt,
    global_state::GlobalState,
    h_flex,
    highlighter::{HighlightTheme, SyntaxHighlighter},
    text::{
        CodeBlockActionsFn,
        document::NodeRenderOptions,
        inline::{Inline, InlineState},
        math::MathNode,
    },
    tooltip::Tooltip,
    v_flex,
};

use super::{TextViewStyle, utils::list_item_prefix};

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
    /// Use for to_markdown get raw definition
    Definition {
        identifier: SharedString,
        url: SharedString,
        title: Option<SharedString>,
        span: Option<Span>,
    },
    Unknown,
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
    pub(super) fn span(&self) -> Option<Span> {
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

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();
        match self {
            BlockNode::Root { children, .. } => {
                let mut block_text = String::new();
                for c in children.iter() {
                    block_text.push_str(&c.selected_text());
                }
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            BlockNode::Paragraph(paragraph) => {
                let mut block_text = String::new();
                block_text.push_str(&paragraph.selected_text());
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            BlockNode::Heading { children, .. } => {
                let mut block_text = String::new();
                block_text.push_str(&children.selected_text());
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            BlockNode::List { children, .. } => {
                for c in children.iter() {
                    text.push_str(&c.selected_text());
                }
            }
            BlockNode::ListItem { children, .. } => {
                for c in children.iter() {
                    text.push_str(&c.selected_text());
                }
            }
            BlockNode::Blockquote { children, .. } => {
                let mut block_text = String::new();
                for c in children.iter() {
                    block_text.push_str(&c.selected_text());
                }

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
                        row_texts.push(cell.children.selected_text());
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
                let block_text = code_block.selected_text();
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            BlockNode::Math(math) => {
                let block_text = math.selected_text();
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

    state: Arc<Mutex<InlineState>>,
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

    pub(crate) state: Arc<Mutex<InlineState>>,
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
            state: Arc::new(Mutex::new(InlineState::default())),
        };
        paragraph.push_str(&text);
        paragraph
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();
        let mut pending_line_break = false;

        for c in self.children.iter() {
            let mut selected = String::new();

            let state = c.state.lock().unwrap();
            if let Some(selection) = &state.selection {
                let part_text = state.text.clone();
                selected.push_str(&part_text[selection.start..selection.end]);
            }
            drop(state);

            if let Some(math) = &c.math {
                selected.push_str(&math.selected_text());
            }

            if !selected.is_empty() {
                if pending_line_break {
                    text.push('\n');
                    pending_line_break = false;
                }
                text.push_str(&selected);
            }

            if c.line_break && !text.is_empty() {
                pending_line_break = true;
            }
        }

        let state = self.state.lock().unwrap();
        if let Some(selection) = &state.selection {
            let all_text = state.text.clone();
            if pending_line_break {
                text.push('\n');
            }
            text.push_str(&all_text[selection.start..selection.end]);
        }

        text
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Table {
    pub(crate) children: Vec<TableRow>,
    pub(crate) column_aligns: Vec<ColumnumnAlign>,
    pub(crate) span: Option<Span>,
}

impl Table {
    pub(crate) fn column_align(&self, index: usize) -> ColumnumnAlign {
        self.column_aligns.get(index).copied().unwrap_or_default()
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub(crate) enum ColumnumnAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl From<mdast::AlignKind> for ColumnumnAlign {
    fn from(value: mdast::AlignKind) -> Self {
        match value {
            mdast::AlignKind::None => ColumnumnAlign::Left,
            mdast::AlignKind::Left => ColumnumnAlign::Left,
            mdast::AlignKind::Center => ColumnumnAlign::Center,
            mdast::AlignKind::Right => ColumnumnAlign::Right,
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
                state: Arc::new(Mutex::new(InlineState::default())),
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
    styles: Vec<(Range<usize>, HighlightStyle)>,
    state: Arc<Mutex<InlineState>>,
    pub span: Option<Span>,
}

impl PartialEq for CodeBlock {
    fn eq(&self, other: &Self) -> bool {
        self.lang == other.lang && self.styles == other.styles
    }
}

impl CodeBlock {
    /// Get the language of the code block.
    pub fn lang(&self) -> Option<SharedString> {
        self.lang.clone()
    }

    /// Get the code content of the code block.
    pub fn code(&self) -> SharedString {
        self.state.lock().unwrap().text.clone()
    }

    pub(crate) fn new(
        code: SharedString,
        lang: Option<SharedString>,
        highlight_theme: &HighlightTheme,
        span: Option<impl Into<Span>>,
    ) -> Self {
        let mut styles = vec![];
        if let Some(lang) = &lang {
            let mut highlighter = SyntaxHighlighter::new(&lang);
            highlighter.update(None, &Rope::from_str(code.as_str()), None);
            styles = highlighter.styles(&(0..code.len()), highlight_theme);
        };

        let state = Arc::new(Mutex::new(InlineState::default()));
        state.lock().unwrap().set_text(code);

        Self {
            lang,
            styles,
            state,
            span: span.map(|s| s.into()),
        }
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();
        let state = self.state.lock().unwrap();
        if let Some(selection) = &state.selection {
            let part_text = state.text.clone();
            text.push_str(&part_text[selection.start..selection.end]);
        }
        text
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
                    .child(Inline::new(
                        "code",
                        self.state.clone(),
                        vec![],
                        self.styles.clone(),
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

fn resolved_link_mark(style: &TextMark, node_cx: &NodeContext) -> Option<LinkMark> {
    let mut link = style.link.clone()?;
    if let Some(identifier) = link.identifier.as_ref()
        && let Some(mark) = node_cx.link_refs.get(identifier)
    {
        link = mark.clone();
    }
    Some(link)
}

fn inline_node_link(node: &InlineNode, node_cx: &NodeContext) -> Option<LinkMark> {
    node.marks
        .iter()
        .find_map(|(_, style)| resolved_link_mark(style, node_cx))
}

fn text_view_has_selection(cx: &mut App) -> bool {
    let Some(text_view_state) = GlobalState::global(cx).text_view_state() else {
        return false;
    };

    text_view_state.read(cx).has_selection()
}

fn text_view_is_selectable(cx: &mut App) -> bool {
    let Some(text_view_state) = GlobalState::global(cx).text_view_state() else {
        return false;
    };

    text_view_state.read(cx).is_selectable()
}

fn inline_node_styles_for_range(
    node: &InlineNode,
    text_range: Range<usize>,
    offset: usize,
    node_cx: &NodeContext,
    cx: &mut App,
) -> (
    Vec<(Range<usize>, HighlightStyle)>,
    Vec<(Range<usize>, LinkMark)>,
) {
    let mut highlights = vec![];
    let mut links = vec![];

    for (range, style) in &node.marks {
        let start = range.start.max(text_range.start);
        let end = range.end.min(text_range.end);
        if start >= end {
            continue;
        }

        let inner_range = (offset + start - text_range.start)..(offset + end - text_range.start);

        let mut highlight = HighlightStyle::default();
        if style.bold {
            highlight.font_weight = Some(FontWeight::BOLD);
        }
        if style.italic {
            highlight.font_style = Some(FontStyle::Italic);
        }
        if style.strikethrough {
            highlight.strikethrough = Some(gpui::StrikethroughStyle {
                thickness: gpui::px(1.),
                ..Default::default()
            });
        }
        if style.underline {
            highlight.underline = Some(gpui::UnderlineStyle {
                thickness: gpui::px(1.),
                ..Default::default()
            });
        }
        if style.code {
            highlight.background_color = Some(cx.theme().accent);
        }

        if let Some(link_mark) = resolved_link_mark(style, node_cx) {
            highlight.color = Some(cx.theme().link);
            highlight.underline = Some(gpui::UnderlineStyle {
                thickness: gpui::px(1.),
                ..Default::default()
            });
            links.push((inner_range.clone(), link_mark));
        }

        highlights.push((inner_range, highlight));
    }

    (highlights, links)
}

#[derive(Clone)]
struct ParagraphInlineLayout {
    id: ElementId,
    children: Vec<InlineNode>,
    node_cx: NodeContext,
    options: NodeRenderOptions,
}

#[derive(Clone)]
enum ParagraphInlineItem {
    Text(ParagraphInlineText),
    Image(ParagraphInlineImage),
    Math(ParagraphInlineMath),
    Break,
}

#[derive(Clone)]
struct ParagraphInlineText {
    text: SharedString,
    state: Arc<Mutex<InlineState>>,
    links: Vec<(Range<usize>, LinkMark)>,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}

#[derive(Clone)]
struct ParagraphInlineMath {
    node: MathNode,
    link: Option<LinkMark>,
    display: bool,
}

#[derive(Clone)]
struct ParagraphInlineImage {
    id: usize,
    node: ImageNode,
    size: Size<Pixels>,
}

struct ParagraphInlineLayoutState;

struct ParagraphInlinePrepaint {
    layout: ParagraphInlineComputed,
    hitbox: Hitbox,
}

#[derive(Default)]
struct ParagraphInlineComputed {
    lines: Vec<ParagraphInlineLine>,
    size: Size<Pixels>,
}

struct ParagraphInlineLine {
    y: Pixels,
    ascent: Pixels,
    descent: Pixels,
    width: Pixels,
    align: ParagraphLineAlign,
    items: Vec<ParagraphInlineLayoutItem>,
}

#[derive(Clone, Copy)]
enum ParagraphLineAlign {
    Column,
    Center,
}

enum ParagraphInlineLayoutItem {
    Text(LaidOutInlineText),
    Image(LaidOutInlineImage),
    Math(LaidOutInlineMath),
}

struct LaidOutInlineText {
    x: Pixels,
    y: Pixels,
    height: Pixels,
    text: SharedString,
    source_text: SharedString,
    source_range: Range<usize>,
    state: Arc<Mutex<InlineState>>,
    line: ShapedLine,
    links: Vec<(Range<usize>, LinkMark)>,
}

struct LaidOutInlineMath {
    x: Pixels,
    y: Pixels,
    size: Size<Pixels>,
    node: MathNode,
    link: Option<LinkMark>,
}

struct LaidOutInlineImage {
    x: Pixels,
    y: Pixels,
    size: Size<Pixels>,
    element: AnyElement,
    link: Option<LinkMark>,
}

#[derive(Clone, Copy)]
struct InlineItemMetrics {
    ascent: Pixels,
    descent: Pixels,
}

const LIST_INLINE_MATH_TOP_PADDING: Pixels = px(0.);
const LIST_INLINE_MATH_MARKER_TOP_MARGIN: Pixels = px(5.);

fn inline_nodes_are_math_only(children: &[InlineNode]) -> bool {
    children.iter().any(|node| node.math.is_some())
        && children.iter().all(|node| {
            if node.math.is_some() {
                node.image.is_none() && !node.line_break
            } else {
                node.text.trim().is_empty() && node.image.is_none() && !node.line_break
            }
        })
}

fn list_item_prefix_element(
    ix: usize,
    ordered: bool,
    depth: usize,
    top_margin: Pixels,
) -> AnyElement {
    let prefix = list_item_prefix(ix, ordered, depth);
    if top_margin > px(0.) {
        div().mt(top_margin).child(prefix).into_any_element()
    } else {
        prefix.into_any_element()
    }
}

fn list_marker_top_margin(children: &[InlineNode], window: &Window) -> Pixels {
    if children.iter().any(|node| node.image.is_some())
        || !children.iter().any(|node| node.math.is_some())
    {
        return px(0.);
    }

    let text_ascent = default_text_line_metrics(window).ascent;
    let mut line_ascent = text_ascent;
    let mut has_first_line_math = false;
    for node in children {
        if node.line_break {
            break;
        }

        if let Some(math) = &node.math {
            has_first_line_math = true;
            line_ascent = line_ascent.max(math.layout_metrics(window).ascent);
            if math.is_display() {
                break;
            }
        }
    }
    if !has_first_line_math {
        return px(0.);
    }

    let top_padding = if inline_nodes_are_math_only(children) {
        LIST_INLINE_MATH_TOP_PADDING
    } else {
        px(0.)
    };

    (top_padding + LIST_INLINE_MATH_MARKER_TOP_MARGIN + line_ascent - text_ascent).max(px(0.))
}

impl ParagraphInlineLayout {
    fn new(
        id: ElementId,
        children: Vec<InlineNode>,
        node_cx: NodeContext,
        options: NodeRenderOptions,
    ) -> Self {
        Self {
            id,
            children,
            node_cx,
            options,
        }
    }

    fn items(
        &self,
        image_max_width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<ParagraphInlineItem> {
        let mut items = Vec::with_capacity(self.children.len());

        for (ix, node) in self.children.iter().enumerate() {
            if node.line_break {
                items.push(ParagraphInlineItem::Break);
                continue;
            }

            if let Some(image) = &node.image {
                items.push(ParagraphInlineItem::Image(ParagraphInlineImage {
                    id: ix,
                    node: image.clone(),
                    size: measure_inline_image(image, image_max_width, window, cx),
                }));
                continue;
            }

            if let Some(math) = &node.math {
                items.push(ParagraphInlineItem::Math(ParagraphInlineMath {
                    node: math.clone(),
                    link: inline_node_link(node, &self.node_cx),
                    display: math.is_display(),
                }));
                continue;
            }

            if node.text.is_empty() {
                continue;
            }

            let (highlights, links) =
                inline_node_styles_for_range(node, 0..node.text.len(), 0, &self.node_cx, cx);
            let highlights = gpui::combine_highlights(Vec::new(), highlights).collect();
            items.push(ParagraphInlineItem::Text(ParagraphInlineText {
                text: node.text.clone(),
                state: node.state.clone(),
                links,
                highlights,
            }));
        }

        items
    }

    fn top_padding(&self) -> Pixels {
        if self.options.in_list && inline_nodes_are_math_only(&self.children) {
            LIST_INLINE_MATH_TOP_PADDING
        } else {
            px(0.)
        }
    }
}

impl IntoElement for ParagraphInlineLayout {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ParagraphInlineLayout {
    type RequestLayoutState = ParagraphInlineLayoutState;
    type PrepaintState = ParagraphInlinePrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();

        let top_padding = self.top_padding();
        let items = self.items(None, window, cx);

        let layout_id =
            window.request_measured_layout(style, move |known, available, window, cx| {
                let width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
                });

                compute_paragraph_inline_layout(&items, width, top_padding, window, cx).size
            });

        (layout_id, ParagraphInlineLayoutState)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let items = self.items(Some(bounds.size.width), window, cx);
        let layout = compute_paragraph_inline_layout(
            &items,
            Some(bounds.size.width),
            self.top_padding(),
            window,
            cx,
        );
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let mut layout = layout;
        prepaint_paragraph_inline_images(&mut layout, bounds, self.options, window, cx);

        ParagraphInlinePrepaint { layout, hitbox }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        paint_paragraph_inline_layout(
            &mut prepaint.layout,
            bounds,
            &prepaint.hitbox,
            self.options,
            window,
            cx,
        );
    }
}

fn compute_paragraph_inline_layout(
    items: &[ParagraphInlineItem],
    width: Option<Pixels>,
    top_padding: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> ParagraphInlineComputed {
    let line_height = window.line_height();
    let default_metrics = default_text_line_metrics(window);
    let wrap_width = width.unwrap_or(Pixels::MAX / 2.);
    let mut computed = ParagraphInlineComputed::default();
    if items.is_empty() {
        if let Some(width) = width {
            computed.size.width = width;
        }
        return computed;
    }

    let mut line = ParagraphInlineLine {
        y: top_padding,
        ascent: default_metrics.ascent,
        descent: default_metrics.descent,
        width: px(0.),
        align: ParagraphLineAlign::Column,
        items: vec![],
    };

    let finish_line = |computed: &mut ParagraphInlineComputed, line: &mut ParagraphInlineLine| {
        computed.size.width = computed.size.width.max(line.width);
        let line_height = paragraph_inline_line_height(line);
        computed.size.height = line.y + line_height;
        let next_y = line.y + line_height;
        let next_align = ParagraphLineAlign::Column;
        computed.lines.push(std::mem::replace(
            line,
            ParagraphInlineLine {
                y: next_y,
                ascent: default_metrics.ascent,
                descent: default_metrics.descent,
                width: px(0.),
                align: next_align,
                items: vec![],
            },
        ));
    };

    for item in items {
        match item {
            ParagraphInlineItem::Break => {
                if !line.items.is_empty() {
                    finish_line(&mut computed, &mut line);
                }
            }
            ParagraphInlineItem::Image(image) => {
                let (element, size, metrics) =
                    layout_inline_image(image, wrap_width, default_metrics);
                if !line.items.is_empty() && line.width + size.width > wrap_width {
                    finish_line(&mut computed, &mut line);
                }

                update_line_metrics(&mut line, metrics.ascent, metrics.descent);
                let y = line.ascent - metrics.ascent;
                line.items
                    .push(ParagraphInlineLayoutItem::Image(LaidOutInlineImage {
                        x: line.width,
                        y,
                        size,
                        element,
                        link: image.node.link.clone(),
                    }));
                line.width += size.width;
            }
            ParagraphInlineItem::Math(math) if math.display => {
                if !line.items.is_empty() {
                    finish_line(&mut computed, &mut line);
                }

                let metrics = math.node.layout_metrics(window);
                line.align = ParagraphLineAlign::Center;
                update_line_metrics(&mut line, metrics.ascent, metrics.descent);
                line.width = metrics.size.width;
                line.items
                    .push(ParagraphInlineLayoutItem::Math(LaidOutInlineMath {
                        x: px(0.),
                        y: line.ascent - metrics.ascent,
                        size: metrics.size,
                        node: math.node.clone(),
                        link: math.link.clone(),
                    }));
                finish_line(&mut computed, &mut line);
            }
            ParagraphInlineItem::Math(math) => {
                let metrics = math.node.layout_metrics(window);
                if !line.items.is_empty() && line.width + metrics.size.width > wrap_width {
                    finish_line(&mut computed, &mut line);
                }

                update_line_metrics(&mut line, metrics.ascent, metrics.descent);
                let y = line.ascent - metrics.ascent;
                line.items
                    .push(ParagraphInlineLayoutItem::Math(LaidOutInlineMath {
                        x: line.width,
                        y,
                        size: metrics.size,
                        node: math.node.clone(),
                        link: math.link.clone(),
                    }));
                line.width += metrics.size.width;
            }
            ParagraphInlineItem::Text(text) => {
                let mut start = 0;
                while start < text.text.len() {
                    let remaining = &text.text[start..];
                    let shaped = shape_inline_text(
                        SharedString::new(remaining),
                        &text.highlights,
                        start..text.text.len(),
                        window,
                        cx,
                    );
                    let remaining_width = wrap_width - line.width;

                    if shaped.width() <= remaining_width || width.is_none() {
                        push_laid_out_text(
                            &mut line,
                            text,
                            start..text.text.len(),
                            shaped,
                            line_height,
                        );
                        break;
                    }

                    let mut split = fitting_text_boundary(remaining, &shaped, remaining_width);
                    if split == 0 {
                        if !line.items.is_empty() {
                            finish_line(&mut computed, &mut line);
                            continue;
                        }

                        split = remaining
                            .char_indices()
                            .nth(1)
                            .map(|(ix, _)| ix)
                            .unwrap_or(remaining.len());
                    }

                    let end = start + split;
                    let shaped = shape_inline_text(
                        SharedString::new(&text.text[start..end]),
                        &text.highlights,
                        start..end,
                        window,
                        cx,
                    );
                    push_laid_out_text(&mut line, text, start..end, shaped, line_height);
                    finish_line(&mut computed, &mut line);
                    start = end;
                }
            }
        }
    }

    if !line.items.is_empty() || computed.lines.is_empty() {
        finish_line(&mut computed, &mut line);
    }

    if let Some(width) = width {
        computed.size.width = width;
    }
    computed
}

fn push_laid_out_text(
    line: &mut ParagraphInlineLine,
    text: &ParagraphInlineText,
    range: Range<usize>,
    shaped: ShapedLine,
    line_height: Pixels,
) {
    let fragment = SharedString::new(&text.text[range.clone()]);
    let width = shaped.width();
    let metrics = shaped_text_line_metrics(&shaped, line_height);
    update_line_metrics(line, metrics.ascent, metrics.descent);
    line.items
        .push(ParagraphInlineLayoutItem::Text(LaidOutInlineText {
            x: line.width,
            y: line.ascent - metrics.ascent,
            height: line_height,
            text: fragment,
            source_text: text.text.clone(),
            source_range: range,
            state: text.state.clone(),
            line: shaped,
            links: text.links.clone(),
        }));
    line.width += width;
}

fn inline_image_element(image: &ImageNode, id: usize, size: Size<Pixels>) -> AnyElement {
    let title = image.title();
    img(image.url.clone())
        .id(("inline-image", id))
        .object_fit(ObjectFit::Contain)
        .w(size.width)
        .h(size.height)
        .when(image.link.is_some(), |this| {
            this.cursor_pointer()
                .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
        })
        .into_any_element()
}

fn inline_image_measure_element(image: &ImageNode) -> AnyElement {
    img(image.url.clone())
        .object_fit(ObjectFit::Contain)
        .max_w(relative(1.))
        .when_some(image.width, |this, width| this.w(width))
        .into_any_element()
}

fn measure_inline_image(
    image: &ImageNode,
    max_width: Option<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Size<Pixels> {
    let mut element = inline_image_measure_element(image);
    let width = max_width
        .map(AvailableSpace::Definite)
        .unwrap_or(AvailableSpace::MaxContent);

    element.layout_as_root(
        Size {
            width,
            height: AvailableSpace::MinContent,
        },
        window,
        cx,
    )
}

fn layout_inline_image(
    image: &ParagraphInlineImage,
    max_width: Pixels,
    default_metrics: InlineItemMetrics,
) -> (AnyElement, Size<Pixels>, InlineItemMetrics) {
    let size = constrain_inline_image_size(image.size, max_width, default_metrics);
    let element = inline_image_element(&image.node, image.id, size);
    let metrics = inline_image_metrics(size, default_metrics);

    (element, size, metrics)
}

fn constrain_inline_image_size(
    mut image_size: Size<Pixels>,
    max_width: Pixels,
    default_metrics: InlineItemMetrics,
) -> Size<Pixels> {
    let line_height = default_metrics.ascent + default_metrics.descent;
    if image_size.height <= px(0.) {
        image_size.height = line_height;
    }
    if image_size.width <= px(0.) {
        image_size.width = image_size.height;
    }

    if max_width > px(0.) && image_size.width > max_width {
        let scale = max_width / image_size.width;
        image_size.width = max_width;
        image_size.height *= scale;
    }

    image_size
}

fn inline_image_metrics(
    size: Size<Pixels>,
    default_metrics: InlineItemMetrics,
) -> InlineItemMetrics {
    let line_height = default_metrics.ascent + default_metrics.descent;
    let top = (line_height - size.height) / 2.;
    let ascent = default_metrics.ascent - top;

    InlineItemMetrics {
        ascent,
        descent: size.height - ascent,
    }
}

fn update_line_metrics(line: &mut ParagraphInlineLine, ascent: Pixels, descent: Pixels) {
    let new_ascent = line.ascent.max(ascent);
    let baseline_shift = new_ascent - line.ascent;
    if baseline_shift == px(0.) && descent <= line.descent {
        return;
    }

    for item in &mut line.items {
        match item {
            ParagraphInlineLayoutItem::Text(text) => text.y += baseline_shift,
            ParagraphInlineLayoutItem::Image(image) => image.y += baseline_shift,
            ParagraphInlineLayoutItem::Math(math) => math.y += baseline_shift,
        }
    }
    line.ascent = new_ascent;
    line.descent = line.descent.max(descent);
}

fn paragraph_inline_line_height(line: &ParagraphInlineLine) -> Pixels {
    line.ascent + line.descent
}

fn default_text_line_metrics(window: &Window) -> InlineItemMetrics {
    let text_style = window.text_style();
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let font_id = window.text_system().resolve_font(&text_style.font());
    let line_height = window.line_height();
    let ascent = window
        .text_system()
        .baseline_offset(font_id, font_size, line_height);

    InlineItemMetrics {
        ascent,
        descent: line_height - ascent,
    }
}

fn shaped_text_line_metrics(shaped: &ShapedLine, line_height: Pixels) -> InlineItemMetrics {
    let padding_top = px(0.);
    let ascent = padding_top + shaped.ascent;

    InlineItemMetrics {
        ascent,
        descent: line_height - ascent,
    }
}

fn shape_inline_text(
    text: SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    source_range: Range<usize>,
    window: &mut Window,
    _cx: &mut App,
) -> ShapedLine {
    let text_style = window.text_style();
    let mut runs: Vec<TextRun> = Vec::new();
    let mut ix = source_range.start;
    for (range, highlight) in highlights {
        let start = range.start.max(source_range.start);
        let end = range.end.min(source_range.end);
        if start >= end {
            continue;
        }

        if ix < start {
            runs.push(text_style.clone().to_run(start - ix));
        }
        runs.push(text_style.clone().highlight(*highlight).to_run(end - start));
        ix = end;
    }
    if ix < source_range.end {
        runs.push(text_style.to_run(source_range.end - ix));
    }

    let runs = crate::text::utils::normalize_runs_for_text(text.as_ref(), runs);
    let font_size = window.text_style().font_size.to_pixels(window.rem_size());
    window
        .text_system()
        .shape_line(text, font_size, &runs, None)
}

fn fitting_text_boundary(text: &str, shaped: &ShapedLine, width: Pixels) -> usize {
    if width <= px(0.) {
        return 0;
    }

    let mut prev_ch = '\0';
    let mut first_non_whitespace = false;
    let mut last_candidate = 0;
    let mut last_fit = 0;

    for (ix, ch) in text.char_indices() {
        if is_inline_word_char(ch) {
            if prev_ch == ' ' && ch != ' ' && first_non_whitespace {
                last_candidate = ix;
            }
        } else if ch != ' ' && first_non_whitespace {
            last_candidate = ix;
        }

        if ch != ' ' {
            first_non_whitespace = true;
        }

        let next_ix = ix + ch.len_utf8();
        if shaped.x_for_index(next_ix) > width {
            return if last_candidate > 0 {
                last_candidate
            } else {
                last_fit
            };
        }

        last_fit = next_ix;
        prev_ch = ch;
    }

    text.len()
}

fn is_inline_word_char(c: char) -> bool {
    matches!(c, '_')
        || c.is_ascii_alphanumeric()
        || matches!(c, '\u{00C0}'..='\u{00FF}')
        || matches!(c, '\u{0100}'..='\u{017F}')
        || matches!(c, '\u{0180}'..='\u{024F}')
        || matches!(c, '\u{0400}'..='\u{04FF}')
        || matches!(c, '\u{1E00}'..='\u{1EFF}')
        || matches!(c, '\u{0300}'..='\u{036F}')
}

fn prepaint_paragraph_inline_images(
    layout: &mut ParagraphInlineComputed,
    bounds: Bounds<Pixels>,
    options: NodeRenderOptions,
    window: &mut Window,
    cx: &mut App,
) {
    for line in &mut layout.lines {
        let line_offset = paragraph_line_offset(line, bounds.size.width, options);
        for item in &mut line.items {
            let ParagraphInlineLayoutItem::Image(image) = item else {
                continue;
            };

            let origin = bounds.origin + point(line_offset + image.x, line.y + image.y);
            image.element.prepaint_as_root(
                origin,
                Size {
                    width: AvailableSpace::Definite(image.size.width),
                    height: AvailableSpace::Definite(image.size.height),
                },
                window,
                cx,
            );
        }
    }
}

fn paint_paragraph_inline_layout(
    layout: &mut ParagraphInlineComputed,
    bounds: Bounds<Pixels>,
    hitbox: &Hitbox,
    options: NodeRenderOptions,
    window: &mut Window,
    cx: &mut App,
) {
    let has_selection = text_view_has_selection(cx);
    let mut hovered_link = None;
    let mut link_bounds = Vec::new();
    let mouse = window.mouse_position();

    let mut text_states: Vec<(Arc<Mutex<InlineState>>, SharedString, Option<Range<usize>>)> =
        Vec::new();

    for line in &mut layout.lines {
        let line_offset = paragraph_line_offset(line, bounds.size.width, options);
        for item in &mut line.items {
            match item {
                ParagraphInlineLayoutItem::Text(text) => {
                    record_text_state(&mut text_states, &text.state, text.source_text.clone());
                    let origin = bounds.origin + point(line_offset + text.x, line.y + text.y);
                    let selected =
                        selected_ranges_for_text(text, origin, window, cx, &mut text_states);
                    for range in selected {
                        let start = range.start - text.source_range.start;
                        let end = range.end - text.source_range.start;
                        let left = text.line.x_for_index(start);
                        let right = text.line.x_for_index(end);
                        window.paint_quad(fill(
                            Bounds::from_corners(
                                origin + point(left, px(0.)),
                                origin + point(right, text.height),
                            ),
                            cx.theme().selection,
                        ));
                    }

                    let _ = text
                        .line
                        .paint(origin, text.height, TextAlign::Left, None, window, cx);

                    for (range, link) in &text.links {
                        let start = range.start.max(text.source_range.start);
                        let end = range.end.min(text.source_range.end);
                        if start >= end {
                            continue;
                        }

                        let left = text.line.x_for_index(start - text.source_range.start);
                        let right = text.line.x_for_index(end - text.source_range.start);
                        let bounds = Bounds::from_corners(
                            origin + point(left, px(0.)),
                            origin + point(right, text.height),
                        );
                        if bounds.contains(&mouse) {
                            hovered_link = Some(link.clone());
                        }
                        link_bounds.push((bounds, link.clone()));
                    }
                }
                ParagraphInlineLayoutItem::Image(image) => {
                    let image_bounds = Bounds {
                        origin: bounds.origin + point(line_offset + image.x, line.y + image.y),
                        size: image.size,
                    };
                    image.element.paint(window, cx);
                    if let Some(link) = &image.link
                        && image_bounds.contains(&mouse)
                    {
                        hovered_link = Some(link.clone());
                    }
                    if let Some(link) = &image.link {
                        link_bounds.push((image_bounds, link.clone()));
                    }
                }
                ParagraphInlineLayoutItem::Math(math) => {
                    let origin = bounds.origin + point(line_offset + math.x, line.y + math.y);
                    let math_bounds = Bounds {
                        origin,
                        size: math.size,
                    };
                    let text_color = math.link.as_ref().map(|_| cx.theme().link);
                    math.node.paint_at(math_bounds, text_color, window, cx);
                    if let Some(link) = &math.link
                        && math_bounds.contains(&mouse)
                    {
                        hovered_link = Some(link.clone());
                    }
                    if let Some(link) = &math.link {
                        link_bounds.push((math_bounds, link.clone()));
                    }
                }
            }
        }
    }

    for (state, text, selection) in text_states {
        let mut state = state.lock().unwrap();
        state.text = text;
        state.selection = selection.map(Into::into);
    }

    if text_view_is_selectable(cx) {
        window.set_cursor_style(gpui::CursorStyle::IBeam, hitbox);
    }

    if hovered_link.is_some() {
        window.set_cursor_style(gpui::CursorStyle::PointingHand, hitbox);
    }

    if !has_selection && !link_bounds.is_empty() {
        let hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if !phase.bubble() || !hitbox.is_hovered(window) {
                return;
            }

            let Some(text_view_state) = GlobalState::global(cx).text_view_state() else {
                return;
            };
            if text_view_state.read(cx).has_selection() {
                return;
            }

            if event.button == gpui::MouseButton::Left
                && let Some((_, link)) = link_bounds
                    .iter()
                    .find(|(bounds, _)| bounds.contains(&event.position))
            {
                cx.stop_propagation();
                cx.open_url(&link.url);
            }
        });
    }
}

fn paragraph_line_offset(
    line: &ParagraphInlineLine,
    width: Pixels,
    options: NodeRenderOptions,
) -> Pixels {
    let align = match line.align {
        ParagraphLineAlign::Center => ColumnumnAlign::Center,
        ParagraphLineAlign::Column => options.column_align,
    };
    match align {
        ColumnumnAlign::Left => px(0.),
        ColumnumnAlign::Center => (width - line.width).max(px(0.)) / 2.,
        ColumnumnAlign::Right => (width - line.width).max(px(0.)),
    }
}

fn record_text_state(
    states: &mut Vec<(Arc<Mutex<InlineState>>, SharedString, Option<Range<usize>>)>,
    state: &Arc<Mutex<InlineState>>,
    text: SharedString,
) {
    if states
        .iter()
        .any(|(existing, _, _)| Arc::ptr_eq(existing, state))
    {
        return;
    }

    states.push((state.clone(), text, None));
}

fn selected_ranges_for_text(
    text: &LaidOutInlineText,
    origin: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
    states: &mut Vec<(Arc<Mutex<InlineState>>, SharedString, Option<Range<usize>>)>,
) -> Vec<Range<usize>> {
    let Some(text_view_state) = GlobalState::global(cx).text_view_state() else {
        return vec![];
    };
    let text_view_state = text_view_state.read(cx);
    if !text_view_state.has_selection() || !text_view_state.is_selectable() {
        return vec![];
    }
    let Some((selection_start, selection_end)) = text_view_state.selection_points() else {
        return vec![];
    };

    let mut selected_ranges = Vec::new();
    let mut selected_start = None;
    let mut selected_end = None;
    let mut local_offset = 0;
    for ch in text.text.chars() {
        let next_offset = local_offset + ch.len_utf8();
        let left = text.line.x_for_index(local_offset);
        let right = text.line.x_for_index(next_offset);
        let char_width = (right - left).max(window.line_height().half());
        let char_origin = origin + point(left, px(0.));
        if point_in_inline_selection(
            char_origin,
            char_width,
            selection_start,
            selection_end,
            text.height,
        ) {
            let start = text.source_range.start + local_offset;
            let end = text.source_range.start + next_offset;
            selected_start.get_or_insert(start);
            selected_end = Some(end);
        } else if let (Some(start), Some(end)) = (selected_start.take(), selected_end.take()) {
            selected_ranges.push(start..end);
        }

        local_offset = next_offset;
    }

    if let (Some(start), Some(end)) = (selected_start, selected_end) {
        selected_ranges.push(start..end);
    }

    for range in &selected_ranges {
        if let Some((_, _, selection)) = states
            .iter_mut()
            .find(|(state, _, _)| Arc::ptr_eq(state, &text.state))
        {
            match selection {
                Some(selection) => {
                    selection.start = selection.start.min(range.start);
                    selection.end = selection.end.max(range.end);
                }
                None => *selection = Some(range.clone()),
            }
        }
    }

    selected_ranges
}

fn point_in_inline_selection(
    pos: Point<Pixels>,
    char_width: Pixels,
    selection_start: Point<Pixels>,
    selection_end: Point<Pixels>,
    line_height: Pixels,
) -> bool {
    let point_in_line = |point: Point<Pixels>| point.y >= pos.y && point.y < pos.y + line_height;
    let top = selection_start.y.min(selection_end.y);
    let bottom = selection_start.y.max(selection_end.y);
    let x = pos.x + char_width.half();

    if pos.y + line_height <= top || pos.y > bottom {
        return false;
    }

    if point_in_line(selection_start) && point_in_line(selection_end) {
        let left = selection_start.x.min(selection_end.x);
        let right = selection_start.x.max(selection_end.x);
        return x >= left && x <= right;
    }

    let (top_point, bottom_point) = if selection_start.y < selection_end.y {
        (selection_start, selection_end)
    } else {
        (selection_end, selection_start)
    };
    let is_top_line = point_in_line(top_point);
    let is_bottom_line = point_in_line(bottom_point);

    if is_top_line {
        x >= top_point.x
    } else if is_bottom_line {
        x <= bottom_point.x
    } else {
        true
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
        cx: &mut App,
    ) -> AnyElement {
        if self.children.iter().any(|node| node.math.is_some()) {
            return ParagraphInlineLayout::new(
                self.span.map(Into::into).unwrap_or_else(|| {
                    ElementId::Name(format!("paragraph-inline-{}", options.ix).into())
                }),
                self.children.clone(),
                node_cx.clone(),
                options,
            )
            .into_any_element();
        }

        let span = self.span;
        let mut child_nodes: Vec<AnyElement> = vec![];

        let mut text = String::new();
        let mut highlights: Vec<(Range<usize>, HighlightStyle)> = vec![];
        let mut links: Vec<(Range<usize>, LinkMark)> = vec![];
        let mut offset = 0;

        let mut ix = 0usize;
        for inline_node in &self.children {
            let node_text = &inline_node.text;
            let text_len = node_text.len();
            text.push_str(node_text);

            if let Some(image) = &inline_node.image {
                if text.len() > 0 {
                    inline_node
                        .state
                        .lock()
                        .unwrap()
                        .set_text(text.clone().into());
                    child_nodes.push(
                        Inline::new(
                            ix,
                            inline_node.state.clone(),
                            links.clone(),
                            highlights.clone(),
                        )
                        .into_any_element(),
                    );
                }
                child_nodes.push(
                    img(image.url.clone())
                        .id(ix)
                        .object_fit(ObjectFit::Contain)
                        .max_w(relative(1.))
                        .when_some(image.width, |this, width| this.w(width))
                        .when_some(image.link.clone(), |this, link| {
                            let title = image.title();
                            this.cursor_pointer()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(title.clone()).build(window, cx)
                                })
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    cx.open_url(&link.url);
                                })
                        })
                        .into_any_element(),
                );

                text.clear();
                links.clear();
                highlights.clear();
                offset = 0;
            } else {
                let (node_highlights, node_links) = inline_node_styles_for_range(
                    inline_node,
                    0..inline_node.text.len(),
                    offset,
                    node_cx,
                    cx,
                );
                links.extend(node_links);
                highlights = gpui::combine_highlights(highlights, node_highlights).collect();
                offset += text_len;
            }
            ix += 1;
        }

        if text.len() > 0 {
            self.state.lock().unwrap().set_text(text.into());
            child_nodes
                .push(Inline::new(ix, self.state.clone(), links, highlights).into_any_element());
        }

        div()
            .id(span.unwrap_or_default())
            .children(child_nodes)
            .into_any_element()
    }
}

impl Paragraph {
    fn to_markdown(&self) -> String {
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
                        let alt = image.alt.clone().unwrap_or_default();
                        let title = image
                            .title
                            .clone()
                            .map_or(String::new(), |t| format!(" \"{}\"", t));
                        text.push_str(&format!("![{}]({}{})", alt, image.url, title))
                    }

                    (text, link, markdown_style)
                }
            };

            push_markdown_segment(&mut text, &mut pending, segment, link, style);
        }

        if let Some(pending) = pending {
            text.push_str(&pending.finish());
        }

        text.push_str("\n\n");
        text
    }
}

impl BlockNode {
    /// Converts the node to markdown format.
    ///
    /// This is used to generate markdown for test.
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
                    format!("{}{}", prefix, child.to_markdown())
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
                        .join("\n")
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
                            .map(|cell| cell.children.to_markdown())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .unwrap_or_default();
                let alignments = table
                    .column_aligns
                    .iter()
                    .map(|align| {
                        match align {
                            ColumnumnAlign::Left => ":--",
                            ColumnumnAlign::Center => ":-:",
                            ColumnumnAlign::Right => "--:",
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
                            .map(|cell| cell.children.to_markdown())
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

                    for (child_ix, child) in children.iter().enumerate() {
                        match child {
                            BlockNode::Paragraph(paragraph) => {
                                let last_not_list = child_ix > 0
                                    && !matches!(children[child_ix - 1], BlockNode::List { .. });
                                let marker_top_margin =
                                    list_marker_top_margin(&paragraph.children, window);

                                let text = child.render_block(
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        todo: checked.is_some(),
                                        in_list: true,
                                        is_last: true,
                                        ..options
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                );

                                // Continuation paragraph: stack vertically below
                                // the previous row, indented to align with the text
                                // column (past bullet/number prefix).
                                if last_not_list {
                                    if let Some(preceding_row) = items.pop() {
                                        items.push(
                                            v_flex().child(preceding_row).child(
                                                div()
                                                    .w_full()
                                                    .pl(rems(0.75))
                                                    .overflow_hidden()
                                                    .child(text),
                                            ),
                                        );
                                        continue;
                                    }
                                }

                                items.push(
                                    h_flex()
                                        .w_full()
                                        .flex_1()
                                        .min_w_0()
                                        .relative()
                                        .items_start()
                                        .content_start()
                                        .when(!options.todo && checked.is_none(), |this| {
                                            this.child(list_item_prefix_element(
                                                ix,
                                                options.ordered,
                                                options.depth,
                                                marker_top_margin,
                                            ))
                                        })
                                        .when_some(*checked, |this, checked| {
                                            // Todo list checkbox
                                            this.child(
                                                div()
                                                    .flex()
                                                    .mt(rems(0.4))
                                                    .mr_1p5()
                                                    .size(rems(0.875))
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded(cx.theme().radius.half())
                                                    .border_1()
                                                    .border_color(cx.theme().primary)
                                                    .text_color(cx.theme().primary_foreground)
                                                    .when(checked, |this| {
                                                        this.bg(cx.theme().primary).child(
                                                            Icon::new(IconName::Check)
                                                                .size_2()
                                                                .text_xs(),
                                                        )
                                                    }),
                                            )
                                        })
                                        .child(
                                            div().flex_1().min_w_0().overflow_hidden().child(text),
                                        ),
                                );
                            }
                            BlockNode::List { .. } => {
                                items.push(div().ml(rems(1.)).child(child.render_block(
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        todo: checked.is_some(),
                                        in_list: true,
                                        is_last: true,
                                        ..options
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                )));
                            }
                            BlockNode::Math(math) => {
                                let text_ascent = default_text_line_metrics(window).ascent;
                                let marker_top_margin = (LIST_INLINE_MATH_MARKER_TOP_MARGIN
                                    + math.layout_metrics(window).ascent
                                    - text_ascent)
                                    .max(px(0.));
                                let text = child.render_block(
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        todo: checked.is_some(),
                                        in_list: true,
                                        is_last: true,
                                        ..options
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                );

                                if child_ix == 0 {
                                    items.push(
                                        h_flex()
                                            .w_full()
                                            .flex_1()
                                            .min_w_0()
                                            .relative()
                                            .items_start()
                                            .content_start()
                                            .when(!options.todo && checked.is_none(), |this| {
                                                this.child(list_item_prefix_element(
                                                    ix,
                                                    options.ordered,
                                                    options.depth,
                                                    marker_top_margin,
                                                ))
                                            })
                                            .when_some(*checked, |this, checked| {
                                                this.child(
                                                    div()
                                                        .flex()
                                                        .mt(rems(0.4))
                                                        .mr_1p5()
                                                        .size(rems(0.875))
                                                        .items_center()
                                                        .justify_center()
                                                        .rounded(cx.theme().radius.half())
                                                        .border_1()
                                                        .border_color(cx.theme().primary)
                                                        .text_color(cx.theme().primary_foreground)
                                                        .when(checked, |this| {
                                                            this.bg(cx.theme().primary).child(
                                                                Icon::new(IconName::Check)
                                                                    .size_2()
                                                                    .text_xs(),
                                                            )
                                                        }),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .child(text),
                                            ),
                                    );
                                } else {
                                    items.push(
                                        div().w_full().pl(rems(0.75)).overflow_hidden().child(text),
                                    );
                                }
                            }
                            _ => {}
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
                                                            align == ColumnumnAlign::Center,
                                                            |this| this.text_center(),
                                                        )
                                                        .when(
                                                            align == ColumnumnAlign::Right,
                                                            |this| this.text_right(),
                                                        )
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
                                c.render_block(options.is_last(is_last), node_cx, window, cx)
                            })
                        }),
                )
                .into_any_element(),
            BlockNode::List {
                children, ordered, ..
            } => v_flex()
                .id((if *ordered { "ol" } else { "ul" }, ix))
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

#[cfg(all(test, feature = "markdown-math"))]
mod tests {
    use super::*;

    #[cfg(feature = "markdown-math")]
    #[test]
    fn test_selected_text_preserves_text_before_inline_math() {
        let math = MathNode::try_new("x^2", false).unwrap();
        math.select_all_for_test();

        let mut paragraph = Paragraph::default();
        paragraph.push_str("before ");
        paragraph.push(InlineNode::math(math));

        let math_ix = paragraph
            .children
            .iter()
            .position(|child| child.math.is_some())
            .unwrap();
        let mut state = paragraph.children[math_ix].state.lock().unwrap();
        state.set_text("before ".into());
        state.selection = Some((0.."before ".len()).into());
        drop(state);

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

        paragraph.children[1]
            .state
            .lock()
            .unwrap()
            .set_text("before".into());
        paragraph.children[1].state.lock().unwrap().selection = Some((0.."before".len()).into());

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
            .set_text("text".into());
        paragraph.children[0].state.lock().unwrap().selection = Some((0.."text".len()).into());

        paragraph.children[1]
            .state
            .lock()
            .unwrap()
            .set_text("（".into());
        paragraph.children[1].state.lock().unwrap().selection = Some((0.."（".len()).into());

        paragraph.children[2]
            .state
            .lock()
            .unwrap()
            .set_text("）".into());
        paragraph.children[2].state.lock().unwrap().selection = Some((0.."）".len()).into());

        paragraph.state.lock().unwrap().set_text("after".into());
        paragraph.state.lock().unwrap().selection = Some((0.."after".len()).into());

        assert_eq!(paragraph.selected_text(), "text（$x^2$）after");
    }
}
