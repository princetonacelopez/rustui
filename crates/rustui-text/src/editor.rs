//! Rope-backed text editing buffer with grapheme-aware cursor, selection, and undo/redo grouping.

use ropey::{LineType, Rope};
use unicode_segmentation::UnicodeSegmentation;

/// Anchor/active byte selection range inside a text buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: usize,
    pub active: usize,
}

impl Selection {
    pub fn collapsed(pos: usize) -> Self {
        Self {
            anchor: pos,
            active: pos,
        }
    }

    pub fn range(anchor: usize, active: usize) -> Self {
        Self { anchor, active }
    }

    #[inline]
    pub fn start(self) -> usize {
        self.anchor.min(self.active)
    }

    #[inline]
    pub fn end(self) -> usize {
        self.anchor.max(self.active)
    }

    #[inline]
    pub fn is_collapsed(self) -> bool {
        self.anchor == self.active
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditRecord {
    pos: usize,
    deleted: String,
    inserted: String,
    before_sel: Selection,
    after_sel: Selection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UndoKind {
    Insert,
    Delete,
    Other,
}

#[derive(Debug, Clone)]
struct UndoEntry {
    kind: UndoKind,
    ops: Vec<EditRecord>,
}

/// Trait for custom text editors (`TextField`, `TextEditor`, and user editors).
pub trait EditableText {
    fn text(&self) -> String;
    fn set_text(&mut self, text: &str);
    fn len_bytes(&self) -> usize;
    fn selection(&self) -> Selection;
    fn set_selection(&mut self, sel: Selection);
    fn selected_text(&self) -> String;
    fn insert(&mut self, s: &str);
    fn delete_backward(&mut self);
    fn delete_forward(&mut self);
    fn move_left(&mut self, extend: bool);
    fn move_right(&mut self, extend: bool);
    fn move_word_left(&mut self, extend: bool);
    fn move_word_right(&mut self, extend: bool);
    fn move_home(&mut self, extend: bool);
    fn move_end(&mut self, extend: bool);
    fn move_up(&mut self, extend: bool);
    fn move_down(&mut self, extend: bool);
    fn select_all(&mut self);
    fn select_word_at(&mut self, byte_idx: usize);
    fn select_line_at(&mut self, byte_idx: usize);
    fn undo(&mut self) -> bool;
    fn redo(&mut self) -> bool;
}

/// Rope-backed editable text buffer with grapheme-aware navigation and coalesced undo/redo.
#[derive(Debug, Clone)]
pub struct TextBuffer {
    rope: Rope,
    selection: Selection,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    explicit_group_open: bool,
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new("")
    }
}

impl TextBuffer {
    pub fn new(initial: &str) -> Self {
        let rope = Rope::from_str(initial);
        let len = rope.len();
        Self {
            rope,
            selection: Selection::collapsed(len),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            explicit_group_open: false,
        }
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines(LineType::LF_CR)
    }

    pub fn begin_undo_group(&mut self) {
        self.explicit_group_open = true;
    }

    pub fn end_undo_group(&mut self) {
        self.explicit_group_open = false;
    }

    fn clamp_to_char_boundary(&self, idx: usize) -> usize {
        let s = self.rope.to_string();
        let mut c = idx.min(s.len());
        while c > 0 && !s.is_char_boundary(c) {
            c -= 1;
        }
        c
    }

    fn apply_edit(&mut self, start: usize, end: usize, insert_text: &str, kind: UndoKind) {
        let s = self.rope.to_string();
        let start = start.min(s.len());
        let end = end.min(s.len()).max(start);
        let deleted = s[start..end].to_owned();
        let before_sel = self.selection;

        if end > start {
            self.rope.remove(start..end);
        }
        if !insert_text.is_empty() {
            self.rope.insert(start, insert_text);
        }

        let new_pos = start + insert_text.len();
        self.selection = Selection::collapsed(new_pos);

        let record = EditRecord {
            pos: start,
            deleted,
            inserted: insert_text.to_owned(),
            before_sel,
            after_sel: self.selection,
        };
        self.redo_stack.clear();

        let can_coalesce = self.undo_stack.last().is_some_and(|last| {
            if self.explicit_group_open {
                return true;
            }
            if last.kind != kind || kind == UndoKind::Other {
                return false;
            }
            let Some(prev) = last.ops.last() else {
                return false;
            };
            match kind {
                UndoKind::Insert => {
                    !insert_text.contains('\n')
                        && !insert_text.chars().any(char::is_whitespace)
                        && prev.pos + prev.inserted.len() == start
                }
                UndoKind::Delete => end == prev.pos || start == prev.pos,
                UndoKind::Other => false,
            }
        });

        if can_coalesce {
            if let Some(last) = self.undo_stack.last_mut() {
                last.ops.push(record);
            }
        } else {
            self.undo_stack.push(UndoEntry {
                kind,
                ops: vec![record],
            });
        }
    }

    fn prev_grapheme_boundary(&self, byte_idx: usize) -> usize {
        let s = self.rope.to_string();
        let clamped = byte_idx.min(s.len());
        let mut last = 0usize;
        for (idx, _) in s[..clamped].grapheme_indices(true) {
            last = idx;
        }
        last
    }

    fn next_grapheme_boundary(&self, byte_idx: usize) -> usize {
        let s = self.rope.to_string();
        let clamped = byte_idx.min(s.len());
        if clamped >= s.len() {
            return s.len();
        }
        if let Some((_, g)) = s[clamped..].grapheme_indices(true).next() {
            clamped + g.len()
        } else {
            s.len()
        }
    }

    fn prev_word_boundary(&self, byte_idx: usize) -> usize {
        let s = self.rope.to_string();
        let clamped = byte_idx.min(s.len());
        let mut prev_start = 0usize;
        for (idx, word) in s[..clamped].split_word_bound_indices() {
            if !word.trim().is_empty() && idx < clamped {
                prev_start = idx;
            }
        }
        prev_start
    }

    fn next_word_boundary(&self, byte_idx: usize) -> usize {
        let s = self.rope.to_string();
        let clamped = byte_idx.min(s.len());
        for (idx, word) in s[clamped..].split_word_bound_indices() {
            let end = clamped + idx + word.len();
            if !word.trim().is_empty() && end > clamped {
                return end;
            }
        }
        s.len()
    }

    fn update_active(&mut self, pos: usize, extend: bool) {
        if extend {
            self.selection.active = pos;
        } else {
            self.selection = Selection::collapsed(pos);
        }
    }
}

impl EditableText for TextBuffer {
    fn text(&self) -> String {
        self.rope.to_string()
    }

    fn set_text(&mut self, text: &str) {
        if self.rope == text {
            return;
        }
        self.rope = Rope::from_str(text);
        let len = self.rope.len();
        self.selection = Selection::collapsed(self.selection.active.min(len));
    }

    fn len_bytes(&self) -> usize {
        self.rope.len()
    }

    fn selection(&self) -> Selection {
        self.selection
    }

    fn set_selection(&mut self, sel: Selection) {
        self.selection = Selection {
            anchor: self.clamp_to_char_boundary(sel.anchor),
            active: self.clamp_to_char_boundary(sel.active),
        };
    }

    fn selected_text(&self) -> String {
        let s = self.rope.to_string();
        let start = self.selection.start().min(s.len());
        let end = self.selection.end().min(s.len());
        s[start..end].to_owned()
    }

    fn insert(&mut self, s: &str) {
        let start = self.selection.start();
        let end = self.selection.end();
        self.apply_edit(start, end, s, UndoKind::Insert);
    }

    fn delete_backward(&mut self) {
        if !self.selection.is_collapsed() {
            self.apply_edit(
                self.selection.start(),
                self.selection.end(),
                "",
                UndoKind::Delete,
            );
            return;
        }
        if self.selection.active == 0 {
            return;
        }
        let prev = self.prev_grapheme_boundary(self.selection.active);
        self.apply_edit(prev, self.selection.active, "", UndoKind::Delete);
    }

    fn delete_forward(&mut self) {
        if !self.selection.is_collapsed() {
            self.apply_edit(
                self.selection.start(),
                self.selection.end(),
                "",
                UndoKind::Delete,
            );
            return;
        }
        let next = self.next_grapheme_boundary(self.selection.active);
        if next > self.selection.active {
            self.apply_edit(self.selection.active, next, "", UndoKind::Delete);
        }
    }

    fn move_left(&mut self, extend: bool) {
        if !extend && !self.selection.is_collapsed() {
            self.selection = Selection::collapsed(self.selection.start());
            return;
        }
        let prev = self.prev_grapheme_boundary(self.selection.active);
        self.update_active(prev, extend);
    }

    fn move_right(&mut self, extend: bool) {
        if !extend && !self.selection.is_collapsed() {
            self.selection = Selection::collapsed(self.selection.end());
            return;
        }
        let next = self.next_grapheme_boundary(self.selection.active);
        self.update_active(next, extend);
    }

    fn move_word_left(&mut self, extend: bool) {
        let pos = self.prev_word_boundary(self.selection.active);
        self.update_active(pos, extend);
    }

    fn move_word_right(&mut self, extend: bool) {
        let pos = self.next_word_boundary(self.selection.active);
        self.update_active(pos, extend);
    }

    fn move_home(&mut self, extend: bool) {
        let active = self.selection.active.min(self.rope.len());
        let line_idx = self.rope.byte_to_line_idx(active, LineType::LF_CR);
        let line_start_byte = self.rope.line_to_byte_idx(line_idx, LineType::LF_CR);
        self.update_active(line_start_byte, extend);
    }

    fn move_end(&mut self, extend: bool) {
        let active = self.selection.active.min(self.rope.len());
        let line_idx = self.rope.byte_to_line_idx(active, LineType::LF_CR);
        let line_str = self.rope.line(line_idx, LineType::LF_CR).to_string();
        let trimmed = line_str.trim_end_matches(['\n', '\r']);
        let line_start_byte = self.rope.line_to_byte_idx(line_idx, LineType::LF_CR);
        self.update_active(line_start_byte + trimmed.len(), extend);
    }

    fn move_up(&mut self, extend: bool) {
        let active_byte = self.selection.active.min(self.rope.len());
        let line_idx = self.rope.byte_to_line_idx(active_byte, LineType::LF_CR);
        if line_idx == 0 {
            self.update_active(0, extend);
            return;
        }
        let line_start = self.rope.line_to_byte_idx(line_idx, LineType::LF_CR);
        let col_bytes = active_byte.saturating_sub(line_start);
        let col_graphemes = self.rope.to_string()[line_start..line_start + col_bytes]
            .graphemes(true)
            .count();

        let prev_line = line_idx - 1;
        let prev_start = self.rope.line_to_byte_idx(prev_line, LineType::LF_CR);
        let prev_str = self.rope.line(prev_line, LineType::LF_CR).to_string();
        let prev_trimmed = prev_str.trim_end_matches(['\n', '\r']);
        let byte_offset: usize = prev_trimmed
            .graphemes(true)
            .take(col_graphemes)
            .map(|g| g.len())
            .sum();
        self.update_active(prev_start + byte_offset, extend);
    }

    fn move_down(&mut self, extend: bool) {
        let active_byte = self.selection.active.min(self.rope.len());
        let line_idx = self.rope.byte_to_line_idx(active_byte, LineType::LF_CR);
        if line_idx + 1 >= self.rope.len_lines(LineType::LF_CR) {
            self.update_active(self.rope.len(), extend);
            return;
        }
        let line_start = self.rope.line_to_byte_idx(line_idx, LineType::LF_CR);
        let col_bytes = active_byte.saturating_sub(line_start);
        let col_graphemes = self.rope.to_string()[line_start..line_start + col_bytes]
            .graphemes(true)
            .count();

        let next_line = line_idx + 1;
        let next_start = self.rope.line_to_byte_idx(next_line, LineType::LF_CR);
        let next_str = self.rope.line(next_line, LineType::LF_CR).to_string();
        let next_trimmed = next_str.trim_end_matches(['\n', '\r']);
        let byte_offset: usize = next_trimmed
            .graphemes(true)
            .take(col_graphemes)
            .map(|g| g.len())
            .sum();
        self.update_active(next_start + byte_offset, extend);
    }

    fn select_all(&mut self) {
        self.selection = Selection::range(0, self.rope.len());
    }

    fn select_word_at(&mut self, byte_idx: usize) {
        let s = self.rope.to_string();
        let idx = byte_idx.min(s.len());
        for (start, word) in s.split_word_bound_indices() {
            let end = start + word.len();
            if idx >= start && idx <= end && !word.trim().is_empty() {
                self.selection = Selection::range(start, end);
                return;
            }
        }
        self.selection = Selection::collapsed(idx);
    }

    fn select_line_at(&mut self, byte_idx: usize) {
        let clamped = byte_idx.min(self.rope.len());
        let line_idx = self.rope.byte_to_line_idx(clamped, LineType::LF_CR);
        let start = self.rope.line_to_byte_idx(line_idx, LineType::LF_CR);
        let line_str = self.rope.line(line_idx, LineType::LF_CR).to_string();
        let trimmed_len = line_str.trim_end_matches(['\n', '\r']).len();
        self.selection = Selection::range(start, start + trimmed_len);
    }

    fn undo(&mut self) -> bool {
        let Some(entry) = self.undo_stack.pop() else {
            return false;
        };
        for op in entry.ops.iter().rev() {
            let end = (op.pos + op.inserted.len()).min(self.rope.len());
            if end > op.pos {
                self.rope.remove(op.pos..end);
            }
            if !op.deleted.is_empty() {
                self.rope.insert(op.pos, &op.deleted);
            }
            self.selection = op.before_sel;
        }
        self.redo_stack.push(entry);
        true
    }

    fn redo(&mut self) -> bool {
        let Some(entry) = self.redo_stack.pop() else {
            return false;
        };
        for op in &entry.ops {
            let end = (op.pos + op.deleted.len()).min(self.rope.len());
            if end > op.pos {
                self.rope.remove(op.pos..end);
            }
            if !op.inserted.is_empty() {
                self.rope.insert(op.pos, &op.inserted);
            }
            self.selection = op.after_sel;
        }
        self.undo_stack.push(entry);
        true
    }
}
