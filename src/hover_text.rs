// Adapted from `iced_widget::text::rich` (MIT, © the iced contributors).
// The functional changes are an `on_link_hover` callback, which lets the
// application highlight the post referenced by a norloge while hovering it,
// and mouse text selection with clipboard support.

use std::any::Any;
use std::cell::OnceCell;

use iced::advanced::graphics::text as graphics_text;
use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::advanced::renderer::Renderer as _;
use iced::advanced::text::Renderer as _;
use iced::advanced::text::{self, Paragraph, Span};
use iced::advanced::widget::text::{Alignment, LineHeight, Shaping, Style, Wrapping};
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{Element, Event, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::app::{LinkAction, Message};
use crate::selection::{TextPosition, TextSelection};

type ParagraphType = <Renderer as text::Renderer>::Paragraph;
type HoverFn<'a> = Box<dyn Fn(Option<LinkAction>, Point) -> Message + 'a>;

/// A rich-text widget with click and hover callbacks for each link span.
pub struct HoverRich<'a> {
    spans: Vec<Span<'a, LinkAction>>,
    size: Option<Pixels>,
    line_height: LineHeight,
    width: Length,
    height: Length,
    font: Option<<Renderer as text::Renderer>::Font>,
    align_x: Alignment,
    align_y: iced::alignment::Vertical,
    wrapping: Wrapping,
    on_link_click: Option<Box<dyn Fn(LinkAction) -> Message + 'a>>,
    on_link_right_click: Option<Box<dyn Fn(LinkAction) -> Message + 'a>>,
    on_link_hover: Option<HoverFn<'a>>,
    /// Document order when this paragraph participates in text selection.
    order: Option<u64>,
    /// Current selection, if the application has one.
    selection: Option<TextSelection>,
}

impl<'a> HoverRich<'a> {
    pub fn with_spans(spans: Vec<Span<'a, LinkAction>>) -> Self {
        Self {
            spans,
            size: None,
            line_height: LineHeight::default(),
            width: Length::Shrink,
            height: Length::Shrink,
            font: None,
            align_x: Alignment::Default,
            align_y: iced::alignment::Vertical::Top,
            wrapping: Wrapping::default(),
            on_link_click: None,
            on_link_right_click: None,
            on_link_hover: None,
            order: None,
            selection: None,
        }
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    pub fn line_height(mut self, line_height: impl Into<LineHeight>) -> Self {
        self.line_height = line_height.into();
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn wrapping(mut self, wrapping: Wrapping) -> Self {
        self.wrapping = wrapping;
        self
    }

    pub fn on_link_click(mut self, on_link_click: impl Fn(LinkAction) -> Message + 'a) -> Self {
        self.on_link_click = Some(Box::new(on_link_click));
        self
    }

    pub fn on_link_right_click(
        mut self,
        on_link_right_click: impl Fn(LinkAction) -> Message + 'a,
    ) -> Self {
        self.on_link_right_click = Some(Box::new(on_link_right_click));
        self
    }

    pub fn on_link_hover(
        mut self,
        on_link_hover: impl Fn(Option<LinkAction>, Point) -> Message + 'a,
    ) -> Self {
        self.on_link_hover = Some(Box::new(on_link_hover));
        self
    }

    /// Makes this paragraph part of the selectable timeline text.
    ///
    /// `order` must grow in reading order across all selectable paragraphs.
    pub fn selectable(mut self, order: u64, selection: Option<TextSelection>) -> Self {
        self.order = Some(order);
        self.selection = selection;
        self
    }

    /// Builds the same paragraph layout as iced, but keeps a plain cosmic-text
    /// buffer so the widget can map pixels to byte offsets and extract the
    /// selected text. It is created lazily: ordinary rendering only pays for
    /// selection once the pointer touches the text.
    fn selection_layout<'b>(
        &self,
        state: &'b State,
        renderer: &Renderer,
        bounds: Size,
    ) -> &'b SelectionLayout {
        state.selection_layout.get_or_init(|| {
            let size = self.size.unwrap_or_else(|| renderer.default_size());
            let font = self.font.unwrap_or_else(|| renderer.default_font());

            SelectionLayout::new(
                &state.spans,
                bounds,
                size,
                self.line_height,
                font,
                self.align_x,
                self.wrapping,
            )
        })
    }

    /// Local selection range for this paragraph, if the global selection
    /// intersects it. Positions are clamped to the actual rendered text.
    fn local_selection(
        &self,
        state: &State,
        renderer: &Renderer,
        bounds: Size,
    ) -> Option<(TextPosition, TextPosition)> {
        let order = self.order?;
        let selection = self.selection?;
        let (start, end) = selection.ordered();

        if order < start.order || order > end.order {
            return None;
        }

        let layout = self.selection_layout(state, renderer, bounds);
        let last_line = layout.buffer.lines.len().saturating_sub(1);
        let last_index = layout
            .buffer
            .lines
            .get(last_line)
            .map(|line| line.text().len())
            .unwrap_or(0);

        let local_start = if order == start.order {
            (start.line, start.index)
        } else {
            (0, 0)
        };
        let local_end = if order == end.order {
            (end.line, end.index)
        } else {
            (last_line, last_index)
        };

        let start = clamp_position(&layout.buffer, order, local_start.0, local_start.1);
        let end = clamp_position(&layout.buffer, order, local_end.0, local_end.1);

        (start < end).then_some((start, end))
    }

    fn position_at(
        &self,
        state: &State,
        renderer: &Renderer,
        bounds: Size,
        point: Point,
    ) -> Option<TextPosition> {
        let order = self.order?;
        let layout = self.selection_layout(state, renderer, bounds);
        let cursor = layout.buffer.hit(point.x, point.y)?;

        Some(clamp_position(
            &layout.buffer,
            order,
            cursor.line,
            cursor.index,
        ))
    }

    fn refresh_selected_text(&self, state: &mut State, renderer: &Renderer, bounds: Size) {
        state.order = self.order.unwrap_or_default();

        if state.cached_selection == self.selection {
            return;
        }

        state.cached_selection = self.selection;
        state.selected_text = self
            .local_selection(state, renderer, bounds)
            .map(|(start, end)| {
                let layout = self.selection_layout(state, renderer, bounds);
                selected_text(layout, start, end)
            });
    }
}

/// A paragraph layout kept for hit-testing and selection extraction.
struct SelectionLayout {
    buffer: graphics_text::cosmic_text::Buffer,
    text: String,
    line_starts: Vec<usize>,
    segments: Vec<SelectionSegment>,
}

/// A run of displayed text, grouped when contiguous spans share a link.
struct SelectionSegment {
    start: usize,
    end: usize,
    link: Option<LinkAction>,
}

impl SelectionLayout {
    fn new(
        spans: &[Span<'static, LinkAction>],
        bounds: Size,
        size: Pixels,
        line_height: LineHeight,
        font: Font,
        align_x: Alignment,
        wrapping: Wrapping,
    ) -> Self {
        use graphics_text::cosmic_text;

        let mut text = String::new();
        let mut segments: Vec<SelectionSegment> = Vec::new();

        for span in spans {
            let start = text.len();
            text.push_str(span.text.as_ref());
            let end = text.len();

            let merge = segments.last().is_some_and(|previous| {
                previous.end == start && previous.link.is_some() && previous.link == span.link
            });

            if merge {
                if let Some(previous) = segments.last_mut() {
                    previous.end = end;
                }
            } else {
                segments.push(SelectionSegment {
                    start,
                    end,
                    link: span.link.clone(),
                });
            }
        }

        let mut line_starts = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }

        let mut font_system = graphics_text::font_system()
            .write()
            .expect("write font system");

        let mut buffer = cosmic_text::Buffer::new(
            font_system.raw(),
            cosmic_text::Metrics::new(size.into(), line_height.to_absolute(size).into()),
        );

        buffer.set_size(font_system.raw(), Some(bounds.width), Some(bounds.height));
        buffer.set_wrap(font_system.raw(), graphics_text::to_wrap(wrapping));

        buffer.set_rich_text(
            font_system.raw(),
            spans.iter().enumerate().map(|(index, span)| {
                let attrs = graphics_text::to_attributes(span.font.unwrap_or(font));

                let attrs = match (span.size, span.line_height) {
                    (None, None) => attrs,
                    _ => {
                        let size = span.size.unwrap_or(size);

                        attrs.metrics(cosmic_text::Metrics::new(
                            size.into(),
                            span.line_height
                                .unwrap_or(line_height)
                                .to_absolute(size)
                                .into(),
                        ))
                    }
                };

                let attrs = if let Some(color) = span.color {
                    attrs.color(graphics_text::to_color(color))
                } else {
                    attrs
                };

                (span.text.as_ref(), attrs.metadata(index))
            }),
            &graphics_text::to_attributes(font),
            cosmic_text::Shaping::Advanced,
            None,
        );

        graphics_text::align(&mut buffer, font_system.raw(), align_x);

        Self {
            buffer,
            text,
            line_starts,
            segments,
        }
    }

    fn offset(&self, position: TextPosition) -> usize {
        let start = self
            .line_starts
            .get(position.line)
            .copied()
            .unwrap_or(self.text.len());
        let line_end = self.text[start..]
            .find('\n')
            .map(|index| start + index)
            .unwrap_or(self.text.len());
        let index = floor_boundary(&self.text[start..line_end], position.index);

        start + index
    }
}

fn clamp_position(
    buffer: &graphics_text::cosmic_text::Buffer,
    order: u64,
    line: usize,
    index: usize,
) -> TextPosition {
    let line = line.min(buffer.lines.len().saturating_sub(1));
    let text = buffer
        .lines
        .get(line)
        .map(|line| line.text())
        .unwrap_or_default();

    TextPosition {
        order,
        line,
        index: floor_boundary(text, index),
    }
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());

    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }

    index
}

fn selected_text(layout: &SelectionLayout, start: TextPosition, end: TextPosition) -> String {
    let from = layout.offset(start);
    let to = layout.offset(end);

    if from >= to {
        return String::new();
    }

    let mut text = String::new();

    for segment in &layout.segments {
        let lo = from.max(segment.start);
        let hi = to.min(segment.end);

        if lo >= hi {
            continue;
        }

        match &segment.link {
            // A link is copied whole, even if only part of its label was
            // selected.
            Some(LinkAction::Url(url)) => text.push_str(url),
            _ => text.push_str(&layout.text[lo..hi]),
        }
    }

    text
}

fn selection_quads(
    layout: &SelectionLayout,
    translation: Point,
    start: TextPosition,
    end: TextPosition,
) -> Vec<Rectangle> {
    let mut quads = Vec::new();

    for run in layout.buffer.layout_runs() {
        if run.line_i < start.line || run.line_i > end.line {
            continue;
        }

        let selected = |glyph_start: usize, glyph_end: usize| {
            if run.line_i == start.line && glyph_end <= start.index {
                return false;
            }

            if run.line_i == end.line && glyph_start >= end.index {
                return false;
            }

            if run.line_i == start.line && run.line_i == end.line {
                glyph_end > start.index && glyph_start < end.index
            } else {
                true
            }
        };

        let mut current: Option<Rectangle> = None;

        for glyph in run.glyphs {
            if !selected(glyph.start, glyph.end) {
                if let Some(rect) = current.take() {
                    quads.push(rect);
                }
                continue;
            }

            let rect = Rectangle::new(
                Point::new(
                    translation.x + glyph.x,
                    translation.y + run.line_top + glyph.y,
                ),
                Size::new(glyph.w, glyph.line_height_opt.unwrap_or(run.line_height)),
            );

            match current.as_mut() {
                Some(existing)
                    if (existing.x + existing.width - rect.x).abs() < 0.01
                        && (existing.y - rect.y).abs() < 0.01
                        && (existing.height - rect.height).abs() < 0.01 =>
                {
                    existing.width += rect.width;
                }
                Some(_) => {
                    quads.push(current.replace(rect).expect("current rectangle"));
                }
                None => current = Some(rect),
            }
        }

        if let Some(rect) = current {
            quads.push(rect);
        }
    }

    quads
}

struct State {
    spans: Vec<Span<'static, LinkAction>>,
    span_pressed: Option<usize>,
    span_right_pressed: Option<usize>,
    hovered_link: Option<usize>,
    paragraph: ParagraphType,
    selection_layout: OnceCell<SelectionLayout>,
    selected_text: Option<String>,
    cached_selection: Option<TextSelection>,
    order: u64,
    button_down: bool,
    press_point: Option<Point>,
    dragging: bool,
}

/// Returns the selected text stored by a [`HoverRich`] widget, in document
/// order, for the clipboard operation used by the application.
pub fn selection_from_state(state: &dyn Any) -> Option<(u64, &str)> {
    let state = state.downcast_ref::<State>()?;

    Some((state.order, state.selected_text.as_deref()?))
}

impl Widget<Message, Theme, Renderer> for HoverRich<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            spans: Vec::new(),
            span_pressed: None,
            span_right_pressed: None,
            hovered_link: None,
            paragraph: ParagraphType::default(),
            selection_layout: OnceCell::new(),
            selected_text: None,
            cached_selection: None,
            order: 0,
            button_down: false,
            press_point: None,
            dragging: false,
        })
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout(
            tree.state.downcast_mut::<State>(),
            renderer,
            limits,
            self.width,
            self.height,
            &self.spans,
            self.line_height,
            self.size,
            self.font,
            self.align_x,
            self.align_y,
            self.wrapping,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if !layout.bounds().intersects(viewport) {
            return;
        }

        let state = tree.state.downcast_ref::<State>();

        let style = Style {
            color: Some(defaults.text_color),
        };

        for (index, span) in self.spans.iter().enumerate() {
            if span.highlight.is_some() || span.underline || span.strikethrough {
                let translation = layout.position() - Point::ORIGIN;
                let regions = state.paragraph.span_bounds(index);

                if let Some(highlight) = span.highlight {
                    for bounds in &regions {
                        let bounds = Rectangle::new(
                            bounds.position() - Vector::new(span.padding.left, span.padding.top),
                            bounds.size() + Size::new(span.padding.x(), span.padding.y()),
                        );

                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: bounds + translation,
                                border: highlight.border,
                                ..Default::default()
                            },
                            highlight.background,
                        );
                    }
                }

                if span.underline || span.strikethrough {
                    let size = span.size.or(self.size).unwrap_or(renderer.default_size());

                    let line_height = span
                        .line_height
                        .unwrap_or(self.line_height)
                        .to_absolute(size);

                    let color = span.color.or(style.color).unwrap_or(defaults.text_color);

                    let baseline =
                        translation + Vector::new(0.0, size.0 + (line_height.0 - size.0) / 2.0);

                    if span.underline {
                        for bounds in &regions {
                            renderer.fill_quad(
                                renderer::Quad {
                                    bounds: Rectangle::new(
                                        bounds.position() + baseline
                                            - Vector::new(0.0, size.0 * 0.08),
                                        Size::new(bounds.width, 1.0),
                                    ),
                                    ..Default::default()
                                },
                                color,
                            );
                        }
                    }

                    if span.strikethrough {
                        for bounds in &regions {
                            renderer.fill_quad(
                                renderer::Quad {
                                    bounds: Rectangle::new(
                                        bounds.position() + baseline
                                            - Vector::new(0.0, size.0 / 2.0),
                                        Size::new(bounds.width, 1.0),
                                    ),
                                    ..Default::default()
                                },
                                color,
                            );
                        }
                    }
                }
            }
        }

        if let Some((start, end)) = self.local_selection(state, renderer, layout.bounds().size()) {
            let layout_state = self.selection_layout(state, renderer, layout.bounds().size());
            let translation = layout.position();
            let mut selection = theme.palette().primary;
            selection.a = if theme.extended_palette().is_dark {
                0.38
            } else {
                0.25
            };

            for bounds in selection_quads(layout_state, translation, start, end) {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        ..Default::default()
                    },
                    selection,
                );
            }
        }

        iced::widget::text::draw(
            renderer,
            defaults,
            layout.bounds(),
            &state.paragraph,
            style,
            viewport,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds().size();

        if self.order.is_some() {
            self.refresh_selected_text(state, renderer, bounds);
        }

        if !state.dragging && (self.on_link_click.is_some() || self.on_link_hover.is_some()) {
            let previous_hovered = state.hovered_link;

            if let Some(position) = cursor.position_in(layout.bounds()) {
                state.hovered_link = state.paragraph.hit_span(position).filter(|span| {
                    self.spans
                        .get(*span)
                        .is_some_and(|span| span.link.is_some())
                });
            } else {
                state.hovered_link = None;
            }

            if previous_hovered != state.hovered_link {
                if let Some(on_link_hover) = &self.on_link_hover {
                    let link = state
                        .hovered_link
                        .and_then(|index| self.spans.get(index))
                        .and_then(|span| span.link.clone());
                    // Report the top-center of the hovered span so hover
                    // popups are anchored above the link/totoz text instead
                    // of the pointer. This is only published when the
                    // hovered link changes, so the popup stays still while
                    // the pointer moves inside the same link.
                    let anchor = state
                        .hovered_link
                        .and_then(|index| span_anchor(&state.paragraph, layout, index))
                        .or_else(|| cursor.position())
                        .unwrap_or_else(|| layout.position());
                    shell.publish(on_link_hover(link, anchor));
                }
                shell.request_redraw();
            }
        }

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                state.button_down = true;

                if let Some(position) = cursor.position_in(layout.bounds()) {
                    if self.order.is_some() {
                        state.press_point = Some(position);
                        state.dragging = false;
                        shell.publish(Message::TextSelectionClear);
                    }

                    if state.hovered_link.is_some() {
                        state.span_pressed = state.hovered_link;
                        shell.capture_event();
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if !state.button_down {
                    return;
                }

                let Some(point) = cursor.position_in(layout.bounds()) else {
                    return;
                };

                if let Some(origin) = state.press_point {
                    if !state.dragging {
                        if distance_squared(origin, point) < 9.0 {
                            return;
                        }

                        let Some(anchor) = self.position_at(state, renderer, bounds, origin) else {
                            return;
                        };

                        state.dragging = true;
                        shell.publish(Message::TextSelectionStart(anchor));
                    }

                    if state.dragging {
                        if let Some(position) = self.position_at(state, renderer, bounds, point) {
                            if self
                                .selection
                                .is_none_or(|selection| selection.focus != position)
                            {
                                shell.publish(Message::TextSelectionUpdate(position));
                            }
                        }
                        shell.capture_event();
                        shell.request_redraw();
                    }
                } else if self.order.is_some()
                    && self.selection.is_some_and(|selection| selection.dragging)
                {
                    // Another paragraph started the drag: extend the
                    // selection when the pointer reaches this one.
                    if let Some(position) = self.position_at(state, renderer, bounds, point) {
                        if self
                            .selection
                            .is_some_and(|selection| selection.focus != position)
                        {
                            shell.publish(Message::TextSelectionUpdate(position));
                            shell.capture_event();
                            shell.request_redraw();
                        }
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.button_down = false;

                if state.dragging {
                    // Keep the click from reaching the post/clock handler.
                    shell.publish(Message::TextSelectionEnd);
                    shell.capture_event();
                } else {
                    match state.span_pressed {
                        Some(span) if Some(span) == state.hovered_link => {
                            if let Some(link) =
                                self.spans.get(span).and_then(|span| span.link.clone())
                            {
                                if let Some(on_link_clicked) = &self.on_link_click {
                                    shell.publish(on_link_clicked(link));
                                }
                            }
                        }
                        _ => {}
                    }
                }

                state.span_pressed = None;
                state.press_point = None;
                state.dragging = false;
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if state.hovered_link.is_some() {
                    state.span_right_pressed = state.hovered_link;
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)) => {
                if let Some(span) = state.span_right_pressed.take() {
                    if Some(span) == state.hovered_link {
                        if let Some(link) = self.spans.get(span).and_then(|span| span.link.clone())
                        {
                            if let Some(on_link_right_click) = &self.on_link_right_click {
                                shell.publish(on_link_right_click(link));
                            }
                        }
                    }

                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        state.order = self.order.unwrap_or_default();
        state.selected_text = self
            .local_selection(state, renderer, layout.bounds().size())
            .map(|(start, end)| {
                let layout_state = self.selection_layout(state, renderer, layout.bounds().size());
                selected_text(layout_state, start, end)
            });

        operation.custom(None, layout.bounds(), state);
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = _tree.state.downcast_ref::<State>();
        if state.hovered_link.is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }
}

fn distance_squared(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

/// Top-center of a span in content coordinates, used to anchor hover popups
/// above the link/totoz text.
fn span_anchor(paragraph: &ParagraphType, layout: Layout<'_>, index: usize) -> Option<Point> {
    let regions = paragraph.span_bounds(index);
    let first = regions.first()?;
    let mut left = first.x;
    let mut right = first.x + first.width;
    let mut top = first.y;
    for region in &regions[1..] {
        left = left.min(region.x);
        right = right.max(region.x + region.width);
        top = top.min(region.y);
    }
    Some(layout.position() + Vector::new((left + right) / 2.0, top))
}

#[allow(clippy::too_many_arguments)]
fn layout(
    state: &mut State,
    renderer: &Renderer,
    limits: &layout::Limits,
    width: Length,
    height: Length,
    spans: &[Span<'_, LinkAction>],
    line_height: LineHeight,
    size: Option<Pixels>,
    font: Option<<Renderer as text::Renderer>::Font>,
    align_x: Alignment,
    align_y: iced::alignment::Vertical,
    wrapping: Wrapping,
) -> layout::Node {
    layout::sized(limits, width, height, |limits| {
        let bounds = limits.max();

        let size = size.unwrap_or_else(|| renderer.default_size());
        let font = font.unwrap_or_else(|| renderer.default_font());

        let text_with_spans = || iced::advanced::Text {
            content: spans,
            bounds,
            size,
            line_height,
            font,
            align_x,
            align_y,
            shaping: Shaping::Advanced,
            wrapping,
        };

        if state.spans != spans {
            state.paragraph =
                <Renderer as text::Renderer>::Paragraph::with_spans(text_with_spans());
            state.spans = spans.iter().cloned().map(Span::to_static).collect();
            state.selection_layout = OnceCell::new();
            state.cached_selection = None;
        } else {
            match state.paragraph.compare(iced::advanced::Text {
                content: (),
                bounds,
                size,
                line_height,
                font,
                align_x,
                align_y,
                shaping: Shaping::Advanced,
                wrapping,
            }) {
                text::Difference::None => {}
                text::Difference::Bounds => {
                    state.paragraph.resize(bounds);
                    state.selection_layout = OnceCell::new();
                    state.cached_selection = None;
                }
                text::Difference::Shape => {
                    state.paragraph =
                        <Renderer as text::Renderer>::Paragraph::with_spans(text_with_spans());
                    state.selection_layout = OnceCell::new();
                    state.cached_selection = None;
                }
            }
        }

        state.paragraph.min_bounds()
    })
}

impl<'a> From<HoverRich<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(text: HoverRich<'a>) -> Element<'a, Message, Theme, Renderer> {
        Element::new(text)
    }
}
