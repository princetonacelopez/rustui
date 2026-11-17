//! IME composition state, inline pre-edit underline ranges, and candidate window positioning.

use rustui_core::Rect;

use crate::editor::EditableText;

/// Tracks active IME pre-edit composition and candidate window anchor rect.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImeState {
    pub enabled: bool,
    pub preedit_text: String,
    pub preedit_cursor: Option<(usize, usize)>,
    pub candidate_window_rect: Rect,
}

impl ImeState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enable(&mut self, caret_rect: Rect) {
        self.enabled = true;
        self.candidate_window_rect = caret_rect;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
        self.preedit_text.clear();
        self.preedit_cursor = None;
    }

    /// Updates inline pre-edit string and candidate anchor position from the caret rect.
    pub fn set_preedit(&mut self, text: String, cursor: Option<(usize, usize)>, caret_rect: Rect) {
        self.enabled = true;
        self.preedit_text = text;
        self.preedit_cursor = cursor;
        self.candidate_window_rect = caret_rect;
    }

    /// Commits IME text into `editor` and clears active composition state.
    pub fn commit(&mut self, committed: &str, editor: &mut impl EditableText) {
        self.preedit_text.clear();
        self.preedit_cursor = None;
        if !committed.is_empty() {
            editor.insert(committed);
        }
    }

    pub fn is_composing(&self) -> bool {
        !self.preedit_text.is_empty()
    }

    /// Combines committed buffer text with active pre-edit string for inline display.
    pub fn compose_display_string(&self, base_text: &str, caret_byte: usize) -> String {
        if self.preedit_text.is_empty() {
            return base_text.to_owned();
        }
        let pos = caret_byte.min(base_text.len());
        let mut out = String::with_capacity(base_text.len() + self.preedit_text.len());
        out.push_str(&base_text[..pos]);
        out.push_str(&self.preedit_text);
        out.push_str(&base_text[pos..]);
        out
    }
}
