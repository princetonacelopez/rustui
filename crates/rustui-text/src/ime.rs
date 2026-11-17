//! IME composition state, inline pre-edit underline ranges, and candidate window positioning.

use rustui_core::{Rect, Size};

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

    /// Cancels active pre-edit composition without committing text.
    pub fn cancel(&mut self) {
        self.preedit_text.clear();
        self.preedit_cursor = None;
    }

    /// Positions the candidate popup below the caret and flips/clamps near viewport edges.
    pub fn clamped_candidate_rect(&self, viewport: Size<f32>, popup_size: Size<f32>) -> Rect {
        let mut x = self.candidate_window_rect.x;
        let mut y = self.candidate_window_rect.max_y() + 4.0;
        if y + popup_size.height > viewport.height {
            y = (self.candidate_window_rect.y - popup_size.height - 4.0).max(0.0);
        }
        if x + popup_size.width > viewport.width {
            x = (viewport.width - popup_size.width).max(0.0);
        }
        Rect::new(x.max(0.0), y, popup_size.width, popup_size.height)
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
