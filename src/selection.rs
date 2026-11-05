//! Plain-text selection shared by the timeline and the custom text widget.
//!
//! A position is expressed in the logical lines of a rendered paragraph. The
//! `order` field identifies the widget in the timeline so a drag can start in
//! one post and end in another. Keeping the position in line/byte form makes
//! it independent from font shaping and from the viewport scroll offset.

/// A caret position inside one selectable text widget.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPosition {
    /// Document order of the widget (timeline posts and fields).
    pub order: u64,
    /// Logical line in the paragraph.
    pub line: usize,
    /// Byte offset in that line.
    pub index: usize,
}

/// An anchor/focus pair. The focus moves while the pointer is dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: TextPosition,
    pub focus: TextPosition,
    /// True while the left button is held after the drag threshold.
    pub dragging: bool,
}

impl TextSelection {
    /// Returns the two ends in document order, whichever way the drag went.
    pub fn ordered(self) -> (TextPosition, TextPosition) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }
}
