// Adapted from `iced_widget::text::rich` (MIT, © the iced contributors).
// The only functional change is an `on_link_hover` callback, which lets the
// application highlight the post referenced by a norloge while hovering it.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::advanced::renderer::Renderer as _;
use iced::advanced::text::Renderer as _;
use iced::advanced::text::{self, Paragraph, Span};
use iced::advanced::widget::text::{Alignment, LineHeight, Shaping, Style, Wrapping};
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{Element, Event, Length, Pixels, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::app::{LinkAction, Message};

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
    on_link_hover: Option<HoverFn<'a>>,
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
            on_link_hover: None,
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

    pub fn on_link_hover(
        mut self,
        on_link_hover: impl Fn(Option<LinkAction>, Point) -> Message + 'a,
    ) -> Self {
        self.on_link_hover = Some(Box::new(on_link_hover));
        self
    }
}

struct State {
    spans: Vec<Span<'static, LinkAction>>,
    span_pressed: Option<usize>,
    hovered_link: Option<usize>,
    paragraph: ParagraphType,
}

impl Widget<Message, Theme, Renderer> for HoverRich<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            spans: Vec::new(),
            span_pressed: None,
            hovered_link: None,
            paragraph: ParagraphType::default(),
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

        let _ = theme;
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
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        if self.on_link_click.is_none() && self.on_link_hover.is_none() {
            return;
        }

        let state = tree.state.downcast_mut::<State>();
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
                // Report the pointer position so hover popups (totoz images)
                // can be anchored right above the cursor. This is only
                // published when the hovered link changes, so the popup stays
                // still while the pointer moves inside the same link.
                let anchor = cursor.position().unwrap_or_else(|| layout.position());
                shell.publish(on_link_hover(link, anchor));
            }
            shell.request_redraw();
        }

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if state.hovered_link.is_some() {
                    state.span_pressed = state.hovered_link;
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                match state.span_pressed {
                    Some(span) if Some(span) == state.hovered_link => {
                        if let Some(link) = self.spans.get(span).and_then(|span| span.link.clone())
                        {
                            if let Some(on_link_clicked) = &self.on_link_click {
                                shell.publish(on_link_clicked(link));
                            }
                        }
                    }
                    _ => {}
                }

                state.span_pressed = None;
            }
            _ => {}
        }
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
                }
                text::Difference::Shape => {
                    state.paragraph =
                        <Renderer as text::Renderer>::Paragraph::with_spans(text_with_spans());
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
