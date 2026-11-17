//! Tier 2 input widgets (`TextField`, `TextEditor`, `SecureField`, `Slider`, `Picker`, `Checkbox`).

use std::cell::RefCell;
use std::rc::Rc;

use rustui_core::{
    AccessibilityRole, Binding, BuildCx, Color, ContainerKind, ControlNodeData, CornerRadii, Cx,
    DirtyFlags, Edges, EmptyView, Frame, KeyCode, NodeId, NodeKind, PointerPhase,
    Primitive, Rect, TextNodeData, VerticalAlignment, View,
};
use rustui_platform::{DesktopPlatform, Platform};
use rustui_text::{EditableText, ImeState, TextBuffer};

use crate::core_widgets::Text;
use crate::theme::{Font, Theme};

fn resolve_theme(cx: &BuildCx) -> Theme {
    cx.env::<Theme>().unwrap_or_default()
}

struct EditableConfig {
    placeholder: String,
    text: Binding<String>,
    multiline: bool,
    masked: bool,
    role: AccessibilityRole,
    on_submit: Option<Rc<dyn Fn(String)>>,
}

fn build_editable_text_node(cfg: EditableConfig, cx: &mut BuildCx) -> NodeId {
    let theme = resolve_theme(cx);
    let initial = cx.runtime.enter(|| cfg.text.get());
    let buffer = Rc::new(RefCell::new(TextBuffer::new(&initial)));
    let ime = Rc::new(RefCell::new(ImeState::new()));
    let clipboard = Rc::new(DesktopPlatform::headless());

    let node = cx.alloc_node(NodeKind::Control(ControlNodeData {
        kind: if cfg.multiline {
            "texteditor"
        } else if cfg.masked {
            "securefield"
        } else {
            "textfield"
        },
        value_text: Some(initial.clone()),
        ..Default::default()
    }));

    let text_child = cx.alloc_node(NodeKind::Text(TextNodeData {
        text: format_display(&initial, &cfg.placeholder, cfg.masked),
        font: if cfg.multiline {
            Font::monospace()
        } else {
            Font::body()
        },
        color: Some(if initial.is_empty() {
            theme.colors.secondary
        } else {
            theme.colors.primary
        }),
        ..Default::default()
    }));

    let placeholder_c = cfg.placeholder.clone();
    let masked = cfg.masked;
    let multiline = cfg.multiline;
    let primary_col = theme.colors.primary;
    let secondary_col = theme.colors.secondary;
    let accent_col = theme.colors.accent;
    let border_col = theme.colors.border;

    let sync_node = {
        let rt = cx.runtime.clone();
        let buffer = buffer.clone();
        let ime = ime.clone();
        let binding = cfg.text.clone();
        let placeholder = placeholder_c.clone();
        Rc::new(move || {
            let current = buffer.borrow().text();
            binding.set(current.clone());
            let sel = buffer.borrow().selection();
            let preedit = {
                let im = ime.borrow();
                if im.preedit_text.is_empty() {
                    None
                } else {
                    Some((
                        im.preedit_text.clone(),
                        im.preedit_cursor.map_or(0, |c| c.0),
                        im.preedit_cursor.map_or(im.preedit_text.len(), |c| c.1),
                    ))
                }
            };
            rt.with_inner_mut(|inner| {
                if let Some(ctrl_node) = inner.nodes.get_mut(node) {
                    ctrl_node.accessibility.value = Some(if masked {
                        "•".repeat(current.chars().count())
                    } else {
                        current.clone()
                    });
                    if let NodeKind::Control(c) = &mut ctrl_node.kind {
                        c.value_text = Some(current.clone());
                    }
                    inner.mark_dirty(node, DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY);
                }
                if let Some(tn) = inner.nodes.get_mut(text_child)
                    && let NodeKind::Text(t) = &mut tn.kind
                {
                    t.text = format_display(&current, &placeholder, masked);
                    t.color = Some(if current.is_empty() && preedit.is_none() {
                        secondary_col
                    } else {
                        primary_col
                    });
                    t.preedit = preedit;
                    t.caret = Some(if current.is_empty() { 0 } else { sel.active });
                    t.selection = if sel.is_collapsed() {
                        None
                    } else {
                        Some((sel.start(), sel.end()))
                    };
                    inner.mark_dirty(text_child, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
                }
            });
        })
    };

    let key_sync = sync_node.clone();
    let key_buf = buffer.clone();
    let key_clip = clipboard.clone();
    let on_submit = cfg.on_submit.clone();

    let preedit_sync = sync_node.clone();
    let preedit_ime = ime.clone();
    let preedit_rt = cx.runtime.clone();

    let commit_sync = sync_node.clone();
    let commit_ime = ime.clone();
    let commit_buf = buffer.clone();

    let a11y_buf = buffer.clone();
    let a11y_sync = sync_node.clone();

    cx.runtime.with_inner_mut(|inner| {
        if let Some(n) = inner.nodes.get_mut(node) {
            n.layout_props.padding = Edges::symmetric(8.0, 10.0);
            n.layout_props.frame = Some(if multiline {
                Frame::new().min_width(220.0).min_height(120.0)
            } else {
                Frame::new().min_width(180.0).height(36.0)
            });
            n.visual_props.background = Some(theme.colors.surface_elevated);
            n.visual_props.border = Some((1.0, border_col));
            n.visual_props.corner_radii = CornerRadii::all(theme.radius_sm);
            n.visual_props.clip_to_bounds = true;
            n.focus.focusable = true;
            n.focus.accepts_text_input = true;
            n.accessibility.role = cfg.role;
            n.accessibility.label = Some(cfg.placeholder.clone());
            n.accessibility.value = Some(initial);
            n.accessibility.supports_focus = true;
            n.accessibility.supports_set_value = true;

            n.events.on_key = Some(Rc::new(move |ev| {
                if !ev.is_down {
                    return false;
                }
                let mut buf = key_buf.borrow_mut();
                let extend = ev.modifiers.shift;
                let ctrl_or_meta = ev.modifiers.ctrl || ev.modifiers.meta;
                let mut handled = true;

                match &ev.key {
                    KeyCode::Backspace => buf.delete_backward(),
                    KeyCode::Delete => buf.delete_forward(),
                    KeyCode::ArrowLeft => {
                        if ctrl_or_meta {
                            buf.move_word_left(extend);
                        } else {
                            buf.move_left(extend);
                        }
                    }
                    KeyCode::ArrowRight => {
                        if ctrl_or_meta {
                            buf.move_word_right(extend);
                        } else {
                            buf.move_right(extend);
                        }
                    }
                    KeyCode::ArrowUp if multiline => buf.move_up(extend),
                    KeyCode::ArrowDown if multiline => buf.move_down(extend),
                    KeyCode::Home => buf.move_home(extend),
                    KeyCode::End => buf.move_end(extend),
                    KeyCode::Enter => {
                        if multiline {
                            buf.insert("\n");
                        } else {
                            let val = buf.text();
                            drop(buf);
                            if let Some(sub) = &on_submit {
                                sub(val);
                            }
                            return true;
                        }
                    }
                    KeyCode::Character(ch) if ctrl_or_meta => match ch.to_ascii_lowercase().as_str() {
                        "a" => buf.select_all(),
                        "c" => {
                            let sel = buf.selected_text();
                            if !sel.is_empty() {
                                key_clip.clipboard_set(&sel);
                            }
                        }
                        "x" => {
                            let sel = buf.selected_text();
                            if !sel.is_empty() {
                                key_clip.clipboard_set(&sel);
                                buf.delete_backward();
                            }
                        }
                        "v" => {
                            if let Some(clip) = key_clip.clipboard_get() {
                                buf.insert(&clip);
                            }
                        }
                        "z" => {
                            if ev.modifiers.shift {
                                buf.redo();
                            } else {
                                buf.undo();
                            }
                        }
                        "y" => {
                            buf.redo();
                        }
                        _ => handled = false,
                    },
                    KeyCode::Space => buf.insert(" "),
                    KeyCode::Character(ch) => {
                        let s = ev.text.as_deref().unwrap_or(ch.as_str());
                        buf.insert(s);
                    }
                    _ => handled = false,
                }
                drop(buf);
                if handled {
                    key_sync();
                }
                handled
            }));

            n.events.on_ime_preedit = Some(Rc::new(move |text, cursor| {
                let caret_rect = preedit_rt
                    .with_inner(|inner| inner.nodes.get(node).map(|x| x.bounds()))
                    .unwrap_or(Rect::ZERO);
                preedit_ime.borrow_mut().set_preedit(text, cursor, caret_rect);
                preedit_sync();
            }));

            n.events.on_ime_commit = Some(Rc::new(move |text| {
                let mut buf = commit_buf.borrow_mut();
                commit_ime.borrow_mut().commit(&text, &mut *buf);
                drop(buf);
                commit_sync();
            }));

            n.events.on_accessibility_set_value = Some(Rc::new(move |val| {
                a11y_buf.borrow_mut().set_text(&val);
                a11y_sync();
            }));
        }
    });

    // React to external binding changes and focus border color.
    let watch_text = cfg.text;
    let watch_buf = buffer;
    cx.runtime.register_node_updater(
        text_child,
        move || watch_text.get(),
        move |n, val| {
            let mut buf = watch_buf.borrow_mut();
            if buf.text() != val {
                buf.set_text(&val);
            }
            if let NodeKind::Text(t) = &mut n.kind {
                t.text = format_display(&val, &placeholder_c, masked);
                t.color = Some(if val.is_empty() {
                    secondary_col
                } else {
                    primary_col
                });
            }
            DirtyFlags::LAYOUT | DirtyFlags::PAINT
        },
    );

    // Highlight border when focused.
    cx.runtime.with_inner_mut(|inner| {
        if let Some(n) = inner.nodes.get_mut(node) {
            let old_pointer = n.events.on_pointer.clone();
            n.events.on_pointer = Some(Rc::new(move |ev| {
                if let Some(p) = &old_pointer {
                    p(ev);
                }
            }));
        }
    });
    let _ = accent_col;

    cx.runtime.attach_child(node, text_child);
    node
}

fn format_display(value: &str, placeholder: &str, masked: bool) -> String {
    if value.is_empty() {
        placeholder.to_owned()
    } else if masked {
        "•".repeat(value.chars().count())
    } else {
        value.to_owned()
    }
}

/// Single-line editable text input widget (`TextField`).
pub struct TextField {
    placeholder: String,
    text: Binding<String>,
    on_submit: Option<Rc<dyn Fn(String)>>,
}

impl std::fmt::Debug for TextField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextField")
            .field("placeholder", &self.placeholder)
            .finish()
    }
}

impl TextField {
    pub fn new(placeholder: impl Into<String>, text: Binding<String>) -> Self {
        Self {
            placeholder: placeholder.into(),
            text,
            on_submit: None,
        }
    }

    pub fn on_submit(mut self, f: impl Fn(String) + 'static) -> Self {
        self.on_submit = Some(Rc::new(f));
        self
    }
}

impl Primitive for TextField {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        build_editable_text_node(
            EditableConfig {
                placeholder: self.placeholder,
                text: self.text,
                multiline: false,
                masked: false,
                role: AccessibilityRole::TextInput,
                on_submit: self.on_submit,
            },
            cx,
        )
    }
}

impl View for TextField {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Masked single-line password field (`SecureField`).
#[derive(Debug)]
pub struct SecureField {
    placeholder: String,
    text: Binding<String>,
}

impl SecureField {
    pub fn new(placeholder: impl Into<String>, text: Binding<String>) -> Self {
        Self {
            placeholder: placeholder.into(),
            text,
        }
    }
}

impl Primitive for SecureField {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        build_editable_text_node(
            EditableConfig {
                placeholder: self.placeholder,
                text: self.text,
                multiline: false,
                masked: true,
                role: AccessibilityRole::PasswordInput,
                on_submit: None,
            },
            cx,
        )
    }
}

impl View for SecureField {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Multi-line rope-backed text editor widget (`TextEditor`).
#[derive(Debug)]
pub struct TextEditor {
    text: Binding<String>,
}

impl TextEditor {
    pub fn new(text: Binding<String>) -> Self {
        Self { text }
    }
}

impl Primitive for TextEditor {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        build_editable_text_node(
            EditableConfig {
                placeholder: String::new(),
                text: self.text,
                multiline: true,
                masked: false,
                role: AccessibilityRole::MultilineTextInput,
                on_submit: None,
            },
            cx,
        )
    }
}

impl View for TextEditor {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Continuous or stepped range slider widget (`Slider`).
#[derive(Debug)]
pub struct Slider {
    value: Binding<f32>,
    min: f32,
    max: f32,
    step: Option<f32>,
}

impl Slider {
    pub fn new(value: Binding<f32>, min: f32, max: f32) -> Self {
        Self {
            value,
            min,
            max: max.max(min + 0.001),
            step: None,
        }
    }

    pub fn step(mut self, step: f32) -> Self {
        self.step = Some(step.max(0.001));
        self
    }
}

impl Primitive for Slider {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let initial = cx.runtime.enter(|| self.value.get()).clamp(self.min, self.max);
        let track = cx.alloc_node(NodeKind::Control(ControlNodeData {
            kind: "slider",
            value_f32: Some(initial),
            ..Default::default()
        }));
        let fill = cx.alloc_node(NodeKind::Modifier);

        let min = self.min;
        let max = self.max;
        let step = self.step;
        let quantize = move |raw: f32| {
            let clamped = raw.clamp(min, max);
            if let Some(s) = step {
                ((clamped - min) / s).round() * s + min
            } else {
                clamped
            }
        };

        let rt = cx.runtime.clone();
        let drag_b = self.value.clone();
        let pointer_b = self.value.clone();
        let key_b = self.value.clone();
        let inc_b = self.value.clone();
        let dec_b = self.value.clone();
        let set_b = self.value.clone();

        let update_from_x = {
            let rt = rt.clone();
            move |b: &Binding<f32>, px: f32| {
                let bounds = rt
                    .with_inner(|inner| inner.nodes.get(track).map(|n| n.bounds()))
                    .unwrap_or(Rect::new(0.0, 0.0, 160.0, 24.0));
                let t = ((px - bounds.x) / bounds.width.max(1.0)).clamp(0.0, 1.0);
                b.set(quantize(min + t * (max - min)));
            }
        };

        let update_drag = update_from_x.clone();
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(track) {
                n.layout_props.frame = Some(Frame::new().min_width(160.0).height(24.0));
                n.visual_props.background = Some(theme.colors.border);
                n.visual_props.corner_radii = CornerRadii::all(6.0);
                n.focus.focusable = true;
                n.accessibility.role = AccessibilityRole::Slider;
                n.accessibility.label = Some("Slider".into());
                n.accessibility.supports_focus = true;
                n.accessibility.supports_increment = true;
                n.accessibility.supports_decrement = true;
                n.accessibility.supports_set_value = true;

                n.events.on_pointer = Some(Rc::new(move |ev| {
                    if ev.phase == PointerPhase::Down {
                        update_from_x(&pointer_b, ev.position.x);
                    }
                }));
                n.events.on_drag = Some(Rc::new(move |ev| {
                    update_drag(&drag_b, ev.current.x);
                }));
                let delta_step = step.unwrap_or((max - min) * 0.05);
                n.events.on_key = Some(Rc::new(move |ev| {
                    if !ev.is_down {
                        return false;
                    }
                    match ev.key {
                        KeyCode::ArrowRight | KeyCode::ArrowUp => {
                            key_b.update(|v| *v = quantize(*v + delta_step));
                            true
                        }
                        KeyCode::ArrowLeft | KeyCode::ArrowDown => {
                            key_b.update(|v| *v = quantize(*v - delta_step));
                            true
                        }
                        _ => false,
                    }
                }));
                n.events.on_accessibility_increment = Some(Rc::new(move || {
                    inc_b.update(|v| *v = quantize(*v + delta_step));
                }));
                n.events.on_accessibility_decrement = Some(Rc::new(move || {
                    dec_b.update(|v| *v = quantize(*v - delta_step));
                }));
                n.events.on_accessibility_set_value = Some(Rc::new(move |s| {
                    if let Ok(parsed) = s.parse::<f32>() {
                        set_b.set(quantize(parsed));
                    }
                }));
            }
            if let Some(f) = inner.nodes.get_mut(fill) {
                f.visual_props.background = Some(theme.colors.accent);
                f.visual_props.corner_radii = CornerRadii::all(6.0);
            }
        });

        let watch_b = self.value.clone();
        cx.runtime.register_node_updater(
            track,
            move || watch_b.get(),
            move |n, val| {
                n.accessibility.value = Some(format!("{val:.2}"));
                if let NodeKind::Control(c) = &mut n.kind {
                    c.value_f32 = Some(val);
                }
                DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY
            },
        );

        cx.runtime.register_node_updater(
            fill,
            move || self.value.get(),
            move |f, val| {
                let frac = ((val - min) / (max - min)).clamp(0.0, 1.0);
                f.layout_props.frame = Some(Frame::fixed(160.0 * frac, 12.0));
                DirtyFlags::LAYOUT | DirtyFlags::PAINT
            },
        );

        cx.runtime.attach_child(track, fill);
        track
    }
}

impl View for Slider {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Option picker / combo box widget (`Picker`).
#[derive(Debug)]
pub struct Picker {
    selected: Binding<usize>,
    options: Vec<String>,
}

impl Picker {
    pub fn new(selected: Binding<usize>, options: Vec<String>) -> Self {
        Self { selected, options }
    }
}

impl Primitive for Picker {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let len = self.options.len().max(1);
        let options = Rc::new(self.options);

        let node = cx.alloc_node(NodeKind::Control(ControlNodeData {
            kind: "picker",
            ..Default::default()
        }));

        let tap_b = self.selected.clone();
        let key_b = self.selected.clone();
        let inc_b = self.selected.clone();
        let dec_b = self.selected.clone();

        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(node) {
                n.layout_props.padding = Edges::symmetric(6.0, 12.0);
                n.visual_props.background = Some(theme.colors.surface);
                n.visual_props.border = Some((1.0, theme.colors.border));
                n.visual_props.corner_radii = CornerRadii::all(theme.radius_sm);
                n.focus.focusable = true;
                n.accessibility.role = AccessibilityRole::ComboBox;
                n.accessibility.label = Some("Picker".into());
                n.accessibility.supports_click = true;
                n.accessibility.supports_focus = true;
                n.accessibility.supports_increment = true;
                n.accessibility.supports_decrement = true;

                n.events.on_tap = Some(Rc::new(move || {
                    tap_b.update(|idx| *idx = (*idx + 1) % len);
                }));
                n.events.on_key = Some(Rc::new(move |ev| {
                    if !ev.is_down {
                        return false;
                    }
                    match ev.key {
                        KeyCode::ArrowDown | KeyCode::Space | KeyCode::Enter => {
                            key_b.update(|idx| *idx = (*idx + 1) % len);
                            true
                        }
                        KeyCode::ArrowUp => {
                            key_b.update(|idx| *idx = (*idx + len - 1) % len);
                            true
                        }
                        _ => false,
                    }
                }));
                n.events.on_accessibility_increment = Some(Rc::new(move || {
                    inc_b.update(|idx| *idx = (*idx + 1) % len);
                }));
                n.events.on_accessibility_decrement = Some(Rc::new(move || {
                    dec_b.update(|idx| *idx = (*idx + len - 1) % len);
                }));
            }
        });

        let sel_for_text = self.selected.clone();
        let opts_for_text = options.clone();
        let label = Text::new(move || {
            let idx = sel_for_text.get().min(opts_for_text.len().saturating_sub(1));
            let val = opts_for_text.get(idx).map(String::as_str).unwrap_or("");
            format!("{val} ▾")
        })
        .build_node(cx);
        cx.runtime.attach_child(node, label);

        cx.runtime.register_node_updater(
            node,
            move || self.selected.get(),
            move |n, idx| {
                let text = options.get(idx).cloned().unwrap_or_default();
                n.accessibility.value = Some(text);
                DirtyFlags::ACCESSIBILITY | DirtyFlags::PAINT
            },
        );

        node
    }
}

impl View for Picker {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Interactive checkbox widget (`Checkbox`).
#[derive(Debug)]
pub struct Checkbox {
    label: String,
    checked: Binding<bool>,
}

impl Checkbox {
    pub fn new(label: impl Into<String>, checked: Binding<bool>) -> Self {
        Self {
            label: label.into(),
            checked,
        }
    }
}

impl Primitive for Checkbox {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let row = cx.alloc_node(NodeKind::Container(ContainerKind::HStack {
            spacing: 8.0,
            alignment: VerticalAlignment::Center,
        }));

        let box_node = cx.alloc_node(NodeKind::Control(ControlNodeData {
            kind: "checkbox",
            value_bool: Some(cx.runtime.enter(|| self.checked.get())),
            ..Default::default()
        }));

        let toggle_b = self.checked.clone();
        let toggle_fn = Rc::new(move || toggle_b.update(|v| *v = !*v));
        let key_fn = toggle_fn.clone();
        let set_b = self.checked.clone();

        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(box_node) {
                n.layout_props.frame = Some(Frame::fixed(20.0, 20.0));
                n.visual_props.corner_radii = CornerRadii::all(4.0);
                n.focus.focusable = true;
                n.accessibility.role = AccessibilityRole::CheckBox;
                n.accessibility.label = Some(self.label.clone());
                n.accessibility.supports_click = true;
                n.accessibility.supports_focus = true;
                n.accessibility.supports_set_value = true;
                n.events.on_tap = Some(toggle_fn);
                n.events.on_key = Some(Rc::new(move |ev| {
                    if ev.is_down && matches!(ev.key, KeyCode::Space | KeyCode::Enter) {
                        key_fn();
                        return true;
                    }
                    false
                }));
                n.events.on_accessibility_set_value = Some(Rc::new(move |val| {
                    set_b.set(matches!(val.as_str(), "true" | "1" | "checked"));
                }));
            }
        });

        let watch_b = self.checked.clone();
        let accent = theme.colors.accent;
        let surface = theme.colors.surface_elevated;
        let border = theme.colors.border;
        cx.runtime.register_node_updater(
            box_node,
            move || watch_b.get(),
            move |n, checked| {
                n.visual_props.background = Some(if checked { accent } else { surface });
                n.visual_props.border = Some((1.5, if checked { accent } else { border }));
                n.accessibility.value = Some(if checked { "true".into() } else { "false".into() });
                if let NodeKind::Control(c) = &mut n.kind {
                    c.value_bool = Some(checked);
                }
                DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY
            },
        );

        let check_mark = Text::new(move || {
            if self.checked.get() {
                "✓".to_owned()
            } else {
                String::new()
            }
        })
        .color(Color::WHITE)
        .font(Font::caption())
        .build_node(cx);
        cx.runtime.attach_child(box_node, check_mark);

        let label = Text::new(self.label).build_node(cx);
        cx.runtime.attach_child(row, box_node);
        cx.runtime.attach_child(row, label);
        row
    }
}

impl View for Checkbox {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}
