//! Lower-level extensible code editor surface (`EditorSurface`) for IDEs and source editors.
//!
//! Features rope-backed storage, multi-cursor editing, persistent anchors, syntax highlight spans,
//! diagnostic squiggles, inline inlay hints, code folding, git gutter diff markers, breakpoints,
//! matching bracket highlights, and an optional minimap.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use rustui_core::{
    AccessibilityRole, Alignment, Binding, BuildCx, Color, ContainerKind, CornerRadii, Cx,
    DirtyFlags, EmptyView, Frame, HorizontalAlignment, KeyCode, NodeId, NodeKind, Point, Primitive,
    Rect, Size, TextNodeData, TextSpanData, View,
};
use rustui_layout::{Children, CustomLayoutBox, Layout};
use rustui_platform::{DesktopPlatform, Platform};
use rustui_render::{DisplayList, PathOp};
use rustui_text::{
    DecorationKind, EditableText, EditorDecoration, FoldRange, GutterDiffKind, ImeState,
    LineDiffMarker, Selection, SyntaxTokenSpan, TextBuffer, TextSystem,
};

use crate::theme::{Font, Theme};

fn resolve_theme(cx: &BuildCx) -> Theme {
    cx.env::<Theme>().unwrap_or_default()
}

/// Complete reactive buffer, decoration, folding, diff, and cursor model driving [`EditorSurface`].
#[derive(Debug, Clone)]
pub struct EditorBufferState {
    pub buffer: TextBuffer,
    pub syntax_spans: Vec<SyntaxTokenSpan>,
    pub decorations: Vec<EditorDecoration>,
    pub folds: Vec<FoldRange>,
    pub line_diffs: Vec<LineDiffMarker>,
    pub breakpoints: BTreeSet<usize>,
    pub scroll_line: usize,
    pub show_minimap: bool,
    pub ime: ImeState,
}

impl Default for EditorBufferState {
    fn default() -> Self {
        Self::from_text("")
    }
}

impl EditorBufferState {
    pub fn from_text(text: &str) -> Self {
        let mut buf = TextBuffer::new(text);
        buf.set_selection(Selection::collapsed(0));
        Self {
            buffer: buf,
            syntax_spans: Vec::new(),
            decorations: Vec::new(),
            folds: Vec::new(),
            line_diffs: Vec::new(),
            breakpoints: BTreeSet::new(),
            scroll_line: 0,
            show_minimap: true,
            ime: ImeState::new(),
        }
    }

    pub fn text(&self) -> String {
        self.buffer.text()
    }

    pub fn cursor_line_col(&self) -> (usize, usize) {
        self.buffer.byte_to_line_col(self.buffer.selection().active)
    }

    pub fn cursor_count(&self) -> usize {
        self.buffer.all_selections().len()
    }

    pub fn status_summary(&self) -> String {
        let (ln, col) = self.cursor_line_col();
        let n = self.cursor_count();
        if n > 1 {
            format!("Ln {}, Col {} ({n} cursors)", ln + 1, col + 1)
        } else {
            format!("Ln {}, Col {}", ln + 1, col + 1)
        }
    }

    pub fn toggle_breakpoint(&mut self, line: usize) {
        if !self.breakpoints.insert(line) {
            self.breakpoints.remove(&line);
        }
    }

    pub fn toggle_fold(&mut self, start_line: usize) {
        if let Some(f) = self.folds.iter_mut().find(|f| f.start_line == start_line) {
            f.collapsed = !f.collapsed;
        }
    }

    pub fn jump_to_line_col(&mut self, line: usize, col: usize) {
        let byte = self.buffer.line_col_to_byte(line, col);
        self.buffer.clear_secondary_cursors();
        self.buffer.set_selection(Selection::collapsed(byte));
        if line < self.scroll_line {
            self.scroll_line = line;
        } else if line > self.scroll_line + 14 {
            self.scroll_line = line.saturating_sub(6);
        }
    }

    pub fn visible_lines(&self) -> Vec<usize> {
        let total = self.buffer.line_count().max(1);
        let mut hidden_ranges: Vec<(usize, usize)> = self
            .folds
            .iter()
            .filter(|f| f.collapsed && f.end_line > f.start_line)
            .map(|f| (f.start_line + 1, f.end_line))
            .collect();
        hidden_ranges.sort_unstable();
        (0..total)
            .filter(|line| !hidden_ranges.iter().any(|(a, b)| line >= a && line <= b))
            .collect()
    }
}

/// Extensible multi-cursor source code editor widget with gutter, diagnostics, folding, and minimap.
pub struct EditorSurface {
    state: Binding<EditorBufferState>,
    font_size: f32,
    on_save: Option<Rc<dyn Fn()>>,
}

impl std::fmt::Debug for EditorSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorSurface")
            .field("font_size", &self.font_size)
            .finish()
    }
}

impl EditorSurface {
    pub fn new(state: Binding<EditorBufferState>) -> Self {
        Self {
            state,
            font_size: 13.0,
            on_save: None,
        }
    }

    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size.clamp(10.0, 28.0);
        self
    }

    pub fn on_save(mut self, f: impl Fn() + 'static) -> Self {
        self.on_save = Some(Rc::new(f));
        self
    }
}

#[derive(Debug, Clone, Copy)]
struct EditorRowsLayout {
    line_h: f32,
}

impl Layout for EditorRowsLayout {
    fn size(&self, proposed: Size<Option<f32>>, _children: &mut Children<'_>) -> Size<f32> {
        Size::new(
            proposed.width.filter(|v| v.is_finite()).unwrap_or(480.0),
            proposed.height.filter(|v| v.is_finite()).unwrap_or(320.0),
        )
    }

    fn place(&self, bounds: Rect, children: &mut Children<'_>) {
        let mut y = bounds.y + 4.0;
        let max_y = bounds.max_y();
        let row_w = (bounds.width - 62.0).max(0.0);
        for i in 0..children.len() {
            if y + self.line_h <= max_y {
                children.child(i).place(
                    Point::new(bounds.x, y),
                    Size::new(Some(row_w), Some(self.line_h)),
                );
            }
            y += self.line_h;
        }
    }
}

fn measure_mono_char_width(ts: &mut TextSystem, font_size: f32) -> f32 {
    let probe = TextNodeData {
        text: "MMMMMMMMMM".into(),
        font: rustui_core::FontSpec {
            size: font_size,
            monospace: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let shaped = ts.shape_node_data(&probe, None, 1.0);
    (shaped.size.width / 10.0).max(font_size * 0.56)
}

fn line_spans_for_slice(
    full_text: &str,
    line_start: usize,
    line_text: &str,
    syntax: &[SyntaxTokenSpan],
    default_color: Color,
) -> Vec<TextSpanData> {
    let line_end = line_start + line_text.len();
    let mut relevant: Vec<SyntaxTokenSpan> = syntax
        .iter()
        .filter(|sp| sp.end_byte > line_start && sp.start_byte < line_end)
        .copied()
        .collect();
    if relevant.is_empty() {
        return Vec::new();
    }
    relevant.sort_by_key(|s| s.start_byte);

    let mut out = Vec::new();
    let mut cursor = line_start;
    for sp in relevant {
        let s = sp.start_byte.max(cursor).min(line_end);
        let e = sp.end_byte.min(line_end);
        if s > cursor {
            out.push(TextSpanData {
                text: full_text[cursor..s].to_owned(),
                font: None,
                color: Some(default_color),
                underline: false,
                strikethrough: false,
            });
        }
        if e > s {
            out.push(TextSpanData {
                text: full_text[s..e].to_owned(),
                font: None,
                color: Some(sp.color),
                underline: false,
                strikethrough: false,
            });
            cursor = e;
        }
    }
    if cursor < line_end {
        out.push(TextSpanData {
            text: full_text[cursor..line_end].to_owned(),
            font: None,
            color: Some(default_color),
            underline: false,
            strikethrough: false,
        });
    }
    out
}

impl Primitive for EditorSurface {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let is_dark = theme.is_dark;
        let bg = if theme.uses_default_palette() {
            if is_dark {
                Color::from_hex(0x0b1220)
            } else {
                Color::surface_elevated()
            }
        } else {
            theme.colors.surface_elevated
        };
        let border = if theme.uses_default_palette() {
            Color::border()
        } else {
            theme.colors.border
        };
        let fg = if theme.uses_default_palette() {
            if is_dark {
                Color::from_hex(0xe2e8f0)
            } else {
                Color::primary()
            }
        } else {
            theme.colors.primary
        };
        let muted = if is_dark {
            Color::from_hex(0x64748b)
        } else {
            Color::from_hex(0x94a3b8)
        };
        let accent = if theme.uses_default_palette() {
            Color::accent()
        } else {
            theme.colors.accent
        };

        let root = cx.alloc_node(NodeKind::Control(rustui_core::ControlNodeData {
            kind: "editorsurface",
            ..Default::default()
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(root) {
                n.visual_props.clip_to_bounds = true;
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .max_height(f32::INFINITY)
                        .alignment(Alignment::TOP_LEADING),
                );
            }
        });

        let ts_ref = Rc::new(RefCell::new(TextSystem::new()));
        let char_w = measure_mono_char_width(&mut ts_ref.borrow_mut(), self.font_size);
        let line_h = (self.font_size * 1.55).ceil().max(18.0);
        let gutter_w = 56.0f32;
        let max_visible_rows = 34usize;

        let zstack = cx.alloc_node(NodeKind::Container(ContainerKind::ZStack {
            alignment: Alignment::TOP_LEADING,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(zstack) {
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .max_height(f32::INFINITY)
                        .alignment(Alignment::TOP_LEADING),
                );
                n.visual_props.clip_to_bounds = true;
            }
        });
        cx.runtime.attach_child(root, zstack);

        // Layer 0: Canvas for gutter, diff bars, active line highlight, selections, squiggles, brackets, carets & minimap.
        let canvas_state = self.state.clone();
        let font_sz = self.font_size;
        let canvas_node = cx.alloc_node(NodeKind::Canvas(rustui_core::CanvasNodeData {
            painter: Rc::new(move |rect, any_dl| {
                let Some(dl) = any_dl.downcast_mut::<DisplayList>() else {
                    return;
                };
                let st = canvas_state.get();
                let visible = st.visible_lines();
                let minimap_w = if st.show_minimap && rect.width > 340.0 {
                    58.0
                } else {
                    0.0
                };
                let code_x = rect.x + gutter_w + 6.0;

                // Gutter panel fill
                dl.fill_rect(
                    Rect::new(rect.x, rect.y, gutter_w, rect.height),
                    if is_dark {
                        Color::from_hex(0x0f172a)
                    } else {
                        Color::from_hex(0xf1f5f9)
                    },
                );
                dl.fill_rect(
                    Rect::new(rect.x + gutter_w - 1.0, rect.y, 1.0, rect.height),
                    Color::from_hex(0x334155).with_alpha(0.35),
                );

                let primary_sel = st.buffer.selection();
                let (active_line, _) = st.buffer.byte_to_line_col(primary_sel.active);
                let match_bracket = st.buffer.find_matching_bracket(primary_sel.active);
                let total_rows = ((rect.height / line_h).ceil() as usize).min(max_visible_rows);

                for row_offset in 0..total_rows {
                    let Some(&doc_line) = visible.get(st.scroll_line + row_offset) else {
                        break;
                    };
                    let y = rect.y + 4.0 + row_offset as f32 * line_h;

                    // Active line highlight
                    if doc_line == active_line {
                        dl.fill_rect(
                            Rect::new(
                                rect.x + gutter_w,
                                y,
                                (rect.width - gutter_w - minimap_w).max(0.0),
                                line_h,
                            ),
                            accent.with_alpha(0.08),
                        );
                    }

                    // Git diff gutter marker strip
                    if let Some(diff) = st.line_diffs.iter().find(|d| d.line == doc_line) {
                        let col = match diff.kind {
                            GutterDiffKind::Added => Color::from_hex(0x22c55e),
                            GutterDiffKind::Modified => Color::from_hex(0x3b82f6),
                            GutterDiffKind::Deleted => Color::from_hex(0xef4444),
                        };
                        dl.fill_rect(Rect::new(rect.x + 2.0, y + 2.0, 3.0, line_h - 4.0), col);
                    }

                    // Breakpoint dot
                    if st.breakpoints.contains(&doc_line) {
                        dl.fill_rounded_rect(
                            Rect::new(rect.x + 7.0, y + (line_h - 9.0) * 0.5, 9.0, 9.0),
                            CornerRadii::all(4.5),
                            Color::from_hex(0xef4444),
                        );
                    }

                    let line_start = st.buffer.line_col_to_byte(doc_line, 0);
                    let line_text = st.buffer.line_str(doc_line);
                    let line_end = line_start + line_text.len();

                    // Matching bracket highlight box
                    if let Some(mb) = match_bracket
                        && mb >= line_start
                        && mb <= line_end
                    {
                        let (_, col) = st.buffer.byte_to_line_col(mb);
                        dl.stroke_rounded_rect(
                            Rect::new(
                                code_x + col as f32 * char_w,
                                y + 1.0,
                                char_w.max(6.0),
                                line_h - 2.0,
                            ),
                            CornerRadii::all(2.0),
                            1.25,
                            accent,
                        );
                    }

                    // Multi-cursor selection highlights and carets on this line
                    for sel in st.buffer.all_selections() {
                        if !sel.is_collapsed() {
                            let s0 = sel.start();
                            let e0 = sel.end();
                            if e0 > line_start && s0 <= line_end {
                                let lo_col = if s0 <= line_start {
                                    0
                                } else {
                                    st.buffer.byte_to_line_col(s0).1
                                };
                                let hi_col = if e0 >= line_end {
                                    line_text.chars().count()
                                } else {
                                    st.buffer.byte_to_line_col(e0).1
                                };
                                let sx = code_x + lo_col as f32 * char_w;
                                let sw = ((hi_col.saturating_sub(lo_col)) as f32 * char_w).max(4.0);
                                dl.fill_rounded_rect(
                                    Rect::new(sx, y + 1.0, sw, line_h - 2.0),
                                    CornerRadii::all(2.0),
                                    accent.with_alpha(0.28),
                                );
                            }
                        }

                        if sel.active >= line_start && sel.active <= line_end {
                            let (_, caret_col) = st.buffer.byte_to_line_col(sel.active);
                            let cx_pos = code_x + caret_col as f32 * char_w;
                            dl.fill_rect(
                                Rect::new(cx_pos, y + 2.0, 2.0, (line_h - 4.0).max(font_sz)),
                                accent,
                            );
                        }
                    }

                    // Diagnostic squiggles on this line
                    for dec in &st.decorations {
                        if dec.end_byte <= line_start || dec.start_byte > line_end {
                            continue;
                        }
                        let lo_col = if dec.start_byte <= line_start {
                            0
                        } else {
                            st.buffer.byte_to_line_col(dec.start_byte).1
                        };
                        let hi_col = if dec.end_byte >= line_end {
                            line_text.chars().count().max(lo_col + 1)
                        } else {
                            st.buffer.byte_to_line_col(dec.end_byte).1.max(lo_col + 1)
                        };
                        let sx = code_x + lo_col as f32 * char_w;
                        let ex = sx + (hi_col - lo_col) as f32 * char_w;
                        let sq_y = y + line_h - 3.0;
                        match &dec.kind {
                            DecorationKind::SquiggleError
                            | DecorationKind::SquiggleWarning
                            | DecorationKind::SquiggleInfo => {
                                let sq_col = match dec.kind {
                                    DecorationKind::SquiggleError => Color::from_hex(0xef4444),
                                    DecorationKind::SquiggleWarning => Color::from_hex(0xf59e0b),
                                    _ => Color::from_hex(0x38bdf8),
                                };
                                let mut ops = vec![PathOp::MoveTo(Point::new(sx, sq_y))];
                                let mut px = sx;
                                let mut up = true;
                                while px < ex {
                                    px = (px + 4.0).min(ex);
                                    ops.push(PathOp::LineTo(Point::new(
                                        px,
                                        if up { sq_y - 2.0 } else { sq_y + 1.0 },
                                    )));
                                    up = !up;
                                }
                                dl.draw_path(ops, None, Some((1.3, sq_col)));
                            }
                            DecorationKind::BackgroundHighlight(bg_col) => {
                                dl.fill_rect(
                                    Rect::new(sx, y + 1.0, (ex - sx).max(6.0), line_h - 2.0),
                                    *bg_col,
                                );
                            }
                            _ => {}
                        }
                    }
                }

                // Minimap overview strip on the trailing edge
                if minimap_w > 0.0 {
                    let mx = rect.max_x() - minimap_w;
                    dl.fill_rect(
                        Rect::new(mx, rect.y, minimap_w, rect.height),
                        if is_dark {
                            Color::from_hex(0x0f172a).with_alpha(0.75)
                        } else {
                            Color::from_hex(0xf1f5f9).with_alpha(0.75)
                        },
                    );
                    let total_doc = st.buffer.line_count().max(1);
                    let step = (total_doc / 48).max(1);
                    let mut py = rect.y + 6.0;
                    for ln in (0..total_doc).step_by(step).take(48) {
                        let len = st.buffer.line_str(ln).chars().count().min(80) as f32;
                        let bar_w = (len * 0.55).max(4.0).min(minimap_w - 10.0);
                        dl.fill_rect(Rect::new(mx + 5.0, py, bar_w, 2.0), muted.with_alpha(0.5));
                        py += 4.0;
                        if py > rect.max_y() - 6.0 {
                            break;
                        }
                    }
                }
            }),
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(canvas_node) {
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .max_height(f32::INFINITY),
                );
            }
        });
        cx.runtime.attach_child(zstack, canvas_node);

        // Layer 1: Windowed visible line rows (gutter number + syntax-highlighted line text).
        let rows_col = cx.alloc_node(NodeKind::Container(ContainerKind::CustomLayout));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(rows_col) {
                n.custom_layout = Some(Rc::new(CustomLayoutBox(Rc::new(EditorRowsLayout {
                    line_h,
                }))));
                n.visual_props.clip_to_bounds = true;
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .max_height(f32::INFINITY)
                        .alignment(Alignment::TOP_LEADING),
                );
            }
        });
        cx.runtime.attach_child(zstack, rows_col);

        let mut row_pairs = Vec::with_capacity(max_visible_rows);
        for _ in 0..max_visible_rows {
            let row = cx.alloc_node(NodeKind::Container(ContainerKind::HStack {
                spacing: 6.0,
                alignment: rustui_core::VerticalAlignment::Center,
            }));
            let g_text = cx.alloc_node(NodeKind::Text(TextNodeData {
                text: String::new(),
                font: rustui_core::FontSpec {
                    size: font_sz - 1.0,
                    monospace: true,
                    ..Font::monospace()
                },
                color: Some(muted),
                alignment: HorizontalAlignment::Trailing,
                ..Default::default()
            }));
            let c_text = cx.alloc_node(NodeKind::Text(TextNodeData {
                text: String::new(),
                font: rustui_core::FontSpec {
                    size: font_sz,
                    monospace: true,
                    ..Font::monospace()
                },
                color: Some(fg),
                ..Default::default()
            }));
            cx.runtime.with_inner_mut(|inner| {
                if let Some(rn) = inner.nodes.get_mut(row) {
                    rn.layout_props.frame =
                        Some(Frame::new().height(line_h).alignment(Alignment::LEADING));
                }
                if let Some(gn) = inner.nodes.get_mut(g_text) {
                    gn.layout_props.frame =
                        Some(Frame::fixed(gutter_w - 4.0, line_h).alignment(Alignment::TRAILING));
                }
                if let Some(cn) = inner.nodes.get_mut(c_text) {
                    cn.layout_props.frame = Some(
                        Frame::new()
                            .min_height(line_h)
                            .max_height(line_h)
                            .max_width(f32::INFINITY)
                            .alignment(Alignment::LEADING),
                    );
                }
            });
            cx.runtime.attach_child(row, g_text);
            cx.runtime.attach_child(row, c_text);
            cx.runtime.attach_child(rows_col, row);
            row_pairs.push((row, g_text, c_text));
        }

        let row_ids_for_size: Vec<NodeId> = row_pairs.iter().map(|(r, _, _)| *r).collect();
        let rt_for_size = cx.runtime.clone();
        cx.runtime.with_inner_mut(|inner| {
            if let Some(rc_node) = inner.nodes.get_mut(rows_col) {
                rc_node.events.on_size_change = Some(Rc::new(move |sz| {
                    let vis = ((sz.height - 4.0).max(0.0) / line_h).floor() as usize;
                    rt_for_size.with_inner_mut(|inner| {
                        for (idx, &rid) in row_ids_for_size.iter().enumerate() {
                            if let Some(rn) = inner.nodes.get_mut(rid) {
                                rn.visual_props.hidden = idx >= vis;
                                if idx >= vis {
                                    rn.layout_cache.bounds = Rect::ZERO;
                                }
                            }
                        }
                    });
                }));
            }
        });

        // Reactive accessibility value/hint sync on root control node.
        let watch_state = self.state.clone();
        cx.runtime.register_node_updater(
            root,
            move || {
                let st = watch_state.get();
                (st.text(), st.status_summary())
            },
            move |root_node, (full_text, summary)| {
                root_node.accessibility.value = Some(full_text);
                root_node.accessibility.hint = Some(summary);
                DirtyFlags::ACCESSIBILITY | DirtyFlags::PAINT
            },
        );
        // Apply visible line updates via an additional post-update node updater directly on each row!
        let sync_state = self.state.clone();
        for (idx, (_row_id, g_id, c_id)) in row_pairs.iter().enumerate() {
            let st_g = sync_state.clone();
            cx.runtime.register_node_updater(
                *g_id,
                move || {
                    let st = st_g.get();
                    let visible = st.visible_lines();
                    visible
                        .get(st.scroll_line + idx)
                        .map(|&dl| {
                            let fold = st
                                .folds
                                .iter()
                                .find(|f| f.start_line == dl)
                                .map(|f| if f.collapsed { "▸" } else { "▾" });
                            match fold {
                                Some(m) => format!("{} {m}", dl + 1),
                                None => format!("{}", dl + 1),
                            }
                        })
                        .unwrap_or_default()
                },
                |n, txt| {
                    if let NodeKind::Text(t) = &mut n.kind {
                        t.text = txt;
                    }
                    DirtyFlags::PAINT
                },
            );

            let st_c = sync_state.clone();
            cx.runtime.register_node_updater(
                *c_id,
                move || {
                    let st = st_c.get();
                    let visible = st.visible_lines();
                    if let Some(&doc_line) = visible.get(st.scroll_line + idx) {
                        let full_text = st.text();
                        let line_start = st.buffer.line_col_to_byte(doc_line, 0);
                        let mut line_txt = st.buffer.line_str(doc_line);
                        let line_end = line_start + line_txt.len();
                        let mut spans = line_spans_for_slice(
                            &full_text,
                            line_start,
                            &line_txt,
                            &st.syntax_spans,
                            fg,
                        );
                        for dec in &st.decorations {
                            if dec.start_byte >= line_start
                                && dec.start_byte <= line_end
                                && let DecorationKind::InlayHint { label } = &dec.kind
                            {
                                let hint_txt = format!("  {label}");
                                if !spans.is_empty() {
                                    spans.push(TextSpanData {
                                        text: hint_txt.clone(),
                                        font: None,
                                        color: Some(muted),
                                        underline: false,
                                        strikethrough: false,
                                    });
                                }
                                line_txt.push_str(&hint_txt);
                            }
                        }
                        (line_txt, spans)
                    } else {
                        (String::new(), Vec::new())
                    }
                },
                |n, (txt, spans)| {
                    if let NodeKind::Text(t) = &mut n.kind {
                        t.text = txt;
                        t.spans = spans;
                    }
                    DirtyFlags::PAINT
                },
            );
        }

        // Interaction handlers (click to position cursor / Ctrl+Click multi-cursor / toggle fold/breakpoint / drag select / keys / scroll / IME)
        let rt_ptr = cx.runtime.clone();
        let st_ptr = self.state.clone();
        let st_drag = self.state.clone();
        let st_scroll = self.state.clone();
        let st_key = self.state.clone();
        let on_save_cb = self.on_save;
        let st_ime_p = self.state.clone();
        let st_ime_c = self.state.clone();
        let st_ak = self.state;

        cx.runtime.with_inner_mut(|inner| {
            let Some(n) = inner.nodes.get_mut(root) else {
                return;
            };
            n.layout_props.frame = Some(
                Frame::new()
                    .min_width(320.0)
                    .min_height(180.0)
                    .max_width(f32::INFINITY)
                    .max_height(f32::INFINITY)
                    .alignment(Alignment::TOP_LEADING),
            );
            n.visual_props.background = Some(bg);
            n.visual_props.border = Some((1.0, border));
            n.visual_props.corner_radii = CornerRadii::all(6.0);
            n.visual_props.clip_to_bounds = true;
            n.focus.focusable = true;
            n.focus.accepts_text_input = true;
            n.accessibility.role = AccessibilityRole::MultilineTextInput;
            n.accessibility.label = Some("Code Editor".into());
            n.accessibility.supports_focus = true;
            n.accessibility.supports_set_value = true;

            n.events.on_pointer = Some(Rc::new(move |ev| {
                if ev.phase != rustui_core::PointerPhase::Down {
                    return;
                }
                let bounds = rt_ptr
                    .with_inner(|i| i.nodes.get(root).map(|node| node.bounds()))
                    .unwrap_or(Rect::ZERO);
                let rel_x = ev.position.x - bounds.x;
                let rel_y = (ev.position.y - bounds.y - 4.0).max(0.0);
                let row_vis = (rel_y / line_h).floor() as usize;

                st_ptr.update(|st| {
                    let visible = st.visible_lines();
                    let Some(&doc_line) = visible
                        .get(st.scroll_line + row_vis)
                        .or_else(|| visible.last())
                    else {
                        return;
                    };

                    if rel_x < 24.0 {
                        st.toggle_breakpoint(doc_line);
                        return;
                    }
                    if rel_x < gutter_w {
                        st.toggle_fold(doc_line);
                        return;
                    }

                    let code_rel = (rel_x - gutter_w - 6.0).max(0.0);
                    let col = (code_rel / char_w).round() as usize;
                    let byte = st.buffer.line_col_to_byte(doc_line, col);
                    if ev.modifiers.ctrl {
                        st.buffer.add_cursor_at(byte);
                    } else if ev.modifiers.shift {
                        let cur = st.buffer.selection();
                        st.buffer.set_selection(Selection::range(cur.anchor, byte));
                    } else {
                        st.buffer.clear_secondary_cursors();
                        st.buffer.set_selection(Selection::collapsed(byte));
                    }
                });
            }));

            let rt_drag = cx.runtime.clone();
            n.events.on_drag = Some(Rc::new(move |ev| {
                let bounds = rt_drag
                    .with_inner(|i| i.nodes.get(root).map(|node| node.bounds()))
                    .unwrap_or(Rect::ZERO);
                let rel_x = (ev.current.x - bounds.x - gutter_w - 6.0).max(0.0);
                let rel_y = (ev.current.y - bounds.y - 4.0).max(0.0);
                let row_vis = (rel_y / line_h).floor() as usize;
                st_drag.update(|st| {
                    let visible = st.visible_lines();
                    let Some(&doc_line) = visible
                        .get(st.scroll_line + row_vis)
                        .or_else(|| visible.last())
                    else {
                        return;
                    };
                    let col = (rel_x / char_w).round() as usize;
                    let byte = st.buffer.line_col_to_byte(doc_line, col);
                    let cur = st.buffer.selection();
                    st.buffer.set_selection(Selection::range(cur.anchor, byte));
                });
            }));

            n.events.on_scroll = Some(Rc::new(move |ev| {
                let delta_lines = (-ev.delta_y / 24.0).round() as isize;
                if delta_lines == 0 {
                    return false;
                }
                st_scroll.update(|st| {
                    let max_scroll = st.visible_lines().len().saturating_sub(1);
                    st.scroll_line = (st.scroll_line as isize + delta_lines)
                        .clamp(0, max_scroll as isize)
                        as usize;
                });
                true
            }));

            n.events.on_key = Some(Rc::new(move |ev| {
                if !ev.is_down {
                    return false;
                }
                if ev.modifiers.ctrl && ev.modifiers.alt && matches!(ev.key, KeyCode::ArrowUp) {
                    st_key.update(|st| st.buffer.add_cursor_above());
                    return true;
                }
                if ev.modifiers.ctrl && ev.modifiers.alt && matches!(ev.key, KeyCode::ArrowDown) {
                    st_key.update(|st| st.buffer.add_cursor_below());
                    return true;
                }
                if ev.modifiers.ctrl
                    && !ev.modifiers.shift
                    && let KeyCode::Character(ref c) = ev.key
                {
                    match c.to_ascii_lowercase().as_str() {
                        "s" => {
                            if let Some(save) = &on_save_cb {
                                save();
                            }
                            return true;
                        }
                        "d" => {
                            st_key.update(|st| {
                                st.buffer.add_cursor_next_match();
                            });
                            return true;
                        }
                        "a" => {
                            st_key.update(|st| st.buffer.select_all());
                            return true;
                        }
                        "c" => {
                            let s = st_key.get();
                            DesktopPlatform::shared().clipboard_set(&s.buffer.selected_text());
                            return true;
                        }
                        "x" => {
                            let sel_txt = st_key.get().buffer.selected_text();
                            if !sel_txt.is_empty() {
                                DesktopPlatform::shared().clipboard_set(&sel_txt);
                                st_key.update(|st| st.buffer.delete_backward());
                            }
                            return true;
                        }
                        "v" => {
                            if let Some(clip) = DesktopPlatform::shared().clipboard_get() {
                                st_key.update(|st| st.buffer.insert(&clip));
                            }
                            return true;
                        }
                        "z" => {
                            st_key.update(|st| {
                                st.buffer.undo();
                            });
                            return true;
                        }
                        "y" => {
                            st_key.update(|st| {
                                st.buffer.redo();
                            });
                            return true;
                        }
                        _ => {}
                    }
                }

                match &ev.key {
                    KeyCode::Escape => {
                        st_key.update(|st| st.buffer.clear_secondary_cursors());
                        true
                    }
                    KeyCode::Tab => {
                        st_key.update(|st| {
                            if ev.modifiers.shift {
                                st.buffer.outdent_selection();
                            } else if !st.buffer.selection().is_collapsed() {
                                st.buffer.indent_selection();
                            } else {
                                st.buffer.insert("    ");
                            }
                        });
                        true
                    }
                    KeyCode::Enter => {
                        st_key.update(|st| st.buffer.insert("\n"));
                        true
                    }
                    KeyCode::Backspace => {
                        st_key.update(|st| st.buffer.delete_backward());
                        true
                    }
                    KeyCode::Delete => {
                        st_key.update(|st| st.buffer.delete_forward());
                        true
                    }
                    KeyCode::ArrowLeft => {
                        st_key.update(|st| st.buffer.move_left(ev.modifiers.shift));
                        true
                    }
                    KeyCode::ArrowRight => {
                        st_key.update(|st| st.buffer.move_right(ev.modifiers.shift));
                        true
                    }
                    KeyCode::ArrowUp => {
                        st_key.update(|st| st.buffer.move_up(ev.modifiers.shift));
                        true
                    }
                    KeyCode::ArrowDown => {
                        st_key.update(|st| st.buffer.move_down(ev.modifiers.shift));
                        true
                    }
                    KeyCode::Home => {
                        st_key.update(|st| st.buffer.move_home(ev.modifiers.shift));
                        true
                    }
                    KeyCode::End => {
                        st_key.update(|st| st.buffer.move_end(ev.modifiers.shift));
                        true
                    }
                    KeyCode::Space if !ev.modifiers.ctrl && !ev.modifiers.meta => {
                        st_key.update(|st| st.buffer.insert(" "));
                        true
                    }
                    KeyCode::Character(s) if !ev.modifiers.ctrl && !ev.modifiers.meta => {
                        st_key.update(|st| st.buffer.insert(s));
                        true
                    }
                    _ => false,
                }
            }));

            n.events.on_ime_preedit = Some(Rc::new(move |txt, cur| {
                st_ime_p.update(|st| st.ime.set_preedit(txt, cur, Rect::ZERO));
            }));
            n.events.on_ime_commit = Some(Rc::new(move |txt| {
                st_ime_c.update(|st| st.ime.commit(&txt, &mut st.buffer));
            }));
            n.events.on_accessibility_set_value = Some(Rc::new(move |val| {
                st_ak.update(|st| st.buffer.set_text(&val));
            }));
        });

        root
    }
}

impl View for EditorSurface {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}
