//! Tier 2 input widgets (`TextField`, `TextEditor`, `SecureField`, `Slider`, `Picker`, `Checkbox`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rustui_core::{
    AccessibilityRole, Alignment, AnyView, Binding, BuildCx, Color, ContainerKind, ControlNodeData,
    CornerRadii, Cx, DirtyFlags, Edges, EmptyView, Frame, HorizontalAlignment, If, KeyCode, NodeId,
    NodeKind, Point, PointerButton, PointerPhase, Primitive, Rect, Runtime, TextNodeData,
    VerticalAlignment, View, ViewExt,
};
use rustui_platform::{DesktopPlatform, Platform};
use rustui_text::{EditableText, ImeState, Selection, TextBuffer, TextSystem};

use crate::core_widgets::{Button, Text, VStack};
use crate::theme::{ButtonStyle, Font, Theme};

thread_local! {
    static EDITOR_TEXT_SYS: RefCell<TextSystem> = RefCell::new(TextSystem::new());
}

fn clamp_char_boundary(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn hit_test_editor_pos(
    rt: &Runtime,
    text_child: NodeId,
    buf: &TextBuffer,
    masked: bool,
    pos: Point,
) -> usize {
    let raw_text = buf.text();
    if raw_text.is_empty() {
        return 0;
    }
    let info = rt.with_inner(|inner| {
        let tn = inner.nodes.get(text_child)?;
        let bounds = tn.bounds();
        if let NodeKind::Text(t) = &tn.kind {
            Some((bounds, t.clone()))
        } else {
            None
        }
    });
    let Some((bounds, data)) = info else {
        return buf.selection().active;
    };
    let shaped = EDITOR_TEXT_SYS.with(|sys| {
        sys.borrow_mut()
            .shape_node_data(&data, Some(bounds.width.max(40.0) + 1.0), 1.0)
    });
    let local_x = (pos.x - bounds.x).max(0.0);
    let local_y = (pos.y - bounds.y).max(0.0);
    let raw_idx = shaped.hit_test_point(local_x, local_y);
    if masked {
        let clamped_mask = clamp_char_boundary(&data.text, raw_idx);
        let char_idx = data.text[..clamped_mask].chars().count();
        raw_text
            .char_indices()
            .nth(char_idx)
            .map_or(raw_text.len(), |(b, _)| b)
    } else {
        clamp_char_boundary(&raw_text, raw_idx)
    }
}

fn move_vertical_visual(
    rt: &Runtime,
    text_child: NodeId,
    buf: &mut TextBuffer,
    delta_lines: isize,
    extend: bool,
    preferred_x: &Cell<Option<f32>>,
) {
    let raw_text = buf.text();
    let active = buf.selection().active.min(raw_text.len());
    let anchor = buf.selection().anchor.min(raw_text.len());
    let info = rt.with_inner(|inner| {
        let tn = inner.nodes.get(text_child)?;
        let bounds = tn.bounds();
        if let NodeKind::Text(t) = &tn.kind {
            Some((bounds, t.clone()))
        } else {
            None
        }
    });
    let Some((bounds, data)) = info else {
        if delta_lines < 0 {
            buf.move_up(extend);
        } else {
            buf.move_down(extend);
        }
        return;
    };
    let shaped = EDITOR_TEXT_SYS.with(|sys| {
        sys.borrow_mut()
            .shape_node_data(&data, Some(bounds.width.max(40.0) + 1.0), 1.0)
    });
    let cur_rect = shaped.caret_rect_for_offset(active, data.font.size * 1.3);
    let target_x = preferred_x.get().unwrap_or(cur_rect.x);
    preferred_x.set(Some(target_x));

    let next_idx = if delta_lines < 0 {
        if cur_rect.y <= 2.0 {
            0
        } else {
            let hit =
                shaped.hit_test_point(target_x, (cur_rect.y - cur_rect.height * 0.5).max(0.0));
            let clamped = clamp_char_boundary(&raw_text, hit);
            if clamped == active {
                buf.move_up(extend);
                return;
            }
            clamped
        }
    } else if cur_rect.max_y() >= shaped.size.height - 2.0 {
        raw_text.len()
    } else {
        let hit = shaped.hit_test_point(target_x, cur_rect.max_y() + cur_rect.height * 0.5);
        let clamped = clamp_char_boundary(&raw_text, hit);
        if clamped == active {
            buf.move_down(extend);
            return;
        }
        clamped
    };

    buf.set_selection(if extend {
        Selection::range(anchor, next_idx)
    } else {
        Selection::collapsed(next_idx)
    });
}

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
    let clipboard = Rc::new(DesktopPlatform::shared());

    let zstack = cx.alloc_node(NodeKind::Container(ContainerKind::ZStack {
        alignment: Alignment::TOP_LEADING,
    }));
    let menu_offset = cx.runtime.create_state::<Option<Point>>(None, Some(zstack));

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

    let standard = theme.uses_default_palette();
    let primary_col = if standard {
        Color::primary()
    } else {
        theme.colors.primary
    };
    let secondary_col = if standard {
        Color::secondary()
    } else {
        theme.colors.secondary
    };
    let bg_col = if standard {
        Color::surface_elevated()
    } else {
        theme.colors.surface_elevated
    };
    let border_col = if standard {
        Color::border()
    } else {
        theme.colors.border
    };

    let init_caret_bytes = if cfg.masked {
        initial.chars().count() * '•'.len_utf8()
    } else {
        initial.len()
    };
    let text_child = cx.alloc_node(NodeKind::Text(TextNodeData {
        text: format_display(&initial, &cfg.placeholder, cfg.masked),
        font: if cfg.multiline {
            Font::monospace()
        } else {
            Font::body()
        },
        color: Some(if initial.is_empty() {
            secondary_col
        } else {
            primary_col
        }),
        caret: Some(if initial.is_empty() {
            0
        } else {
            init_caret_bytes
        }),
        ..Default::default()
    }));

    let placeholder_c = cfg.placeholder.clone();
    let masked = cfg.masked;
    let multiline = cfg.multiline;
    let last_synced = Rc::new(RefCell::new(initial.clone()));

    let sync_node = {
        let rt = cx.runtime.clone();
        let buffer = buffer.clone();
        let ime = ime.clone();
        let binding = cfg.text.clone();
        let placeholder = placeholder_c.clone();
        let last_synced = last_synced.clone();
        Rc::new(move || {
            let current = buffer.borrow().text();
            if *last_synced.borrow() != current {
                *last_synced.borrow_mut() = current.clone();
                binding.set(current.clone());
            }
            let sel = buffer.borrow().selection();
            let map_byte = |b: usize| -> usize {
                if !masked {
                    return b.min(current.len());
                }
                let chars = current[..b.min(current.len())].chars().count();
                chars * '•'.len_utf8()
            };
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
                    let has_preedit = preedit.is_some();
                    t.text = if current.is_empty() && has_preedit {
                        String::new()
                    } else {
                        format_display(&current, &placeholder, masked)
                    };
                    t.color = Some(if current.is_empty() && !has_preedit {
                        secondary_col
                    } else {
                        primary_col
                    });
                    t.preedit = preedit;
                    t.caret = Some(if current.is_empty() {
                        0
                    } else {
                        map_byte(sel.active)
                    });
                    t.selection = if sel.is_collapsed() {
                        None
                    } else {
                        Some((map_byte(sel.start()), map_byte(sel.end())))
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

    let ptr_buf = buffer.clone();
    let ptr_sync = sync_node.clone();
    let ptr_rt = cx.runtime.clone();
    let is_dragging_sel = Rc::new(Cell::new(false));
    let drag_sel_ptr = is_dragging_sel.clone();
    let preferred_x = Rc::new(Cell::new(None::<f32>));
    let ptr_pref_x = preferred_x.clone();
    let key_pref_x = preferred_x;
    let key_rt = cx.runtime.clone();

    cx.runtime.with_inner_mut(|inner| {
        if let Some(tn) = inner.nodes.get_mut(text_child) {
            tn.layout_props.frame = Some(
                Frame::new()
                    .max_width(f32::INFINITY)
                    .max_height(if multiline { f32::INFINITY } else { 22.0 })
                    .alignment(if multiline {
                        Alignment::TOP_LEADING
                    } else {
                        Alignment::LEADING
                    }),
            );
        }
        if let Some(n) = inner.nodes.get_mut(node) {
            n.layout_props.padding = Edges::symmetric(8.0, 10.0);
            n.layout_props.frame = Some(if multiline {
                Frame::new()
                    .min_width(220.0)
                    .min_height(120.0)
                    .max_width(f32::INFINITY)
                    .max_height(f32::INFINITY)
                    .alignment(Alignment::TOP_LEADING)
            } else {
                Frame::new()
                    .min_width(140.0)
                    .ideal_width(220.0)
                    .max_width(f32::INFINITY)
                    .height(36.0)
                    .alignment(Alignment::LEADING)
            });
            n.visual_props.background = Some(bg_col);
            n.visual_props.border = Some((1.0, border_col));
            n.visual_props.corner_radii = CornerRadii::all(theme.radius_sm);
            n.visual_props.clip_to_bounds = true;
            n.focus.focusable = true;
            n.focus.accepts_text_input = true;
            n.accessibility.role = cfg.role;
            let a11y_label = if cfg.placeholder.is_empty() {
                if cfg.multiline {
                    "Text Editor".to_owned()
                } else {
                    "Text Field".to_owned()
                }
            } else {
                cfg.placeholder.clone()
            };
            n.accessibility.label = Some(a11y_label);
            n.accessibility.value = Some(initial);
            n.accessibility.supports_focus = true;
            n.accessibility.supports_set_value = true;

            n.events.on_pointer = Some(Rc::new(move |ev| match ev.phase {
                PointerPhase::Down => {
                    if ev.button == PointerButton::Secondary {
                        let nb = ptr_rt
                            .with_inner(|i| i.nodes.get(node).map(|x| x.bounds()))
                            .unwrap_or(Rect::ZERO);
                        let rel = Point::new(
                            (ev.position.x - nb.x).max(0.0),
                            (ev.position.y - nb.y).max(0.0),
                        );
                        menu_offset.update(|m| {
                            if m.is_some() {
                                *m = None;
                            } else {
                                *m = Some(rel);
                            }
                        });
                        return;
                    }
                    if ev.button == PointerButton::Primary {
                        if menu_offset.get_untracked().is_some() {
                            menu_offset.set(None);
                        }
                        ptr_pref_x.set(None);
                        drag_sel_ptr.set(true);
                        let mut buf = ptr_buf.borrow_mut();
                        let idx =
                            hit_test_editor_pos(&ptr_rt, text_child, &buf, masked, ev.position);
                        if ev.click_count >= 3 {
                            buf.select_line_at(idx);
                        } else if ev.click_count == 2 {
                            buf.select_word_at(idx);
                        } else if ev.modifiers.shift {
                            let anchor = buf.selection().anchor;
                            buf.set_selection(Selection::range(anchor, idx));
                        } else {
                            buf.set_selection(Selection::collapsed(idx));
                        }
                        drop(buf);
                        ptr_sync();
                    }
                }
                PointerPhase::Move => {
                    if drag_sel_ptr.get() {
                        let mut buf = ptr_buf.borrow_mut();
                        let idx =
                            hit_test_editor_pos(&ptr_rt, text_child, &buf, masked, ev.position);
                        let anchor = buf.selection().anchor;
                        if buf.selection().active != idx {
                            buf.set_selection(Selection::range(anchor, idx));
                            drop(buf);
                            ptr_sync();
                        }
                    }
                }
                PointerPhase::Up | PointerPhase::Cancel => {
                    drag_sel_ptr.set(false);
                }
            }));

            n.events.on_key = Some(Rc::new(move |ev| {
                if !ev.is_down {
                    return false;
                }
                if ev.key == KeyCode::Escape && menu_offset.get_untracked().is_some() {
                    menu_offset.set(None);
                    return true;
                }
                let mut buf = key_buf.borrow_mut();
                let extend = ev.modifiers.shift;
                let ctrl_or_meta = ev.modifiers.ctrl || ev.modifiers.meta;
                let mut handled = true;

                let phys_shortcut = match ev.physical_code.as_str() {
                    "KeyCode::KeyA" | "KeyA" => Some('a'),
                    "KeyCode::KeyC" | "KeyC" => Some('c'),
                    "KeyCode::KeyX" | "KeyX" => Some('x'),
                    "KeyCode::KeyV" | "KeyV" => Some('v'),
                    "KeyCode::KeyZ" | "KeyZ" => Some('z'),
                    "KeyCode::KeyY" | "KeyY" => Some('y'),
                    _ => None,
                };
                let char_shortcut = match &ev.key {
                    KeyCode::Character(ch) => ch.chars().next().map(|c| c.to_ascii_lowercase()),
                    _ => None,
                };
                let shortcut_ch = char_shortcut.or(phys_shortcut);

                match &ev.key {
                    KeyCode::Backspace => {
                        key_pref_x.set(None);
                        buf.delete_backward();
                    }
                    KeyCode::Delete => {
                        key_pref_x.set(None);
                        buf.delete_forward();
                    }
                    KeyCode::ArrowLeft => {
                        key_pref_x.set(None);
                        if ctrl_or_meta {
                            buf.move_word_left(extend);
                        } else {
                            buf.move_left(extend);
                        }
                    }
                    KeyCode::ArrowRight => {
                        key_pref_x.set(None);
                        if ctrl_or_meta {
                            buf.move_word_right(extend);
                        } else {
                            buf.move_right(extend);
                        }
                    }
                    KeyCode::ArrowUp => {
                        if multiline {
                            move_vertical_visual(
                                &key_rt,
                                text_child,
                                &mut buf,
                                -1,
                                extend,
                                &key_pref_x,
                            );
                        } else {
                            let anchor = buf.selection().anchor;
                            buf.set_selection(if extend {
                                Selection::range(anchor, 0)
                            } else {
                                Selection::collapsed(0)
                            });
                        }
                    }
                    KeyCode::ArrowDown => {
                        if multiline {
                            move_vertical_visual(
                                &key_rt,
                                text_child,
                                &mut buf,
                                1,
                                extend,
                                &key_pref_x,
                            );
                        } else {
                            let anchor = buf.selection().anchor;
                            let end = buf.len_bytes();
                            buf.set_selection(if extend {
                                Selection::range(anchor, end)
                            } else {
                                Selection::collapsed(end)
                            });
                        }
                    }
                    KeyCode::Home => {
                        key_pref_x.set(None);
                        buf.move_home(extend);
                    }
                    KeyCode::End => {
                        key_pref_x.set(None);
                        buf.move_end(extend);
                    }
                    KeyCode::Named(name) if name == "Insert" && ev.modifiers.shift => {
                        key_pref_x.set(None);
                        if let Some(clip) = key_clip.clipboard_get() {
                            buf.insert(&clip);
                        }
                    }
                    KeyCode::Named(name) if name == "Insert" && ctrl_or_meta => {
                        let sel = buf.selected_text();
                        if !sel.is_empty() && !masked {
                            key_clip.clipboard_set(&sel);
                        }
                    }
                    KeyCode::Enter => {
                        key_pref_x.set(None);
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
                    _ if ctrl_or_meta => match shortcut_ch {
                        Some('a') => buf.select_all(),
                        Some('c') => {
                            if !masked {
                                let sel = buf.selected_text();
                                if !sel.is_empty() {
                                    key_clip.clipboard_set(&sel);
                                }
                            }
                        }
                        Some('x') => {
                            if !masked {
                                let sel = buf.selected_text();
                                if !sel.is_empty() {
                                    key_clip.clipboard_set(&sel);
                                    buf.delete_backward();
                                }
                            }
                        }
                        Some('v') => {
                            if let Some(clip) = key_clip.clipboard_get() {
                                buf.insert(&clip);
                            }
                        }
                        Some('z') => {
                            if ev.modifiers.shift {
                                buf.redo();
                            } else {
                                buf.undo();
                            }
                        }
                        Some('y') => {
                            buf.redo();
                        }
                        _ => handled = false,
                    },
                    KeyCode::Space => {
                        key_pref_x.set(None);
                        buf.insert(" ");
                    }
                    KeyCode::Character(ch) => {
                        key_pref_x.set(None);
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
                preedit_ime
                    .borrow_mut()
                    .set_preedit(text, cursor, caret_rect);
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

    // React to external binding changes.
    let watch_text = cfg.text;
    let watch_buf = buffer.clone();
    let watch_synced = last_synced;
    cx.runtime.register_node_updater(
        text_child,
        move || watch_text.get(),
        move |n, val| {
            *watch_synced.borrow_mut() = val.clone();
            let mut buf = watch_buf.borrow_mut();
            if buf.text() != val {
                buf.set_text(&val);
            }
            let sel = buf.selection();
            let map_b = |b: usize| -> usize {
                if !masked {
                    return b.min(val.len());
                }
                val[..b.min(val.len())].chars().count() * '•'.len_utf8()
            };
            if let NodeKind::Text(t) = &mut n.kind {
                let has_preedit = t.preedit.is_some();
                t.text = if val.is_empty() && has_preedit {
                    String::new()
                } else {
                    format_display(&val, &placeholder_c, masked)
                };
                t.color = Some(if val.is_empty() && !has_preedit {
                    secondary_col
                } else {
                    primary_col
                });
                t.caret = Some(if val.is_empty() { 0 } else { map_b(sel.active) });
                t.selection = if sel.is_collapsed() {
                    None
                } else {
                    Some((map_b(sel.start()), map_b(sel.end())))
                };
            }
            DirtyFlags::LAYOUT | DirtyFlags::PAINT
        },
    );

    cx.runtime.attach_child(node, text_child);
    cx.runtime.attach_child(zstack, node);

    // Built-in right-click context menu overlay (Cut, Copy, Paste, Select All, Undo, Clear).
    let ctx_buf = buffer;
    let ctx_clip = clipboard;
    let ctx_sync = sync_node;
    let menu_item_style = ButtonStyle {
        foreground: Color::primary(),
        padding: Edges::symmetric(5.0, 10.0),
        ..ButtonStyle::plain(&theme)
    };
    let menu_view = If::new(
        move || menu_offset.get().is_some(),
        move || {
            fn select_cut_target(buf: &mut TextBuffer, multiline: bool) -> String {
                let sel = buf.selected_text();
                if !sel.is_empty() {
                    return sel;
                }
                if !multiline {
                    buf.select_all();
                    return buf.selected_text();
                }
                let active = buf.selection().active;
                buf.select_line_at(active);
                buf.selected_text()
            }

            let off = menu_offset.get_untracked().unwrap_or(Point::ZERO);
            let mut btns: Vec<AnyView> = Vec::new();

            if !masked {
                let cut_b = ctx_buf.clone();
                let cut_c = ctx_clip.clone();
                let cut_s = ctx_sync.clone();
                let cut_st = menu_item_style.clone();
                btns.push(AnyView::new(
                    Button::new("Cut", move || {
                        let mut b = cut_b.borrow_mut();
                        let txt = select_cut_target(&mut b, multiline);
                        if !txt.is_empty() {
                            cut_c.clipboard_set(&txt);
                            b.delete_backward();
                        }
                        drop(b);
                        cut_s();
                        menu_offset.set(None);
                    })
                    .style(cut_st)
                    .accessibility_role(AccessibilityRole::MenuItem),
                ));

                let copy_b = ctx_buf.clone();
                let copy_c = ctx_clip.clone();
                let copy_s = ctx_sync.clone();
                let copy_st = menu_item_style.clone();
                btns.push(AnyView::new(
                    Button::new("Copy", move || {
                        let b = copy_b.borrow();
                        let sel = b.selected_text();
                        let txt = if !sel.is_empty() { sel } else { b.text() };
                        if !txt.is_empty() {
                            copy_c.clipboard_set(&txt);
                        }
                        drop(b);
                        copy_s();
                        menu_offset.set(None);
                    })
                    .style(copy_st)
                    .accessibility_role(AccessibilityRole::MenuItem),
                ));
            }

            let paste_b = ctx_buf.clone();
            let paste_c = ctx_clip.clone();
            let paste_s = ctx_sync.clone();
            let paste_st = menu_item_style.clone();
            btns.push(AnyView::new(
                Button::new("Paste", move || {
                    if let Some(txt) = paste_c.clipboard_get()
                        && !txt.is_empty()
                    {
                        paste_b.borrow_mut().insert(&txt);
                        paste_s();
                    }
                    menu_offset.set(None);
                })
                .style(paste_st)
                .accessibility_role(AccessibilityRole::MenuItem),
            ));

            let all_b = ctx_buf.clone();
            let all_s = ctx_sync.clone();
            let all_st = menu_item_style.clone();
            btns.push(AnyView::new(
                Button::new("Select All", move || {
                    all_b.borrow_mut().select_all();
                    all_s();
                    menu_offset.set(None);
                })
                .style(all_st)
                .accessibility_role(AccessibilityRole::MenuItem),
            ));

            let undo_b = ctx_buf.clone();
            let undo_s = ctx_sync.clone();
            let undo_st = menu_item_style.clone();
            btns.push(AnyView::new(
                Button::new("Undo", move || {
                    if undo_b.borrow_mut().undo() {
                        undo_s();
                    }
                    menu_offset.set(None);
                })
                .style(undo_st)
                .accessibility_role(AccessibilityRole::MenuItem),
            ));

            let clr_b = ctx_buf.clone();
            let clr_s = ctx_sync.clone();
            let clr_st = menu_item_style.clone();
            btns.push(AnyView::new(
                Button::new("Clear", move || {
                    let mut b = clr_b.borrow_mut();
                    if !b.text().is_empty() {
                        b.select_all();
                        b.delete_backward();
                    }
                    drop(b);
                    clr_s();
                    menu_offset.set(None);
                })
                .style(clr_st)
                .accessibility_role(AccessibilityRole::MenuItem),
            ));

            VStack::new(2, btns)
                .alignment(HorizontalAlignment::Leading)
                .padding(6)
                .frame(Frame::new().min_width(136.0))
                .background(Color::surface_elevated())
                .border(1, Color::border())
                .corner_radius(8)
                .shadow(Color::rgba(0.0, 0.0, 0.0, 0.22), 12, Point::new(0.0, 4.0))
                .offset(off.x, off.y)
                .accessibility_role(AccessibilityRole::Popover)
        },
    );
    cx.build_child(zstack, menu_view);
    zstack
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
        let initial = cx
            .runtime
            .enter(|| self.value.get())
            .clamp(self.min, self.max);
        let track = cx.alloc_node(NodeKind::Control(ControlNodeData {
            kind: "slider",
            value_f32: Some(initial),
            ..Default::default()
        }));
        let inner_stack = cx.alloc_node(NodeKind::Container(ContainerKind::ZStack {
            alignment: Alignment::LEADING,
        }));
        let fill = cx.alloc_node(NodeKind::Modifier);
        let thumb = cx.alloc_node(NodeKind::Modifier);

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
        let standard = theme.uses_default_palette();
        let track_bg = if standard {
            Color::border()
        } else {
            theme.colors.border
        };
        let fill_bg = if standard {
            Color::accent()
        } else {
            theme.colors.accent
        };
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(track) {
                n.layout_props.frame = Some(
                    Frame::new()
                        .width(180.0)
                        .height(12.0)
                        .alignment(Alignment::LEADING),
                );
                n.visual_props.background = Some(track_bg);
                n.visual_props.border = Some((1.0, track_bg.with_alpha(0.65)));
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
            if let Some(st) = inner.nodes.get_mut(inner_stack) {
                st.layout_props.frame = Some(
                    Frame::new()
                        .width(180.0)
                        .height(16.0)
                        .alignment(Alignment::LEADING),
                );
            }
            if let Some(f) = inner.nodes.get_mut(fill) {
                f.visual_props.background = Some(fill_bg);
                f.visual_props.corner_radii = CornerRadii::all(6.0);
            }
            if let Some(th) = inner.nodes.get_mut(thumb) {
                th.layout_props.frame = Some(Frame::fixed(16.0, 16.0));
                th.visual_props.background = Some(Color::WHITE);
                th.visual_props.border = Some((2.0, fill_bg));
                th.visual_props.corner_radii = CornerRadii::all(8.0);
                th.visual_props.shadow = Some(rustui_core::BoxShadow {
                    color: Color::rgba(0.0, 0.0, 0.0, 0.22),
                    offset: Point::new(0.0, 1.0),
                    blur_radius: 3.0,
                });
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

        let fill_b = self.value.clone();
        cx.runtime.register_node_updater(
            fill,
            move || fill_b.get(),
            move |f, val| {
                let frac = ((val - min) / (max - min)).clamp(0.0, 1.0);
                let fill_w = (180.0 * frac).max(if frac > 0.0 { 6.0 } else { 0.0 });
                f.layout_props.frame = Some(Frame::fixed(fill_w, 12.0));
                DirtyFlags::LAYOUT | DirtyFlags::PAINT
            },
        );

        cx.runtime.register_node_updater(
            thumb,
            move || self.value.get(),
            move |th, val| {
                let frac = ((val - min) / (max - min)).clamp(0.0, 1.0);
                let thumb_x = (180.0 * frac - 8.0).clamp(0.0, 164.0);
                th.layout_props.offset = Point::new(thumb_x, 0.0);
                DirtyFlags::LAYOUT | DirtyFlags::PAINT
            },
        );

        cx.runtime.attach_child(inner_stack, fill);
        cx.runtime.attach_child(inner_stack, thumb);
        cx.runtime.attach_child(track, inner_stack);
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

        let standard = theme.uses_default_palette();
        let bg_col = if standard {
            Color::surface_elevated()
        } else {
            theme.colors.surface_elevated
        };
        let border_col = if standard {
            Color::border()
        } else {
            theme.colors.border
        };

        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(node) {
                n.layout_props.padding = Edges::symmetric(6.0, 12.0);
                n.layout_props.frame = Some(
                    Frame::new()
                        .min_width(92.0)
                        .min_height(34.0)
                        .alignment(Alignment::CENTER),
                );
                n.visual_props.background = Some(bg_col);
                n.visual_props.border = Some((1.0, border_col));
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
            let idx = sel_for_text
                .get()
                .min(opts_for_text.len().saturating_sub(1));
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
        let lbl_toggle = toggle_fn.clone();
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
        let standard = theme.uses_default_palette();
        let accent = if standard {
            Color::accent()
        } else {
            theme.colors.accent
        };
        let surface = if standard {
            Color::surface_elevated()
        } else {
            theme.colors.surface_elevated
        };
        let border = if standard {
            Color::border()
        } else {
            theme.colors.border
        };
        cx.runtime.register_node_updater(
            box_node,
            move || watch_b.get(),
            move |n, checked| {
                n.visual_props.background = Some(if checked { accent } else { surface });
                n.visual_props.border = Some((1.5, if checked { accent } else { border }));
                n.accessibility.value = Some(if checked {
                    "true".into()
                } else {
                    "false".into()
                });
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
        cx.runtime.with_inner_mut(|inner| {
            if let Some(l) = inner.nodes.get_mut(label) {
                l.events.on_tap = Some(lbl_toggle);
            }
        });
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
