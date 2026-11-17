//! Rope-backed text editing buffer with grapheme-aware cursor, selection, and undo/redo grouping.

use std::collections::HashMap;

use ropey::{LineType, Rope};
use rustui_core::Color;
use unicode_segmentation::UnicodeSegmentation;

/// Persistent text buffer marker surviving insertions and deletions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnchorId(u64);

/// Affinity of an anchor when text is inserted exactly at its offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnchorBias {
    Left,
    #[default]
    Right,
}

#[derive(Debug, Clone, Copy)]
struct AnchorEntry {
    pos: usize,
    bias: AnchorBias,
}

/// Visual decoration kind for `EditorSurface` overlays and gutters.
#[derive(Debug, Clone, PartialEq)]
pub enum DecorationKind {
    SquiggleError,
    SquiggleWarning,
    SquiggleInfo,
    BackgroundHighlight(Color),
    InlayHint { label: String },
    GutterMarker { symbol: char, color: Color },
}

/// Byte-range decoration (diagnostic squiggle, highlight, inlay hint, or gutter badge).
#[derive(Debug, Clone, PartialEq)]
pub struct EditorDecoration {
    pub start_byte: usize,
    pub end_byte: usize,
    pub kind: DecorationKind,
    pub message: Option<String>,
}

/// Collapsible code folding range in 0-based line indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldRange {
    pub start_line: usize,
    pub end_line: usize,
    pub collapsed: bool,
}

/// Syntax highlighting color span over byte offsets in a source buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyntaxTokenSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub color: Color,
}

/// Git line diff classification shown in the `EditorSurface` gutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GutterDiffKind {
    Added,
    Modified,
    Deleted,
}

/// Single line diff marker in the editor gutter (0-based line index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineDiffMarker {
    pub line: usize,
    pub kind: GutterDiffKind,
}

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

/// Rope-backed editable text buffer with multi-cursor support, persistent anchors,
/// UTF-16 conversion, grapheme-aware navigation, and coalesced undo/redo.
#[derive(Debug, Clone)]
pub struct TextBuffer {
    rope: Rope,
    selection: Selection,
    extra_selections: Vec<Selection>,
    anchors: HashMap<AnchorId, AnchorEntry>,
    next_anchor_id: u64,
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
            extra_selections: Vec::new(),
            anchors: HashMap::new(),
            next_anchor_id: 1,
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

    /// Returns the content of 0-based `line_idx` without trailing newline characters.
    pub fn line_str(&self, line_idx: usize) -> String {
        if line_idx >= self.line_count() {
            return String::new();
        }
        self.rope
            .line(line_idx, LineType::LF_CR)
            .to_string()
            .trim_end_matches(['\n', '\r'])
            .to_owned()
    }

    /// Creates a persistent anchor surviving edits according to `bias`.
    pub fn create_anchor(&mut self, byte_offset: usize, bias: AnchorBias) -> AnchorId {
        let pos = self.clamp_to_char_boundary(byte_offset);
        let id = AnchorId(self.next_anchor_id);
        self.next_anchor_id = self.next_anchor_id.wrapping_add(1);
        self.anchors.insert(id, AnchorEntry { pos, bias });
        id
    }

    /// Resolves the current byte offset of an anchor.
    pub fn resolve_anchor(&self, id: AnchorId) -> Option<usize> {
        self.anchors.get(&id).map(|a| a.pos.min(self.rope.len()))
    }

    /// Removes a previously created anchor.
    pub fn remove_anchor(&mut self, id: AnchorId) {
        self.anchors.remove(&id);
    }

    /// Returns all active selections (primary first, followed by secondary cursors).
    pub fn all_selections(&self) -> Vec<Selection> {
        let mut out = Vec::with_capacity(1 + self.extra_selections.len());
        out.push(self.selection);
        out.extend_from_slice(&self.extra_selections);
        out
    }

    /// Replaces the full cursor list (`primary` plus any secondary selections).
    pub fn set_all_selections(&mut self, mut sels: Vec<Selection>) {
        if sels.is_empty() {
            self.extra_selections.clear();
            return;
        }
        for s in &mut sels {
            s.anchor = self.clamp_to_char_boundary(s.anchor);
            s.active = self.clamp_to_char_boundary(s.active);
        }
        self.selection = sels[0];
        self.extra_selections = sels.into_iter().skip(1).collect();
    }

    /// Removes all secondary cursors, leaving only the primary selection.
    pub fn clear_secondary_cursors(&mut self) {
        self.extra_selections.clear();
    }

    /// Adds a secondary collapsed cursor at `byte_idx`.
    pub fn add_cursor_at(&mut self, byte_idx: usize) {
        let pos = self.clamp_to_char_boundary(byte_idx);
        let sel = Selection::collapsed(pos);
        if sel != self.selection && !self.extra_selections.contains(&sel) {
            self.extra_selections.push(sel);
        }
    }

    /// Adds a cursor one visual/logical line above the primary cursor preserving column.
    pub fn add_cursor_above(&mut self) {
        let (line, col) = self.byte_to_line_col(self.selection.active);
        if line > 0 {
            let pos = self.line_col_to_byte(line - 1, col);
            self.add_cursor_at(pos);
        }
    }

    /// Adds a cursor one line below the lowest cursor preserving column.
    pub fn add_cursor_below(&mut self) {
        let last_active = self
            .extra_selections
            .iter()
            .chain(std::iter::once(&self.selection))
            .map(|s| s.active)
            .max()
            .unwrap_or(self.selection.active);
        let (line, col) = self.byte_to_line_col(last_active);
        if line + 1 < self.line_count() {
            let pos = self.line_col_to_byte(line + 1, col);
            self.add_cursor_at(pos);
        }
    }

    /// Adds a selection covering the next occurrence of the primary selected text (or word under cursor).
    pub fn add_cursor_next_match(&mut self) -> bool {
        if self.selection.is_collapsed() {
            let pos = self.selection.active;
            self.select_word_at(pos);
            return !self.selection.is_collapsed();
        }
        let needle = self.selected_text();
        if needle.is_empty() {
            return false;
        }
        let s = self.rope.to_string();
        let search_from = self
            .extra_selections
            .iter()
            .chain(std::iter::once(&self.selection))
            .map(|sel| sel.end())
            .max()
            .unwrap_or(self.selection.end())
            .min(s.len());
        if let Some(rel) = s[search_from..].find(&needle) {
            let start = search_from + rel;
            let sel = Selection::range(start, start + needle.len());
            if sel != self.selection && !self.extra_selections.contains(&sel) {
                self.extra_selections.push(sel);
                return true;
            }
        } else if let Some(rel) = s[..self.selection.start().min(s.len())].find(&needle) {
            let sel = Selection::range(rel, rel + needle.len());
            if sel != self.selection && !self.extra_selections.contains(&sel) {
                self.extra_selections.push(sel);
                return true;
            }
        }
        false
    }

    /// Creates a rectangular column selection spanning lines `start_line..=end_line` across char columns `col_start..col_end`.
    pub fn set_column_selection(
        &mut self,
        start_line: usize,
        end_line: usize,
        col_start: usize,
        col_end: usize,
    ) {
        let lo = start_line.min(end_line);
        let hi = start_line
            .max(end_line)
            .min(self.line_count().saturating_sub(1));
        let mut sels = Vec::new();
        for line in lo..=hi {
            let a = self.line_col_to_byte(line, col_start);
            let b = self.line_col_to_byte(line, col_end);
            sels.push(Selection::range(a, b));
        }
        self.set_all_selections(sels);
    }

    /// Converts a UTF-8 byte offset to `(0-based line, 0-based Unicode char column)`.
    pub fn byte_to_line_col(&self, byte_idx: usize) -> (usize, usize) {
        let clamped = self.clamp_to_char_boundary(byte_idx);
        let line = self.rope.byte_to_line_idx(clamped, LineType::LF_CR);
        let line_start = self.rope.line_to_byte_idx(line, LineType::LF_CR);
        let col = self.rope.slice(line_start..clamped).chars().count();
        (line, col)
    }

    /// Converts `(0-based line, 0-based Unicode char column)` to a UTF-8 byte offset.
    pub fn line_col_to_byte(&self, line_idx: usize, col_chars: usize) -> usize {
        let total_lines = self.line_count().max(1);
        let line = line_idx.min(total_lines - 1);
        let line_start = self.rope.line_to_byte_idx(line, LineType::LF_CR);
        let line_text = self.line_str(line);
        let offset: usize = line_text.chars().take(col_chars).map(char::len_utf8).sum();
        line_start + offset
    }

    /// Converts a UTF-8 byte offset to `(0-based line, 0-based UTF-16 code-unit column)` for LSP.
    pub fn byte_to_utf16_line_col(&self, byte_idx: usize) -> (u32, u32) {
        let clamped = self.clamp_to_char_boundary(byte_idx);
        let line = self.rope.byte_to_line_idx(clamped, LineType::LF_CR);
        let line_start = self.rope.line_to_byte_idx(line, LineType::LF_CR);
        let utf16_col: usize = self
            .rope
            .slice(line_start..clamped)
            .chars()
            .map(char::len_utf16)
            .sum();
        (line as u32, utf16_col as u32)
    }

    /// Converts an LSP `(0-based line, 0-based UTF-16 code-unit column)` back to a UTF-8 byte offset.
    pub fn utf16_line_col_to_byte(&self, line_idx: u32, col_utf16: u32) -> usize {
        let total_lines = self.line_count().max(1);
        let line = (line_idx as usize).min(total_lines - 1);
        let line_start = self.rope.line_to_byte_idx(line, LineType::LF_CR);
        let line_text = self.line_str(line);
        let mut units = 0u32;
        let mut bytes = 0usize;
        for ch in line_text.chars() {
            let u = ch.len_utf16() as u32;
            if units + u > col_utf16 {
                break;
            }
            units += u;
            bytes += ch.len_utf8();
        }
        line_start + bytes
    }

    /// Finds the matching `()`, `[]`, or `{}` byte offset for the delimiter at or just before `byte_idx`.
    pub fn find_matching_bracket(&self, byte_idx: usize) -> Option<usize> {
        let len = self.rope.len();
        let byte_at = |i: usize| self.rope.byte(i);
        let probe = if byte_idx < len
            && matches!(byte_at(byte_idx), b'(' | b')' | b'[' | b']' | b'{' | b'}')
        {
            byte_idx
        } else if byte_idx > 0
            && byte_idx <= len
            && matches!(
                byte_at(byte_idx - 1),
                b'(' | b')' | b'[' | b']' | b'{' | b'}'
            )
        {
            byte_idx - 1
        } else {
            return None;
        };

        let (open, close, forward) = match byte_at(probe) {
            b'(' => (b'(', b')', true),
            b'[' => (b'[', b']', true),
            b'{' => (b'{', b'}', true),
            b')' => (b'(', b')', false),
            b']' => (b'[', b']', false),
            b'}' => (b'{', b'}', false),
            _ => return None,
        };

        let mut depth = 0i32;
        if forward {
            let max_scan = (probe + 512_000).min(len);
            for (rel, b) in self.rope.bytes_at(probe).take(max_scan - probe).enumerate() {
                if b == open {
                    depth += 1;
                } else if b == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(probe + rel);
                    }
                }
            }
        } else {
            let min_scan = probe.saturating_sub(512_000);
            let mut idx = probe;
            for b in self.rope.bytes_at(probe + 1).reversed() {
                if b == close {
                    depth += 1;
                } else if b == open {
                    depth -= 1;
                    if depth == 0 {
                        return Some(idx);
                    }
                }
                if idx <= min_scan {
                    break;
                }
                idx -= 1;
            }
        }
        None
    }

    /// Indents all lines intersected by the primary selection with 4 spaces.
    pub fn indent_selection(&mut self) {
        let (l0, _) = self.byte_to_line_col(self.selection.start());
        let (l1, _) = self.byte_to_line_col(self.selection.end());
        self.begin_undo_group();
        for line in (l0..=l1).rev() {
            let byte = self.line_col_to_byte(line, 0);
            self.apply_edit(byte, byte, "    ", UndoKind::Other);
        }
        self.end_undo_group();
    }

    /// Outdents up to 4 leading spaces or one tab on lines intersected by the primary selection.
    pub fn outdent_selection(&mut self) {
        let (l0, _) = self.byte_to_line_col(self.selection.start());
        let (l1, _) = self.byte_to_line_col(self.selection.end());
        self.begin_undo_group();
        for line in (l0..=l1).rev() {
            let text = self.line_str(line);
            let remove_bytes = if text.starts_with('\t') {
                1
            } else {
                text.bytes().take(4).take_while(|&b| b == b' ').count()
            };
            if remove_bytes > 0 {
                let byte = self.line_col_to_byte(line, 0);
                self.apply_edit(byte, byte + remove_bytes, "", UndoKind::Other);
            }
        }
        self.end_undo_group();
    }

    pub fn begin_undo_group(&mut self) {
        self.explicit_group_open = true;
    }

    pub fn end_undo_group(&mut self) {
        self.explicit_group_open = false;
    }

    fn clamp_to_char_boundary(&self, idx: usize) -> usize {
        let len = self.rope.len();
        let mut c = idx.min(len);
        while c > 0 && c < len && (self.rope.byte(c) & 0xC0) == 0x80 {
            c -= 1;
        }
        c
    }

    fn ceil_char_boundary(&self, idx: usize) -> usize {
        let len = self.rope.len();
        let mut c = idx.min(len);
        while c < len && (self.rope.byte(c) & 0xC0) == 0x80 {
            c += 1;
        }
        c
    }

    fn shift_anchors_for_edit(&mut self, start: usize, deleted_len: usize, inserted_len: usize) {
        for entry in self.anchors.values_mut() {
            entry.pos = if entry.pos < start {
                entry.pos
            } else if entry.pos > start + deleted_len {
                (entry.pos - deleted_len) + inserted_len
            } else {
                match entry.bias {
                    AnchorBias::Left => start,
                    AnchorBias::Right => start + inserted_len,
                }
            };
        }
    }

    fn apply_edit(&mut self, start: usize, end: usize, insert_text: &str, kind: UndoKind) {
        let len = self.rope.len();
        let start = self.clamp_to_char_boundary(start.min(len));
        let end = self.clamp_to_char_boundary(end.min(len)).max(start);
        let deleted = if end > start {
            self.rope.slice(start..end).to_string()
        } else {
            String::new()
        };
        let before_sel = self.selection;

        if end > start {
            self.rope.remove(start..end);
        }
        if !insert_text.is_empty() {
            self.rope.insert(start, insert_text);
        }

        self.shift_anchors_for_edit(start, end - start, insert_text.len());
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
        let clamped = self.clamp_to_char_boundary(byte_idx);
        let win_start = self.clamp_to_char_boundary(clamped.saturating_sub(256));
        let win = self.rope.slice(win_start..clamped).to_string();
        let mut last = win_start;
        for (idx, _) in win.grapheme_indices(true) {
            last = win_start + idx;
        }
        last
    }

    fn next_grapheme_boundary(&self, byte_idx: usize) -> usize {
        let len = self.rope.len();
        let clamped = self.clamp_to_char_boundary(byte_idx);
        if clamped >= len {
            return len;
        }
        let win_end = self.ceil_char_boundary((clamped + 256).min(len));
        let win = self.rope.slice(clamped..win_end).to_string();
        if let Some((_, g)) = win.grapheme_indices(true).next() {
            clamped + g.len()
        } else {
            len
        }
    }

    fn prev_word_boundary(&self, byte_idx: usize) -> usize {
        let clamped = self.clamp_to_char_boundary(byte_idx);
        let win_start = self.clamp_to_char_boundary(clamped.saturating_sub(512));
        let win = self.rope.slice(win_start..clamped).to_string();
        let mut prev_start = win_start;
        for (idx, word) in win.split_word_bound_indices() {
            if !word.trim().is_empty() && win_start + idx < clamped {
                prev_start = win_start + idx;
            }
        }
        prev_start
    }

    fn next_word_boundary(&self, byte_idx: usize) -> usize {
        let len = self.rope.len();
        let clamped = self.clamp_to_char_boundary(byte_idx);
        let win_end = self.ceil_char_boundary((clamped + 512).min(len));
        let win = self.rope.slice(clamped..win_end).to_string();
        for (idx, word) in win.split_word_bound_indices() {
            let end = clamped + idx + word.len();
            if !word.trim().is_empty() && end > clamped {
                return end;
            }
        }
        len
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
        let len = self.rope.len();
        let start = self.clamp_to_char_boundary(self.selection.start().min(len));
        let end = self
            .clamp_to_char_boundary(self.selection.end().min(len))
            .max(start);
        self.rope.slice(start..end).to_string()
    }

    fn insert(&mut self, s: &str) {
        if self.extra_selections.is_empty() {
            let start = self.selection.start();
            let end = self.selection.end();
            self.apply_edit(start, end, s, UndoKind::Insert);
            return;
        }
        let mut indexed: Vec<(usize, Selection)> =
            self.all_selections().into_iter().enumerate().collect();
        indexed.sort_by_key(|(_, sel)| sel.start());
        let mut updated = vec![Selection::default(); indexed.len()];
        let mut cum_delta: isize = 0;
        self.begin_undo_group();
        for (orig_idx, sel) in indexed {
            let start = (sel.start() as isize + cum_delta).max(0) as usize;
            let end = (sel.end() as isize + cum_delta).max(0) as usize;
            self.apply_edit(start, end, s, UndoKind::Insert);
            updated[orig_idx] = Selection::collapsed(start + s.len());
            cum_delta += s.len() as isize - (end - start) as isize;
        }
        self.end_undo_group();
        self.selection = updated[0];
        self.extra_selections = updated.into_iter().skip(1).collect();
    }

    fn delete_backward(&mut self) {
        if self.extra_selections.is_empty() {
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
            return;
        }
        let mut indexed: Vec<(usize, Selection)> =
            self.all_selections().into_iter().enumerate().collect();
        indexed.sort_by_key(|(_, sel)| sel.start());
        let mut updated = vec![Selection::default(); indexed.len()];
        let mut cum_delta: isize = 0;
        self.begin_undo_group();
        for (orig_idx, sel) in indexed {
            let start = (sel.start() as isize + cum_delta).max(0) as usize;
            let end = (sel.end() as isize + cum_delta).max(0) as usize;
            if start != end {
                self.apply_edit(start, end, "", UndoKind::Delete);
                updated[orig_idx] = Selection::collapsed(start);
                cum_delta -= (end - start) as isize;
            } else if start > 0 {
                let prev = self.prev_grapheme_boundary(start);
                self.apply_edit(prev, start, "", UndoKind::Delete);
                updated[orig_idx] = Selection::collapsed(prev);
                cum_delta -= (start - prev) as isize;
            } else {
                updated[orig_idx] = Selection::collapsed(0);
            }
        }
        self.end_undo_group();
        self.selection = updated[0];
        self.extra_selections = updated.into_iter().skip(1).collect();
    }

    fn delete_forward(&mut self) {
        if self.extra_selections.is_empty() {
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
            return;
        }
        let mut indexed: Vec<(usize, Selection)> =
            self.all_selections().into_iter().enumerate().collect();
        indexed.sort_by_key(|(_, sel)| sel.start());
        let mut updated = vec![Selection::default(); indexed.len()];
        let mut cum_delta: isize = 0;
        self.begin_undo_group();
        for (orig_idx, sel) in indexed {
            let start = (sel.start() as isize + cum_delta).max(0) as usize;
            let end = (sel.end() as isize + cum_delta).max(0) as usize;
            if start != end {
                self.apply_edit(start, end, "", UndoKind::Delete);
                updated[orig_idx] = Selection::collapsed(start);
                cum_delta -= (end - start) as isize;
            } else {
                let next = self.next_grapheme_boundary(start);
                if next > start {
                    self.apply_edit(start, next, "", UndoKind::Delete);
                    updated[orig_idx] = Selection::collapsed(start);
                    cum_delta -= (next - start) as isize;
                } else {
                    updated[orig_idx] = Selection::collapsed(start);
                }
            }
        }
        self.end_undo_group();
        self.selection = updated[0];
        self.extra_selections = updated.into_iter().skip(1).collect();
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
        let col_prefix = self
            .rope
            .slice(line_start..line_start + col_bytes)
            .to_string();
        let col_graphemes = col_prefix.graphemes(true).count();

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
        let col_prefix = self
            .rope
            .slice(line_start..line_start + col_bytes)
            .to_string();
        let col_graphemes = col_prefix.graphemes(true).count();

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
        let clamped = self.clamp_to_char_boundary(byte_idx);
        let line_idx = self.rope.byte_to_line_idx(clamped, LineType::LF_CR);
        let line_start = self.rope.line_to_byte_idx(line_idx, LineType::LF_CR);
        let line_str = self.rope.line(line_idx, LineType::LF_CR).to_string();
        let rel = clamped.saturating_sub(line_start).min(line_str.len());
        for (start, word) in line_str.split_word_bound_indices() {
            let end = start + word.len();
            if rel >= start && rel <= end && !word.trim().is_empty() {
                self.selection = Selection::range(line_start + start, line_start + end);
                return;
            }
        }
        self.selection = Selection::collapsed(clamped);
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
            self.shift_anchors_for_edit(op.pos, end.saturating_sub(op.pos), op.deleted.len());
            self.selection = op.before_sel;
        }
        self.extra_selections.clear();
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
            self.shift_anchors_for_edit(op.pos, end.saturating_sub(op.pos), op.inserted.len());
            self.selection = op.after_sel;
        }
        self.extra_selections.clear();
        self.undo_stack.push(entry);
        true
    }
}
