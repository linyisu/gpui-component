use gpui::{Pixels, Size};

#[derive(Clone, Copy, Default)]
pub(crate) struct MathMetrics {
    pub(crate) size: Size<Pixels>,
    /// The baseline offset from the top of `size`.
    pub(crate) ascent: Pixels,
    pub(crate) descent: Pixels,
}

#[cfg(feature = "markdown-math")]
mod real {
    use std::sync::{Arc, Mutex};

    use gpui::{
        App, Bounds, Element, ElementId, FillOptions, FillRule, GlobalElementId, Hsla,
        InspectorElementId, IntoElement, LayoutId, PathBuilder, PathStyle, Pixels, Rgba,
        ShapedLine, SharedString, Style, TextAlign, TextRun, TextSystem, Window, fill, font, point,
        px, size,
    };
    use ratex_font::{FontId, katex_ttf_glyph_char};
    use ratex_layout::{LayoutOptions, layout, to_display_list};
    use ratex_types::{Color, DisplayItem, DisplayList, MathStyle, path_command::PathCommand};

    use crate::{ActiveTheme as _, global_state::GlobalState};

    use crate::text::{inline::InlineState, node::Span};

    const MATH_PADDING: Pixels = px(1.);
    const PATH_STROKE_WIDTH: Pixels = px(1.5);

    pub(super) fn init(cx: &mut App) {
        register_katex_fonts(cx.text_system().as_ref());
    }

    #[derive(Debug, Clone)]
    pub(crate) struct MathNode {
        source: SharedString,
        markdown_source: SharedString,
        display: bool,
        display_list: Option<DisplayList>,
        state: Arc<Mutex<InlineState>>,
        span: Option<Span>,
    }

    impl PartialEq for MathNode {
        fn eq(&self, other: &Self) -> bool {
            self.source == other.source
                && self.markdown_source == other.markdown_source
                && self.display == other.display
                && self.span == other.span
        }
    }

    impl MathNode {
        pub(crate) fn try_new(
            source: impl Into<SharedString>,
            display: bool,
        ) -> Result<Self, SharedString> {
            let source = source.into();
            let markdown_source = math_markdown_source(&source, display);
            let ast = ratex_parser::parse(source.as_ref())
                .map_err(|err| SharedString::from(err.to_string()))?;
            let options = LayoutOptions {
                style: if display {
                    MathStyle::Display
                } else {
                    MathStyle::Text
                },
                color: Color::BLACK,
                ..LayoutOptions::default()
            };
            let layout_box = layout(&ast, &options);
            let display_list = to_display_list(&layout_box);
            let state = Arc::new(Mutex::new(InlineState::default()));
            state.lock().unwrap().set_text(markdown_source.clone());

            Ok(Self {
                source,
                markdown_source,
                display,
                display_list: Some(display_list),
                state,
                span: None,
            })
        }

        pub(crate) fn fallback(
            source: impl Into<SharedString>,
            markdown_source: impl Into<SharedString>,
            display: bool,
        ) -> Self {
            let source = source.into();
            let markdown_source = markdown_source.into();
            let state = Arc::new(Mutex::new(InlineState::default()));
            state.lock().unwrap().set_text(markdown_source.clone());

            Self {
                source,
                markdown_source,
                display,
                display_list: None,
                state,
                span: None,
            }
        }

        pub(crate) fn with_span(mut self, span: Option<Span>) -> Self {
            self.span = span;
            self
        }

        pub(crate) fn with_markdown_source(
            mut self,
            markdown_source: impl Into<SharedString>,
        ) -> Self {
            self.markdown_source = markdown_source.into();
            self.state
                .lock()
                .unwrap()
                .set_text(self.markdown_source.clone());
            self
        }

        pub(crate) fn span(&self) -> Option<Span> {
            self.span
        }

        pub(crate) fn source(&self) -> &SharedString {
            &self.source
        }

        pub(crate) fn is_display(&self) -> bool {
            self.display
        }

        pub(crate) fn render(&self) -> MathElement {
            MathElement::new(self.clone())
        }

        pub(crate) fn layout_metrics(&self, window: &Window) -> super::MathMetrics {
            MathElement::new(self.clone()).layout_for(window).1
        }

        pub(crate) fn paint_at(
            &self,
            bounds: Bounds<Pixels>,
            text_color: Option<Hsla>,
            window: &mut Window,
            cx: &mut App,
        ) {
            let element = MathElement::new(self.clone());
            let (layout, _) = element.layout_for(window);
            let text_color = text_color.unwrap_or_else(|| window.text_style().color);
            element.paint_with_color(bounds, &layout, text_color, window, cx);
        }

        pub(crate) fn markdown_source(&self) -> SharedString {
            self.markdown_source.clone()
        }

        pub(crate) fn selected_text(&self) -> String {
            let state = self.state.lock().unwrap();
            if let Some(selection) = &state.selection {
                let text = state.text.clone();
                text[selection.start..selection.end].to_string()
            } else {
                String::new()
            }
        }

        #[cfg(test)]
        pub(crate) fn select_all_for_test(&self) {
            let mut state = self.state.lock().unwrap();
            state.selection = Some((0..state.text.len()).into());
        }
    }

    fn math_markdown_source(source: &SharedString, display: bool) -> SharedString {
        if display {
            format!("$$\n{}\n$$", source).into()
        } else {
            format!("${}$", source).into()
        }
    }

    #[derive(Clone)]
    pub(crate) struct MathElement {
        node: MathNode,
    }

    #[derive(Clone, Copy)]
    pub(crate) struct MathLayout {
        em: Pixels,
        padding: Pixels,
    }

    impl MathElement {
        pub(crate) fn new(node: MathNode) -> Self {
            Self { node }
        }

        fn layout_for(&self, window: &Window) -> (MathLayout, super::MathMetrics) {
            let font_size = window.text_style().font_size.to_pixels(window.rem_size());
            if self.node.display_list.is_none() {
                return self.fallback_layout_for(font_size, window);
            }

            let display_list = self.node.display_list.as_ref().unwrap();
            let em = if self.node.display {
                font_size * 1.1
            } else {
                font_size
            };
            let padding = MATH_PADDING;
            let em_px = f32::from(em);
            let width = px((display_list.width as f32 * em_px).max(1.)) + padding * 2.;
            let ascent = px((display_list.height as f32 * em_px).max(0.)) + padding;
            let descent = px((display_list.depth as f32 * em_px).max(0.)) + padding;
            let height = (ascent + descent).max(px(1.));

            (
                MathLayout { em, padding },
                super::MathMetrics {
                    size: size(width, height),
                    ascent,
                    descent,
                },
            )
        }

        fn fallback_layout_for(
            &self,
            font_size: Pixels,
            window: &Window,
        ) -> (MathLayout, super::MathMetrics) {
            let padding = MATH_PADDING;
            let line_height = window.text_style().line_height_in_pixels(window.rem_size());
            let mut width = px(1.);
            let mut line_count = 0;

            for line in fallback_math_lines(&self.node.markdown_source) {
                line_count += 1;
                width = width.max(
                    shape_fallback_line(line, font_size, window.text_style().color, window).width(),
                );
            }

            let content_height = line_height * line_count.max(1) as f32;
            let ascent = content_height + padding;
            let descent = padding;
            (
                MathLayout {
                    em: font_size,
                    padding,
                },
                super::MathMetrics {
                    size: size(width + padding * 2., ascent + descent),
                    ascent,
                    descent,
                },
            )
        }

        fn update_selection(&self, bounds: Bounds<Pixels>, window: &Window, cx: &mut App) -> bool {
            let Some(text_view_state) = GlobalState::global(cx).text_view_state() else {
                self.node.state.lock().unwrap().selection = None;
                return false;
            };
            let text_view_state = text_view_state.read(cx);
            if !text_view_state.has_selection() {
                self.node.state.lock().unwrap().selection = None;
                return false;
            }

            let is_selected = text_view_state.is_selectable()
                && text_view_state
                    .selection_points()
                    .map(|(start, end)| bounds_intersects_selection(bounds, start, end, window))
                    .unwrap_or(false);

            let mut state = self.node.state.lock().unwrap();
            state.selection = if is_selected {
                Some((0..state.text.len()).into())
            } else {
                None
            };

            is_selected
        }

        fn paint_with_color(
            &self,
            bounds: Bounds<Pixels>,
            layout: &MathLayout,
            text_color: Hsla,
            window: &mut Window,
            cx: &mut App,
        ) {
            let origin = bounds.origin + point(layout.padding, layout.padding);

            if self.update_selection(bounds, window, cx) {
                window.paint_quad(fill(bounds, cx.theme().selection));
            }

            let Some(display_list) = &self.node.display_list else {
                self.paint_fallback(origin, text_color, window, cx);
                return;
            };

            for item in &display_list.items {
                paint_display_item(item, origin, layout.em, text_color, window);
            }
        }

        fn paint_fallback(
            &self,
            origin: gpui::Point<Pixels>,
            text_color: Hsla,
            window: &mut Window,
            cx: &mut App,
        ) {
            let font_size = window.text_style().font_size.to_pixels(window.rem_size());
            let line_height = window.text_style().line_height_in_pixels(window.rem_size());
            for (ix, text) in fallback_math_lines(&self.node.markdown_source)
                .into_iter()
                .enumerate()
            {
                let line = shape_fallback_line(text, font_size, text_color, window);
                let origin = origin + point(px(0.), line_height * ix as f32);
                let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
            }
        }
    }

    fn fallback_math_lines(source: &SharedString) -> Vec<&str> {
        let lines = source.as_ref().lines().collect::<Vec<_>>();
        if lines.is_empty() { vec![""] } else { lines }
    }

    fn shape_fallback_line(
        text: &str,
        font_size: Pixels,
        color: Hsla,
        window: &Window,
    ) -> ShapedLine {
        let text: SharedString = text.to_string().into();
        let run = TextRun {
            len: text.len(),
            font: window.text_style().font().clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(text, font_size, &[run], None)
    }

    fn bounds_intersects_selection(
        bounds: Bounds<Pixels>,
        selection_start: gpui::Point<Pixels>,
        selection_end: gpui::Point<Pixels>,
        window: &Window,
    ) -> bool {
        let line_height = window.line_height();
        let bounds_left = bounds.origin.x;
        let bounds_right = bounds.origin.x + bounds.size.width;
        let bounds_center_y = bounds.origin.y + bounds.size.height / 2.;

        let y_delta = if selection_start.y > selection_end.y {
            selection_start.y - selection_end.y
        } else {
            selection_end.y - selection_start.y
        };
        let same_line = y_delta <= line_height / 2.;
        if same_line {
            let selection_left = selection_start.x.min(selection_end.x);
            let selection_right = selection_start.x.max(selection_end.x);
            let selection_y = selection_start.y;

            return bounds_center_y >= selection_y - line_height / 2.
                && bounds_center_y <= selection_y + line_height / 2.
                && bounds_right >= selection_left
                && bounds_left <= selection_right;
        }

        let (top_point, bottom_point) = if selection_start.y < selection_end.y {
            (selection_start, selection_end)
        } else {
            (selection_end, selection_start)
        };

        if bounds_center_y < top_point.y - line_height / 2.
            || bounds_center_y > bottom_point.y + line_height / 2.
        {
            return false;
        }

        if bounds_center_y <= top_point.y + line_height / 2. {
            return bounds_right >= top_point.x;
        }

        if bounds_center_y >= bottom_point.y - line_height / 2. {
            return bounds_left <= bottom_point.x;
        }

        true
    }

    impl IntoElement for MathElement {
        type Element = Self;

        fn into_element(self) -> Self::Element {
            self
        }
    }

    impl Element for MathElement {
        type RequestLayoutState = MathLayout;
        type PrepaintState = ();

        fn id(&self) -> Option<ElementId> {
            None
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
            let (layout, metrics) = self.layout_for(window);
            let mut style = Style::default();
            style.size.width = metrics.size.width.into();
            style.size.height = metrics.size.height.into();

            (window.request_layout(style, [], cx), layout)
        }

        fn prepaint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            _: Bounds<Pixels>,
            _: &mut Self::RequestLayoutState,
            _: &mut Window,
            _: &mut App,
        ) -> Self::PrepaintState {
        }

        fn paint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            bounds: Bounds<Pixels>,
            layout: &mut Self::RequestLayoutState,
            _: &mut Self::PrepaintState,
            window: &mut Window,
            cx: &mut App,
        ) {
            let text_color = window.text_style().color;
            self.paint_with_color(bounds, layout, text_color, window, cx);
        }
    }

    fn paint_display_item(
        item: &DisplayItem,
        origin: gpui::Point<Pixels>,
        em: Pixels,
        text_color: Hsla,
        window: &mut Window,
    ) {
        let em_px = f32::from(em);
        match item {
            DisplayItem::GlyphPath {
                x,
                y,
                scale,
                font,
                char_code,
                color,
            } => {
                let origin = origin + point(px(*x as f32 * em_px), px(*y as f32 * em_px));
                let em = em * *scale as f32;
                let color = math_color(color, text_color);
                paint_glyph(origin, em, font, *char_code, color, window);
            }
            DisplayItem::Line {
                x,
                y,
                width,
                thickness,
                color,
                dashed,
            } => {
                paint_line(
                    origin + point(px(*x as f32 * em_px), px(*y as f32 * em_px)),
                    px(*width as f32 * em_px),
                    px((*thickness as f32 * em_px).max(1.)),
                    *dashed,
                    math_color(color, text_color),
                    window,
                );
            }
            DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color,
            } => {
                let bounds = Bounds {
                    origin: origin + point(px(*x as f32 * em_px), px(*y as f32 * em_px)),
                    size: size(
                        px((*width as f32 * em_px).max(1.)),
                        px((*height as f32 * em_px).max(1.)),
                    ),
                };
                window.paint_quad(fill(bounds, math_color(color, text_color)));
            }
            DisplayItem::Path {
                x,
                y,
                commands,
                fill,
                color,
            } => {
                let origin = origin + point(px(*x as f32 * em_px), px(*y as f32 * em_px));
                paint_path(
                    origin,
                    commands,
                    *fill,
                    em,
                    math_color(color, text_color),
                    window,
                );
            }
        }
    }

    fn paint_line(
        origin: gpui::Point<Pixels>,
        width: Pixels,
        thickness: Pixels,
        dashed: bool,
        color: Hsla,
        window: &mut Window,
    ) {
        let top = origin.y - thickness / 2.;

        if dashed {
            let dash = thickness * 4.;
            let gap = dash;
            let mut x = origin.x;
            let end = origin.x + width;
            while x < end {
                let segment_width = dash.min(end - x).max(px(1.));
                window.paint_quad(fill(
                    Bounds {
                        origin: point(x, top),
                        size: size(segment_width, thickness),
                    },
                    color,
                ));
                x += dash + gap;
            }
        } else {
            window.paint_quad(fill(
                Bounds {
                    origin: point(origin.x, top),
                    size: size(width, thickness),
                },
                color,
            ));
        }
    }

    fn paint_path(
        origin: gpui::Point<Pixels>,
        commands: &[PathCommand],
        is_fill: bool,
        em: Pixels,
        color: Hsla,
        window: &mut Window,
    ) {
        if is_fill {
            let mut start = 0;
            for ix in 1..commands.len() {
                if matches!(commands[ix], PathCommand::MoveTo { .. }) {
                    paint_path_segment(origin, &commands[start..ix], true, em, color, window);
                    start = ix;
                }
            }
            paint_path_segment(origin, &commands[start..], true, em, color, window);
        } else {
            paint_path_segment(origin, commands, false, em, color, window);
        }
    }

    fn paint_path_segment(
        origin: gpui::Point<Pixels>,
        commands: &[PathCommand],
        is_fill: bool,
        em: Pixels,
        color: Hsla,
        window: &mut Window,
    ) -> bool {
        if commands.is_empty() {
            return false;
        }

        let mut builder = if is_fill {
            PathBuilder::fill().with_style(PathStyle::Fill(
                FillOptions::default().with_fill_rule(FillRule::EvenOdd),
            ))
        } else {
            PathBuilder::stroke(PATH_STROKE_WIDTH)
        };

        let em_px = f32::from(em);
        for command in commands {
            match command {
                PathCommand::MoveTo { x, y } => {
                    builder.move_to(origin + point(px(*x as f32 * em_px), px(*y as f32 * em_px)));
                }
                PathCommand::LineTo { x, y } => {
                    builder.line_to(origin + point(px(*x as f32 * em_px), px(*y as f32 * em_px)));
                }
                PathCommand::CubicTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                } => {
                    builder.cubic_bezier_to(
                        origin + point(px(*x as f32 * em_px), px(*y as f32 * em_px)),
                        origin + point(px(*x1 as f32 * em_px), px(*y1 as f32 * em_px)),
                        origin + point(px(*x2 as f32 * em_px), px(*y2 as f32 * em_px)),
                    );
                }
                PathCommand::QuadTo { x1, y1, x, y } => {
                    builder.curve_to(
                        origin + point(px(*x as f32 * em_px), px(*y as f32 * em_px)),
                        origin + point(px(*x1 as f32 * em_px), px(*y1 as f32 * em_px)),
                    );
                }
                PathCommand::Close => builder.close(),
            }
        }

        match builder.build() {
            Ok(path) => {
                window.paint_path(path, color);
                true
            }
            Err(err) if cfg!(debug_assertions) => {
                tracing::warn!("failed building math path: {err:?}");
                false
            }
            Err(_) => false,
        }
    }

    fn paint_glyph(
        origin: gpui::Point<Pixels>,
        em: Pixels,
        font_name: &str,
        char_code: u32,
        color: Hsla,
        window: &mut Window,
    ) {
        let font_id = FontId::parse(font_name).unwrap_or(FontId::MainRegular);
        let (ch, font, require_katex_font) = if is_system_fallback_font(font_id) {
            let Some(ch) = char::from_u32(char_code) else {
                return;
            };
            (ch, window.text_style().font().clone(), false)
        } else {
            let ch = katex_ttf_glyph_char(font_id, char_code);
            (ch, katex_gpui_font(font_name), true)
        };
        let text = ch.to_string();
        let run = TextRun {
            len: text.len(),
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window
            .text_system()
            .shape_line(text.into(), em, &[run], None);

        let Some(run) = line.runs.first() else {
            return;
        };
        if require_katex_font && !resolved_font_matches(window, run.font_id, &font) {
            if paint_ttf_glyph_path(origin, em, font_id, ch, color, window) {
                return;
            }
        };
        let Some(glyph) = run.glyphs.first() else {
            return;
        };

        let origin = origin + point(glyph.position.x, px(0.));
        let result = if glyph.is_emoji {
            window.paint_emoji(origin, run.font_id, glyph.id, em)
        } else {
            window.paint_glyph(origin, run.font_id, glyph.id, em, color)
        };

        if let Err(err) = result {
            if cfg!(debug_assertions) {
                tracing::debug!("failed painting math glyph: {err:?}");
            }
        }
    }

    fn paint_ttf_glyph_path(
        origin: gpui::Point<Pixels>,
        em: Pixels,
        font_id: FontId,
        ch: char,
        color: Hsla,
        window: &mut Window,
    ) -> bool {
        if let Some(commands) = katex_ttf_outline_commands(font_id, ch) {
            paint_path_segment(origin, &commands, true, em, color, window)
        } else {
            if cfg!(debug_assertions) {
                tracing::debug!("missing KaTeX TTF outline for {font_id:?} glyph {ch:?}");
            }
            false
        }
    }

    fn katex_ttf_outline_commands(font_id: FontId, ch: char) -> Option<Vec<PathCommand>> {
        let filename = KATEX_FONT_FILES
            .iter()
            .find_map(|(id, filename)| (*id == font_id).then_some(*filename))?;
        let font_data = ratex_katex_fonts::ttf_bytes(filename)?;
        let face = ttf_parser::Face::parse(font_data.as_ref(), 0).ok()?;
        let glyph_id = face.glyph_index(ch)?;
        let units_per_em = f64::from(face.units_per_em());
        let mut builder = TtfOutlineBuilder {
            units_per_em,
            commands: Vec::new(),
        };

        face.outline_glyph(glyph_id, &mut builder)?;
        Some(builder.commands)
    }

    struct TtfOutlineBuilder {
        units_per_em: f64,
        commands: Vec<PathCommand>,
    }

    impl TtfOutlineBuilder {
        fn x(&self, x: f32) -> f64 {
            f64::from(x) / self.units_per_em
        }

        fn y(&self, y: f32) -> f64 {
            -f64::from(y) / self.units_per_em
        }
    }

    impl ttf_parser::OutlineBuilder for TtfOutlineBuilder {
        fn move_to(&mut self, x: f32, y: f32) {
            self.commands.push(PathCommand::MoveTo {
                x: self.x(x),
                y: self.y(y),
            });
        }

        fn line_to(&mut self, x: f32, y: f32) {
            self.commands.push(PathCommand::LineTo {
                x: self.x(x),
                y: self.y(y),
            });
        }

        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            self.commands.push(PathCommand::QuadTo {
                x1: self.x(x1),
                y1: self.y(y1),
                x: self.x(x),
                y: self.y(y),
            });
        }

        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            self.commands.push(PathCommand::CubicTo {
                x1: self.x(x1),
                y1: self.y(y1),
                x2: self.x(x2),
                y2: self.y(y2),
                x: self.x(x),
                y: self.y(y),
            });
        }

        fn close(&mut self) {
            self.commands.push(PathCommand::Close);
        }
    }

    fn is_system_fallback_font(font_id: FontId) -> bool {
        matches!(
            font_id,
            FontId::CjkRegular | FontId::CjkFallback | FontId::EmojiFallback
        )
    }

    fn register_katex_fonts(text_system: &TextSystem) {
        let fonts = KATEX_FONT_FILES
            .iter()
            .filter_map(|(_, filename)| ratex_katex_fonts::ttf_bytes(filename))
            .collect::<Vec<_>>();

        if fonts.is_empty() {
            return;
        }

        match text_system.add_fonts(fonts) {
            Ok(()) => {}
            Err(err) if cfg!(debug_assertions) => {
                tracing::warn!("failed registering embedded KaTeX fonts: {err:?}");
            }
            Err(_) => {}
        }
    }

    fn resolved_font_matches(
        window: &Window,
        font_id: gpui::FontId,
        expected: &gpui::Font,
    ) -> bool {
        window
            .text_system()
            .get_font_for_id(font_id)
            .map(|resolved| {
                resolved.family == expected.family
                    && resolved.weight == expected.weight
                    && resolved.style == expected.style
            })
            .unwrap_or(false)
    }

    fn katex_gpui_font(font_name: &str) -> gpui::Font {
        match font_name {
            "Main-Bold" => font("KaTeX_Main").bold(),
            "Main-Italic" => font("KaTeX_Main").italic(),
            "Main-BoldItalic" => font("KaTeX_Main").bold().italic(),
            "Math-Italic" => font("KaTeX_Math").italic(),
            "Math-BoldItalic" => font("KaTeX_Math").bold().italic(),
            "AMS-Regular" => font("KaTeX_AMS"),
            "Caligraphic-Regular" => font("KaTeX_Caligraphic"),
            "Fraktur-Regular" => font("KaTeX_Fraktur"),
            "Fraktur-Bold" => font("KaTeX_Fraktur").bold(),
            "SansSerif-Regular" => font("KaTeX_SansSerif"),
            "SansSerif-Bold" => font("KaTeX_SansSerif").bold(),
            "SansSerif-Italic" => font("KaTeX_SansSerif").italic(),
            "Script-Regular" => font("KaTeX_Script"),
            "Typewriter-Regular" => font("KaTeX_Typewriter"),
            "Size1-Regular" => font("KaTeX_Size1"),
            "Size2-Regular" => font("KaTeX_Size2"),
            "Size3-Regular" => font("KaTeX_Size3"),
            "Size4-Regular" => font("KaTeX_Size4"),
            _ => font("KaTeX_Main"),
        }
    }

    fn math_color(color: &Color, default_color: Hsla) -> Hsla {
        if color.r == 0.0 && color.g == 0.0 && color.b == 0.0 && color.a == 1.0 {
            default_color
        } else {
            Rgba {
                r: color.r,
                g: color.g,
                b: color.b,
                a: color.a,
            }
            .into()
        }
    }

    const KATEX_FONT_FILES: &[(FontId, &str)] = &[
        (FontId::MainRegular, "KaTeX_Main-Regular.ttf"),
        (FontId::MainBold, "KaTeX_Main-Bold.ttf"),
        (FontId::MainItalic, "KaTeX_Main-Italic.ttf"),
        (FontId::MainBoldItalic, "KaTeX_Main-BoldItalic.ttf"),
        (FontId::MathItalic, "KaTeX_Math-Italic.ttf"),
        (FontId::MathBoldItalic, "KaTeX_Math-BoldItalic.ttf"),
        (FontId::AmsRegular, "KaTeX_AMS-Regular.ttf"),
        (FontId::CaligraphicRegular, "KaTeX_Caligraphic-Regular.ttf"),
        (FontId::FrakturRegular, "KaTeX_Fraktur-Regular.ttf"),
        (FontId::FrakturBold, "KaTeX_Fraktur-Bold.ttf"),
        (FontId::SansSerifRegular, "KaTeX_SansSerif-Regular.ttf"),
        (FontId::SansSerifBold, "KaTeX_SansSerif-Bold.ttf"),
        (FontId::SansSerifItalic, "KaTeX_SansSerif-Italic.ttf"),
        (FontId::ScriptRegular, "KaTeX_Script-Regular.ttf"),
        (FontId::TypewriterRegular, "KaTeX_Typewriter-Regular.ttf"),
        (FontId::Size1Regular, "KaTeX_Size1-Regular.ttf"),
        (FontId::Size2Regular, "KaTeX_Size2-Regular.ttf"),
        (FontId::Size3Regular, "KaTeX_Size3-Regular.ttf"),
        (FontId::Size4Regular, "KaTeX_Size4-Regular.ttf"),
    ];

    #[cfg(test)]
    mod tests {
        use ratex_font::{FontId, katex_ttf_glyph_char};

        use super::{MathNode, katex_ttf_outline_commands};

        #[test]
        fn test_math_selected_text_matches_markdown_source() {
            let inline = MathNode::try_new("x^2+y^2", false).unwrap();
            let inline_markdown = inline.markdown_source();
            assert_eq!(inline_markdown.as_ref(), "$x^2+y^2$");
            inline.state.lock().unwrap().selection = Some((0..inline_markdown.len()).into());
            assert_eq!(inline.selected_text(), inline_markdown.as_ref());

            let block = MathNode::try_new("x^2+y^2", true).unwrap();
            let block_markdown = block.markdown_source();
            assert_eq!(block_markdown.as_ref(), "$$\nx^2+y^2\n$$");
            block.state.lock().unwrap().selection = Some((0..block_markdown.len()).into());
            assert_eq!(block.selected_text(), block_markdown.as_ref());

            let paragraph_display = MathNode::try_new("x^2+y^2", true)
                .unwrap()
                .with_markdown_source("$$x^2+y^2$$");
            let paragraph_display_markdown = paragraph_display.markdown_source();
            paragraph_display.state.lock().unwrap().selection =
                Some((0..paragraph_display_markdown.len()).into());
            assert_eq!(paragraph_display.selected_text(), "$$x^2+y^2$$");
        }

        #[test]
        fn test_adjacent_absolute_value_bars_are_preserved() {
            let math =
                MathNode::try_new("\\vec a\\cdot\\vec b=|\\vec a||\\vec b|\\cos\\theta", true)
                    .unwrap();

            let bars: Vec<_> = math
                .display_list
                .as_ref()
                .unwrap()
                .items
                .iter()
                .filter_map(|item| match item {
                    ratex_types::DisplayItem::GlyphPath { char_code, x, .. }
                        if *char_code == '∣' as u32 || *char_code == '|' as u32 =>
                    {
                        Some(x)
                    }
                    _ => None,
                })
                .collect();

            assert_eq!(bars.len(), 4, "expected four absolute-value bars");
            assert!(
                bars.windows(2).all(|pair| pair[0] < pair[1]),
                "bars should render at distinct increasing x positions: {bars:?}"
            );
        }

        #[test]
        fn test_multiline_absolute_value_bars_are_preserved() {
            let math = MathNode::try_new(
                "\\vec a\\cdot\\vec b\n=\n|\\vec a||\\vec b|\\cos\\theta",
                true,
            )
            .unwrap();

            let bar_count = math
                .display_list
                .as_ref()
                .unwrap()
                .items
                .iter()
                .filter(|item| match item {
                    ratex_types::DisplayItem::GlyphPath { char_code, .. } => {
                        matches!(char::from_u32(*char_code), Some('|' | '∣'))
                    }
                    _ => false,
                })
                .count();

            assert_eq!(bar_count, 4, "expected four absolute-value bars");
        }

        #[test]
        fn test_system_fallback_math_glyphs_are_preserved() {
            let cjk = '\u{4E2D}';
            let math = MathNode::try_new(format!("\\text{{{cjk}}}"), false).unwrap();

            assert!(
                math.display_list
                    .as_ref()
                    .unwrap()
                    .items
                    .iter()
                    .any(|item| {
                        matches!(
                            item,
                            ratex_types::DisplayItem::GlyphPath {
                                font,
                                char_code,
                                ..
                        } if font == "CJK-Regular" && char::from_u32(*char_code) == Some(cjk)
                        )
                    }),
                "expected CJK fallback glyph to stay in the display list"
            );
            assert!(super::is_system_fallback_font(
                ratex_font::FontId::CjkRegular
            ));

            let emoji = '\u{1F60A}';
            let math = MathNode::try_new(format!("\\text{{{emoji}}}"), false).unwrap();
            assert!(
                math.display_list
                    .as_ref()
                    .unwrap()
                    .items
                    .iter()
                    .any(|item| {
                        matches!(
                            item,
                            ratex_types::DisplayItem::GlyphPath {
                                font,
                                char_code,
                                ..
                        } if font == "CJK-Regular" && char::from_u32(*char_code) == Some(emoji)
                        )
                    }),
                "expected emoji fallback glyph to stay in the display list"
            );
            assert!(super::is_system_fallback_font(
                ratex_font::FontId::EmojiFallback
            ));
        }

        #[test]
        fn test_katex_ttf_outline_fallback_covers_symbol_fonts() {
            let ams_commands = katex_ttf_outline_commands(FontId::AmsRegular, 'R')
                .expect("expected AMS outline commands");
            assert!(!ams_commands.is_empty());

            let size_commands = katex_ttf_outline_commands(
                FontId::Size4Regular,
                katex_ttf_glyph_char(FontId::Size4Regular, 0x23AA),
            )
            .expect("expected Size4 outline commands");
            assert!(!size_commands.is_empty());
        }
    }
}

#[cfg(feature = "markdown-math")]
pub(crate) use real::MathNode;

pub(super) fn init(cx: &mut gpui::App) {
    #[cfg(feature = "markdown-math")]
    real::init(cx);

    #[cfg(not(feature = "markdown-math"))]
    let _ = cx;
}

#[cfg(not(feature = "markdown-math"))]
mod no_math {
    use gpui::{AnyElement, App, Bounds, Hsla, IntoElement, Pixels, SharedString, Window, div};

    use crate::text::node::Span;

    #[derive(Debug, Clone, PartialEq)]
    pub(crate) struct MathNode {
        source: SharedString,
        markdown_source: SharedString,
        display: bool,
        span: Option<Span>,
    }

    impl MathNode {
        pub(crate) fn try_new(
            _source: impl Into<SharedString>,
            _display: bool,
        ) -> Result<Self, SharedString> {
            Err("markdown math feature is disabled".into())
        }

        pub(crate) fn fallback(
            source: impl Into<SharedString>,
            markdown_source: impl Into<SharedString>,
            display: bool,
        ) -> Self {
            Self {
                source: source.into(),
                markdown_source: markdown_source.into(),
                display,
                span: None,
            }
        }

        pub(crate) fn with_span(mut self, span: Option<Span>) -> Self {
            self.span = span;
            self
        }

        pub(crate) fn with_markdown_source(
            mut self,
            markdown_source: impl Into<SharedString>,
        ) -> Self {
            self.markdown_source = markdown_source.into();
            self
        }

        pub(crate) fn span(&self) -> Option<Span> {
            self.span
        }

        pub(crate) fn source(&self) -> &SharedString {
            &self.source
        }

        pub(crate) fn is_display(&self) -> bool {
            self.display
        }

        pub(crate) fn render(&self) -> AnyElement {
            div().into_any_element()
        }

        pub(crate) fn layout_metrics(&self, _window: &Window) -> super::MathMetrics {
            super::MathMetrics::default()
        }

        pub(crate) fn paint_at(
            &self,
            _bounds: Bounds<Pixels>,
            _text_color: Option<Hsla>,
            _window: &mut Window,
            _cx: &mut App,
        ) {
        }

        pub(crate) fn markdown_source(&self) -> SharedString {
            self.markdown_source.clone()
        }

        pub(crate) fn selected_text(&self) -> String {
            String::new()
        }
    }
}

#[cfg(not(feature = "markdown-math"))]
pub(crate) use no_math::MathNode;
