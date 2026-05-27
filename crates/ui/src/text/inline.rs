use gpui::SharedString;

use crate::input::Selection;

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
