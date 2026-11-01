//! Text input wrapper that reports caret movement to the application.
//!
//! iced's `text_input` emits the new value but not the caret position. Knowing
//! where the caret is lets clock quotes, totoz and emoji insertions land where
//! the user is actually typing instead of always at the end of the draft.

use iced::advanced::layout;
use iced::advanced::text;
use iced::advanced::widget::operation::TextInput as _;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::widget::text_input::{self, cursor, Value};
use iced::{Element, Event, Length, Rectangle, Size, Vector};

pub struct ComposerInput<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Renderer: text::Renderer,
{
    content: Element<'a, Message, Theme, Renderer>,
    cursor: usize,
    on_cursor: Box<dyn Fn(usize) -> Message + 'a>,
}

impl<'a, Message, Theme, Renderer> ComposerInput<'a, Message, Theme, Renderer>
where
    Renderer: text::Renderer,
{
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        cursor: usize,
        on_cursor: impl Fn(usize) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            cursor,
            on_cursor: Box::new(on_cursor),
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for ComposerInput<'_, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: text::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::stateless()
    }

    fn state(&self) -> tree::State {
        tree::State::None
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: iced::advanced::mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );

        let input = tree.children[0]
            .state
            .downcast_ref::<text_input::State<<Renderer as text::Renderer>::Paragraph>>();
        if !input.is_focused() {
            return;
        }
        let value = Value::new(input.text());
        let caret = match input.cursor().state(&value) {
            cursor::State::Index(index) => index,
            cursor::State::Selection { start, end } => start.min(end),
        };
        if caret != self.cursor {
            shell.publish((self.on_cursor)(caret));
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: iced::advanced::mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> iced::advanced::mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &iced::advanced::renderer::Style,
        layout: Layout<'_>,
        cursor: iced::advanced::mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<ComposerInput<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Theme: 'a,
    Renderer: 'a + text::Renderer,
{
    fn from(input: ComposerInput<'a, Message, Theme, Renderer>) -> Self {
        Element::new(input)
    }
}
