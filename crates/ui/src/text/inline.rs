use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, FontStyle, FontWeight,
    GlobalElementId, Half, HighlightStyle, Hitbox, HitboxBehavior, InspectorElementId,
    InteractiveElement as _, IntoElement, LayoutId, MouseMoveEvent, MouseUpEvent, ObjectFit,
    ParentElement, Pixels, Point, ShapedLine, SharedString, Size, StatefulInteractiveElement,
    Style, Styled, StyledImage as _, TextAlign, TextRun, TextStyle, Window, div, fill, img, point,
    prelude::FluentBuilder as _, px, relative, rems,
};
use unicode_linebreak::linebreaks;
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    ActiveTheme as _, Icon, IconName,
    global_state::GlobalState,
    input::Selection,
    text::{
        document::{ListItemPrefix, NodeRenderOptions},
        math::{MathMetrics, MathNode},
        node::{ColumnAlign, ImageNode, InlineNode, LinkMark, NodeContext, TextMark},
    },
    tooltip::Tooltip,
};

use super::utils::list_item_prefix;

/// The inline text state, used RefCell to keep the selection state.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct InlineState {
    /// The text that actually rendering, matched with selection.
    pub(super) text: SharedString,
    pub(super) selection: Option<Selection>,
}

impl InlineState {
    /// Save actually rendered text for selected text to use.
    pub(crate) fn set_text(&mut self, text: SharedString) {
        self.text = text;
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
pub(super) struct ParagraphInlineLayout {
    id: ElementId,
    children: Vec<InlineNode>,
    items: Option<Vec<ParagraphInlineItem>>,
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
    source_text: SharedString,
    source_offset: usize,
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

pub(super) struct ParagraphInlineLayoutState;

pub(super) struct ParagraphInlinePrepaint {
    layout: ParagraphInlineComputed,
    hitbox: Hitbox,
}

#[derive(Default)]
struct ParagraphInlineComputed {
    prefix: Option<ParagraphInlinePrefix>,
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

struct ParagraphInlineFlowItem<'a> {
    logical_end: usize,
    width: Pixels,
    kind: ParagraphInlineFlowItemKind<'a>,
}

enum ParagraphInlineFlowItemKind<'a> {
    Text {
        text: &'a ParagraphInlineText,
        range: Range<usize>,
    },
    Image {
        image: &'a ParagraphInlineImage,
        size: Size<Pixels>,
        metrics: InlineItemMetrics,
    },
    Math {
        math: &'a ParagraphInlineMath,
        metrics: MathMetrics,
    },
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

enum ParagraphInlinePrefix {
    Marker {
        y: Pixels,
        width: Pixels,
        height: Pixels,
        line: Option<ShapedLine>,
    },
    Todo {
        y: Pixels,
        width: Pixels,
        size: Pixels,
        element: Option<AnyElement>,
    },
}

impl ParagraphInlinePrefix {
    fn width(&self) -> Pixels {
        match self {
            Self::Marker { width, .. } | Self::Todo { width, .. } => *width,
        }
    }
}

const TODO_CHECKBOX_SIZE_REM: f32 = 0.875;
const TODO_CHECKBOX_GAP_REM: f32 = 0.375;
const INLINE_OBJECT_REPLACEMENT: char = '\u{fffc}';

fn paragraph_inline_prefix(
    prefix: Option<ListItemPrefix>,
    text_style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) -> Option<ParagraphInlinePrefix> {
    match prefix {
        Some(ListItemPrefix::Marker {
            ix,
            ordered,
            depth,
            visible,
        }) => {
            let text = list_item_prefix(ix, ordered, depth);
            let len = text.len();
            let line = shape_inline_text(text.into(), &[], 0..len, text_style, window, cx);
            Some(ParagraphInlinePrefix::Marker {
                y: px(0.),
                width: line.width(),
                height: text_style.line_height_in_pixels(window.rem_size()),
                line: visible.then_some(line),
            })
        }
        Some(ListItemPrefix::Todo { checked, visible }) => {
            let size = rems(TODO_CHECKBOX_SIZE_REM).to_pixels(window.rem_size());
            let gap = rems(TODO_CHECKBOX_GAP_REM).to_pixels(window.rem_size());
            Some(ParagraphInlinePrefix::Todo {
                y: px(0.),
                width: size + gap,
                size,
                element: visible.then(|| todo_checkbox_element(checked, cx)),
            })
        }
        None => None,
    }
}

fn align_paragraph_inline_prefix(layout: &mut ParagraphInlineComputed) {
    let Some(line) = layout.lines.first() else {
        return;
    };
    let Some(prefix) = &mut layout.prefix else {
        return;
    };

    match prefix {
        ParagraphInlinePrefix::Marker {
            y,
            height,
            line: Some(marker),
            ..
        } => {
            let metrics = shaped_text_line_metrics(marker, *height);
            *y = line.y + line.ascent - metrics.ascent;
        }
        ParagraphInlinePrefix::Todo { y, size, .. } => {
            *y = line.y + line.ascent - *size * 0.9;
        }
        _ => {}
    }
}

fn todo_checkbox_element(checked: bool, cx: &mut App) -> AnyElement {
    let size = rems(TODO_CHECKBOX_SIZE_REM);

    div()
        .flex()
        .size(size)
        .items_center()
        .justify_center()
        .rounded(cx.theme().radius.half())
        .border_1()
        .border_color(cx.theme().primary)
        .text_color(cx.theme().primary_foreground)
        .when(checked, |this| {
            this.bg(cx.theme().primary)
                .child(Icon::new(IconName::Check).size_2().text_xs())
        })
        .into_any_element()
}

impl ParagraphInlineLayout {
    pub(super) fn new(
        id: ElementId,
        children: Vec<InlineNode>,
        node_cx: NodeContext,
        options: NodeRenderOptions,
    ) -> Self {
        Self {
            id,
            children,
            items: None,
            node_cx,
            options,
        }
    }

    pub(super) fn highlighted_text(
        id: ElementId,
        text: SharedString,
        state: Arc<Mutex<InlineState>>,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
        node_cx: NodeContext,
        options: NodeRenderOptions,
    ) -> Self {
        Self {
            id,
            children: vec![],
            items: Some(paragraph_inline_highlighted_text_items(
                text, state, highlights,
            )),
            node_cx,
            options,
        }
    }

    fn items(&self, window: &mut Window, cx: &mut App) -> Vec<ParagraphInlineItem> {
        if let Some(items) = &self.items {
            return items.clone();
        }

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
                    size: measure_inline_image(image, None, window, cx),
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
                source_text: node.text.clone(),
                source_offset: 0,
                state: node.state.clone(),
                links,
                highlights,
            }));
        }

        items
    }
}

fn paragraph_inline_highlighted_text_items(
    text: SharedString,
    state: Arc<Mutex<InlineState>>,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
) -> Vec<ParagraphInlineItem> {
    let mut items = Vec::new();
    let mut start = 0;

    for (ix, ch) in text.char_indices() {
        if ch != '\n' {
            continue;
        }

        if start < ix {
            items.push(ParagraphInlineItem::Text(paragraph_inline_text_segment(
                &text,
                start..ix,
                state.clone(),
                &highlights,
            )));
        }
        items.push(ParagraphInlineItem::Break);
        start = ix + ch.len_utf8();
    }

    if start < text.len() {
        items.push(ParagraphInlineItem::Text(paragraph_inline_text_segment(
            &text,
            start..text.len(),
            state,
            &highlights,
        )));
    }

    items
}

fn paragraph_inline_text_segment(
    source_text: &SharedString,
    range: Range<usize>,
    state: Arc<Mutex<InlineState>>,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> ParagraphInlineText {
    let text = SharedString::new(&source_text[range.clone()]);
    let highlights = highlights
        .iter()
        .filter_map(|(highlight_range, highlight)| {
            let start = highlight_range.start.max(range.start);
            let end = highlight_range.end.min(range.end);
            (start < end).then(|| ((start - range.start)..(end - range.start), *highlight))
        })
        .collect();

    ParagraphInlineText {
        text,
        source_text: source_text.clone(),
        source_offset: range.start,
        state,
        links: vec![],
        highlights,
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
        style.flex_grow = 1.;
        style.flex_shrink = 1.;
        style.size.width = relative(1.).into();
        style.min_size.width = px(0.).into();

        let list_prefix = self.options.list_prefix;
        let items = self.items(window, cx);
        let text_style = window.text_style();

        let layout_id =
            window.request_measured_layout(style, move |known, available, window, cx| {
                let width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
                });

                compute_paragraph_inline_layout(&items, width, list_prefix, &text_style, window, cx)
                    .size
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
        let items = self.items(window, cx);
        let text_style = window.text_style();
        let layout = compute_paragraph_inline_layout(
            &items,
            Some(bounds.size.width),
            self.options.list_prefix,
            &text_style,
            window,
            cx,
        );
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let mut layout = layout;
        prepaint_paragraph_inline_prefix(&mut layout, bounds, window, cx);
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
    prefix: Option<ListItemPrefix>,
    text_style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) -> ParagraphInlineComputed {
    let line_height = text_style.line_height_in_pixels(window.rem_size());
    let default_metrics = default_text_line_metrics(text_style, window);
    let prefix = paragraph_inline_prefix(prefix, text_style, window, cx);
    let prefix_width = prefix.as_ref().map_or(px(0.), ParagraphInlinePrefix::width);
    let wrap_width = width
        .map(|width| (width - prefix_width).max(px(0.)))
        .unwrap_or(Pixels::MAX / 2.);
    let mut computed = ParagraphInlineComputed::default();
    computed.prefix = prefix;
    if items.is_empty() && computed.prefix.is_none() {
        if let Some(width) = width {
            computed.size.width = width;
        }
        return computed;
    }

    let mut line = ParagraphInlineLine {
        y: px(0.),
        ascent: default_metrics.ascent,
        descent: default_metrics.descent,
        width: px(0.),
        align: ParagraphLineAlign::Column,
        items: vec![],
    };

    let mut flow_items: Vec<ParagraphInlineFlowItem<'_>> = Vec::new();
    let mut logical = String::new();

    for item in items {
        match item {
            ParagraphInlineItem::Break => {
                flush_paragraph_inline_flow_items(
                    &mut computed,
                    &mut line,
                    &mut flow_items,
                    &mut logical,
                    wrap_width,
                    line_height,
                    default_metrics,
                    prefix_width,
                    text_style,
                    window,
                    cx,
                );
                if !line.items.is_empty() || !computed.lines.is_empty() {
                    finish_paragraph_inline_line(
                        &mut computed,
                        &mut line,
                        prefix_width,
                        default_metrics,
                    );
                }
            }
            ParagraphInlineItem::Image(image) => {
                let (size, metrics) = layout_inline_image(image.size, wrap_width, default_metrics);
                logical.push(INLINE_OBJECT_REPLACEMENT);
                flow_items.push(ParagraphInlineFlowItem {
                    logical_end: logical.len(),
                    width: size.width,
                    kind: ParagraphInlineFlowItemKind::Image {
                        image,
                        size,
                        metrics,
                    },
                });
            }
            ParagraphInlineItem::Math(math) if math.display => {
                flush_paragraph_inline_flow_items(
                    &mut computed,
                    &mut line,
                    &mut flow_items,
                    &mut logical,
                    wrap_width,
                    line_height,
                    default_metrics,
                    prefix_width,
                    text_style,
                    window,
                    cx,
                );
                if !line.items.is_empty() {
                    finish_paragraph_inline_line(
                        &mut computed,
                        &mut line,
                        prefix_width,
                        default_metrics,
                    );
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
                finish_paragraph_inline_line(
                    &mut computed,
                    &mut line,
                    prefix_width,
                    default_metrics,
                );
            }
            ParagraphInlineItem::Math(math) => {
                let metrics = math.node.layout_metrics(window);
                logical.push(INLINE_OBJECT_REPLACEMENT);
                flow_items.push(ParagraphInlineFlowItem {
                    logical_end: logical.len(),
                    width: metrics.size.width,
                    kind: ParagraphInlineFlowItemKind::Math { math, metrics },
                });
            }
            ParagraphInlineItem::Text(text) => {
                if text.text.is_empty() {
                    continue;
                }

                let shaped = shape_inline_text(
                    text.text.clone(),
                    &text.highlights,
                    0..text.text.len(),
                    text_style,
                    window,
                    cx,
                );
                for (start, grapheme) in text.text.grapheme_indices(true) {
                    let end = start + grapheme.len();
                    let width = shaped.x_for_index(end) - shaped.x_for_index(start);
                    logical.push_str(grapheme);
                    flow_items.push(ParagraphInlineFlowItem {
                        logical_end: logical.len(),
                        width,
                        kind: ParagraphInlineFlowItemKind::Text {
                            text,
                            range: start..end,
                        },
                    });
                }
            }
        }
    }

    flush_paragraph_inline_flow_items(
        &mut computed,
        &mut line,
        &mut flow_items,
        &mut logical,
        wrap_width,
        line_height,
        default_metrics,
        prefix_width,
        text_style,
        window,
        cx,
    );
    if !line.items.is_empty() || computed.lines.is_empty() {
        finish_paragraph_inline_line(&mut computed, &mut line, prefix_width, default_metrics);
    }

    if let Some(width) = width {
        computed.size.width = width;
    }
    align_paragraph_inline_prefix(&mut computed);
    computed
}

fn finish_paragraph_inline_line(
    computed: &mut ParagraphInlineComputed,
    line: &mut ParagraphInlineLine,
    prefix_width: Pixels,
    default_metrics: InlineItemMetrics,
) {
    computed.size.width = computed.size.width.max(prefix_width + line.width);
    let line_height = paragraph_inline_line_height(line);
    computed.size.height = line.y + line_height;
    let next_y = line.y + line_height;
    computed.lines.push(std::mem::replace(
        line,
        ParagraphInlineLine {
            y: next_y,
            ascent: default_metrics.ascent,
            descent: default_metrics.descent,
            width: px(0.),
            align: ParagraphLineAlign::Column,
            items: vec![],
        },
    ));
}

fn flush_paragraph_inline_flow_items(
    computed: &mut ParagraphInlineComputed,
    line: &mut ParagraphInlineLine,
    flow_items: &mut Vec<ParagraphInlineFlowItem<'_>>,
    logical: &mut String,
    wrap_width: Pixels,
    line_height: Pixels,
    default_metrics: InlineItemMetrics,
    prefix_width: Pixels,
    text_style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) {
    if flow_items.is_empty() {
        return;
    }

    let logical_breaks: Vec<usize> = linebreaks(logical.as_str()).map(|(ix, _)| ix).collect();
    append_paragraph_inline_flow_items(
        computed,
        line,
        flow_items,
        &logical_breaks,
        wrap_width,
        line_height,
        default_metrics,
        prefix_width,
        text_style,
        window,
        cx,
    );
    flow_items.clear();
    logical.clear();
}

fn append_paragraph_inline_flow_items(
    computed: &mut ParagraphInlineComputed,
    line: &mut ParagraphInlineLine,
    flow_items: &[ParagraphInlineFlowItem<'_>],
    logical_breaks: &[usize],
    wrap_width: Pixels,
    line_height: Pixels,
    default_metrics: InlineItemMetrics,
    prefix_width: Pixels,
    text_style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) {
    let mut start = 0;

    while start < flow_items.len() {
        let line_start = start;
        let mut end = start;
        let mut line_width = line.width;
        let mut last_fit_end = start;
        let mut last_break_end = None;

        while end < flow_items.len() {
            let item = &flow_items[end];
            let next_width = line_width + item.width;
            if next_width > wrap_width && end > line_start {
                break;
            }

            line_width = next_width;
            end += 1;
            last_fit_end = end;

            if logical_breaks.binary_search(&item.logical_end).is_ok() {
                last_break_end = Some(end);
            }

            if line_width > wrap_width {
                break;
            }
        }

        let mut line_end = if end < flow_items.len() {
            last_break_end
                .filter(|break_end| *break_end > line_start)
                .unwrap_or(last_fit_end.max(line_start + 1))
        } else {
            flow_items.len()
        };

        while line_end > line_start + 1
            && line.width
                + paragraph_inline_flow_items_width(
                    &flow_items[start..line_end],
                    text_style,
                    window,
                    cx,
                )
                > wrap_width
        {
            line_end -= 1;
        }

        push_paragraph_inline_flow_items(
            line,
            &flow_items[start..line_end],
            line_height,
            text_style,
            window,
            cx,
        );

        if line_end < flow_items.len() {
            finish_paragraph_inline_line(computed, line, prefix_width, default_metrics);
        }

        start = line_end;
    }
}

fn paragraph_inline_flow_items_width(
    flow_items: &[ParagraphInlineFlowItem<'_>],
    text_style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) -> Pixels {
    let mut width = px(0.);
    let mut ix = 0;

    while ix < flow_items.len() {
        match &flow_items[ix].kind {
            ParagraphInlineFlowItemKind::Text { text, range } => {
                let mut end_ix = ix + 1;
                let mut range = range.clone();

                while let Some(ParagraphInlineFlowItem {
                    kind:
                        ParagraphInlineFlowItemKind::Text {
                            text: next_text,
                            range: next_range,
                        },
                    ..
                }) = flow_items.get(end_ix)
                {
                    if std::ptr::eq(*text, *next_text) && range.end == next_range.start {
                        range.end = next_range.end;
                        end_ix += 1;
                    } else {
                        break;
                    }
                }

                let shaped = shape_inline_text(
                    SharedString::new(&text.text[range.clone()]),
                    &text.highlights,
                    range,
                    text_style,
                    window,
                    cx,
                );
                width += shaped.width();
                ix = end_ix;
            }
            ParagraphInlineFlowItemKind::Image { size, .. } => {
                width += size.width;
                ix += 1;
            }
            ParagraphInlineFlowItemKind::Math { metrics, .. } => {
                width += metrics.size.width;
                ix += 1;
            }
        }
    }

    width
}

fn push_paragraph_inline_flow_items(
    line: &mut ParagraphInlineLine,
    flow_items: &[ParagraphInlineFlowItem<'_>],
    line_height: Pixels,
    text_style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) {
    let mut ix = 0;
    while ix < flow_items.len() {
        match &flow_items[ix].kind {
            ParagraphInlineFlowItemKind::Text { text, range } => {
                let mut end_ix = ix + 1;
                let mut range = range.clone();

                while let Some(ParagraphInlineFlowItem {
                    kind:
                        ParagraphInlineFlowItemKind::Text {
                            text: next_text,
                            range: next_range,
                        },
                    ..
                }) = flow_items.get(end_ix)
                {
                    if std::ptr::eq(*text, *next_text) && range.end == next_range.start {
                        range.end = next_range.end;
                        end_ix += 1;
                    } else {
                        break;
                    }
                }

                let shaped = shape_inline_text(
                    SharedString::new(&text.text[range.clone()]),
                    &text.highlights,
                    range.clone(),
                    text_style,
                    window,
                    cx,
                );
                push_laid_out_text(line, text, range, shaped, line_height);
                ix = end_ix;
            }
            ParagraphInlineFlowItemKind::Image {
                image,
                size,
                metrics,
            } => {
                push_laid_out_image(line, image, *size, *metrics);
                ix += 1;
            }
            ParagraphInlineFlowItemKind::Math { math, metrics } => {
                push_laid_out_math(line, math, *metrics);
                ix += 1;
            }
        }
    }
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
    let height = metrics.ascent + metrics.descent;
    update_line_metrics(line, metrics.ascent, metrics.descent);
    line.items
        .push(ParagraphInlineLayoutItem::Text(LaidOutInlineText {
            x: line.width,
            y: line.ascent - metrics.ascent,
            height,
            text: fragment,
            source_text: text.source_text.clone(),
            source_range: (text.source_offset + range.start)..(text.source_offset + range.end),
            state: text.state.clone(),
            line: shaped,
            links: text.links.clone(),
        }));
    line.width += width;
}

fn push_laid_out_image(
    line: &mut ParagraphInlineLine,
    image: &ParagraphInlineImage,
    size: Size<Pixels>,
    metrics: InlineItemMetrics,
) {
    update_line_metrics(line, metrics.ascent, metrics.descent);
    let y = line.ascent - metrics.ascent;
    let element = inline_image_element(&image.node, image.id, size);
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

fn push_laid_out_math(
    line: &mut ParagraphInlineLine,
    math: &ParagraphInlineMath,
    metrics: MathMetrics,
) {
    update_line_metrics(line, metrics.ascent, metrics.descent);
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

fn inline_image_element(image: &ImageNode, id: usize, size: Size<Pixels>) -> AnyElement {
    let title = image.title();
    let link = image.link.clone();

    img(image.url.clone())
        .id(("inline-image", id))
        .object_fit(ObjectFit::Contain)
        .w(size.width)
        .h(size.height)
        .when_some(link, |this, link| {
            this.cursor_pointer()
                .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    cx.open_url(&link.url);
                })
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
    image_size: Size<Pixels>,
    max_width: Pixels,
    default_metrics: InlineItemMetrics,
) -> (Size<Pixels>, InlineItemMetrics) {
    let size = constrain_inline_image_size(image_size, max_width, default_metrics);
    let metrics = inline_image_metrics(size, default_metrics);

    (size, metrics)
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
    let descent = default_metrics.descent.max(px(0.)).min(size.height);

    InlineItemMetrics {
        ascent: size.height - descent,
        descent,
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

fn default_text_line_metrics(text_style: &TextStyle, window: &Window) -> InlineItemMetrics {
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let font_id = window.text_system().resolve_font(&text_style.font());
    let line_height = text_style.line_height_in_pixels(window.rem_size());
    let ascent = window
        .text_system()
        .baseline_offset(font_id, font_size, line_height);

    InlineItemMetrics {
        ascent,
        descent: line_height - ascent,
    }
}

fn shaped_text_line_metrics(shaped: &ShapedLine, line_height: Pixels) -> InlineItemMetrics {
    let line_height = line_height.max(shaped.ascent + shaped.descent);
    let padding_top = (line_height - shaped.ascent - shaped.descent) / 2.;
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
    text_style: &TextStyle,
    window: &mut Window,
    _cx: &mut App,
) -> ShapedLine {
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
        runs.push(text_style.clone().to_run(source_range.end - ix));
    }

    let runs = crate::text::utils::normalize_runs_for_text(text.as_ref(), runs);
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    window
        .text_system()
        .shape_line(text, font_size, &runs, None)
}

fn prepaint_paragraph_inline_images(
    layout: &mut ParagraphInlineComputed,
    bounds: Bounds<Pixels>,
    options: NodeRenderOptions,
    window: &mut Window,
    cx: &mut App,
) {
    let prefix_width = layout
        .prefix
        .as_ref()
        .map_or(px(0.), ParagraphInlinePrefix::width);
    for line in &mut layout.lines {
        let line_offset = paragraph_line_offset(line, bounds.size.width, prefix_width, options);
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

fn prepaint_paragraph_inline_prefix(
    layout: &mut ParagraphInlineComputed,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(ParagraphInlinePrefix::Todo {
        y,
        size,
        element: Some(element),
        ..
    }) = &mut layout.prefix
    else {
        return;
    };

    element.prepaint_as_root(
        bounds.origin + point(px(0.), *y),
        Size {
            width: AvailableSpace::Definite(*size),
            height: AvailableSpace::Definite(*size),
        },
        window,
        cx,
    );
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

    paint_paragraph_inline_prefix(layout, bounds, window, cx);

    let prefix_width = layout
        .prefix
        .as_ref()
        .map_or(px(0.), ParagraphInlinePrefix::width);
    for line in &mut layout.lines {
        let line_offset = paragraph_line_offset(line, bounds.size.width, prefix_width, options);
        for item in &mut line.items {
            match item {
                ParagraphInlineLayoutItem::Text(text) => {
                    record_text_state(&mut text_states, &text.state, text.source_text.clone());
                    let origin = bounds.origin + point(line_offset + text.x, line.y + text.y);
                    let _ = text.line.paint_background(
                        origin,
                        text.height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
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
        let current_view = window.current_view();
        let hover_hitbox = hitbox.clone();
        let link_hover_bounds = link_bounds
            .iter()
            .map(|(bounds, _)| *bounds)
            .collect::<Vec<_>>();
        let mut hovering_link = hovered_link.is_some();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if !phase.bubble() || !hover_hitbox.is_hovered(window) {
                return;
            }

            let updated = link_hover_bounds
                .iter()
                .any(|bounds| bounds.contains(&event.position));
            if hovering_link != updated {
                hovering_link = updated;
                cx.notify(current_view);
            }
        });

        let click_hitbox = hitbox.clone();
        let text_view_state = GlobalState::global(cx).text_view_state().cloned();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if !phase.bubble() || !click_hitbox.is_hovered(window) {
                return;
            }

            let Some(text_view_state) = &text_view_state else {
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

fn paint_paragraph_inline_prefix(
    layout: &mut ParagraphInlineComputed,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    match &mut layout.prefix {
        Some(ParagraphInlinePrefix::Marker {
            y,
            height,
            line: Some(line),
            ..
        }) => {
            let origin = bounds.origin + point(px(0.), *y);
            let _ = line.paint(origin, *height, TextAlign::Left, None, window, cx);
        }
        Some(ParagraphInlinePrefix::Todo {
            element: Some(element),
            ..
        }) => {
            element.paint(window, cx);
        }
        _ => {}
    }
}

fn paragraph_line_offset(
    line: &ParagraphInlineLine,
    width: Pixels,
    prefix_width: Pixels,
    options: NodeRenderOptions,
) -> Pixels {
    let width = (width - prefix_width).max(px(0.));
    let align = match line.align {
        ParagraphLineAlign::Center => ColumnAlign::Center,
        ParagraphLineAlign::Column => options.column_align,
    };
    prefix_width
        + match align {
            ColumnAlign::Left => px(0.),
            ColumnAlign::Center => (width - line.width).max(px(0.)) / 2.,
            ColumnAlign::Right => (width - line.width).max(px(0.)),
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

#[cfg(test)]
mod inline_selection_tests {
    use gpui::{point, px};

    use super::point_in_inline_selection;

    #[test]
    fn test_point_in_inline_selection_multiline() {
        let line_height = px(20.);
        let char_width = px(10.);
        let start = point(px(50.), px(50.));
        let end = point(px(150.), px(150.));

        assert!(point_in_inline_selection(
            point(px(50.), px(50.)),
            char_width,
            start,
            end,
            line_height
        ));
        assert!(!point_in_inline_selection(
            point(px(40.), px(50.)),
            char_width,
            start,
            end,
            line_height
        ));
        assert!(point_in_inline_selection(
            point(px(40.), px(70.)),
            char_width,
            start,
            end,
            line_height
        ));
        assert!(!point_in_inline_selection(
            point(px(160.), px(140.)),
            char_width,
            start,
            end,
            line_height
        ));
    }

    #[test]
    fn test_point_in_inline_selection_same_visual_line_with_reversed_y() {
        let line_height = px(20.);
        let char_width = px(10.);
        let start = point(px(60.), px(58.));
        let end = point(px(100.), px(55.));

        assert!(!point_in_inline_selection(
            point(px(40.), px(50.)),
            char_width,
            start,
            end,
            line_height
        ));
        assert!(point_in_inline_selection(
            point(px(70.), px(50.)),
            char_width,
            start,
            end,
            line_height
        ));
        assert!(!point_in_inline_selection(
            point(px(110.), px(50.)),
            char_width,
            start,
            end,
            line_height
        ));
    }
}
