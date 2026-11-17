//! Hit testing, Tab/Shift+Tab focus traversal, pointer capture, gestures, keyboard & IME routing.

use std::time::{Duration, Instant};

use rustui_core::{
    DirtyFlags, DragEvent, KeyCode, KeyEvent, Modifiers, NodeId, NodeKind, PinchEvent, Point,
    PointerButton, PointerEvent, PointerPhase, Rect, Runtime, RuntimeInner, ScrollAxes,
    ScrollEvent,
};

/// Clip-aware, front-to-back hit tester on the retained node tree.
#[derive(Debug, Default)]
pub struct HitTester;

impl HitTester {
    /// Returns the front-most visible node containing `point` along with its ancestor path.
    pub fn hit_test(runtime: &Runtime, root: NodeId, point: Point) -> Option<NodeId> {
        runtime.with_inner(|inner| hit_test_recursive(inner, root, point, None))
    }

    /// Returns the chain `[hit_node, parent, ..., root]` for event bubbling.
    pub fn ancestor_chain(runtime: &Runtime, node: NodeId) -> Vec<NodeId> {
        let mut chain = Vec::new();
        runtime.with_inner(|inner| {
            let mut cursor = Some(node);
            while let Some(id) = cursor {
                chain.push(id);
                cursor = inner.nodes.get(id).and_then(|n| n.parent);
            }
        });
        chain
    }
}

fn hit_test_recursive(
    inner: &RuntimeInner,
    node_id: NodeId,
    point: Point,
    clip: Option<Rect>,
) -> Option<NodeId> {
    let node = inner.nodes.get(node_id)?;
    if node.visual_props.hidden {
        return None;
    }

    let bounds = node.bounds();
    if let Some(c) = clip
        && !c.contains(point)
    {
        return None;
    }

    let next_clip = if node.visual_props.clip_to_bounds || matches!(node.kind, NodeKind::Scroll(_))
    {
        Some(match clip {
            Some(c) => c.intersection(&bounds).unwrap_or(Rect::ZERO),
            None => bounds,
        })
    } else {
        clip
    };

    // Front-to-back: iterate children in reverse paint order.
    for &cid in node.children.iter().rev() {
        if let Some(hit) = hit_test_recursive(inner, cid, point, next_clip) {
            return Some(hit);
        }
    }

    if bounds.width > 0.0 && bounds.height > 0.0 && bounds.contains(point) {
        let is_interactive = node.events.on_tap.is_some()
            || node.events.on_double_tap.is_some()
            || node.events.on_long_press.is_some()
            || node.events.on_hover.is_some()
            || node.events.on_pointer.is_some()
            || node.events.on_drag.is_some()
            || node.events.on_scroll.is_some()
            || node.events.on_pinch.is_some()
            || node.events.on_key.is_some()
            || node.focus.focusable
            || matches!(node.kind, NodeKind::Control(_) | NodeKind::Scroll(_));
        if is_interactive {
            return Some(node_id);
        }
    }

    None
}

/// Manages focus tree, `Tab` / `Shift+Tab` traversal, and `.focused(binding)` sync.
#[derive(Debug, Default)]
pub struct FocusManager {
    focused: Option<NodeId>,
}

impl FocusManager {
    pub fn new() -> Self {
        Self { focused: None }
    }

    pub fn focused_node(&self) -> Option<NodeId> {
        self.focused
    }

    /// Collects all focusable nodes in visual/tree order sorted by `tab_index`.
    pub fn focus_ring(&self, runtime: &Runtime, root: NodeId) -> Vec<NodeId> {
        let mut ring = Vec::new();
        runtime.with_inner(|inner| collect_focusable(inner, root, &mut ring));
        ring.sort_by_key(|&id| {
            runtime.with_inner(|inner| inner.nodes.get(id).map_or(0, |n| n.focus.tab_index))
        });
        ring
    }

    pub fn set_focus(&mut self, runtime: &Runtime, target: Option<NodeId>) {
        if self.focused == target {
            return;
        }
        let prev = self.focused;
        self.focused = target;

        let mut bindings_to_update = Vec::new();
        runtime.with_inner_mut(|inner| {
            if let Some(pid) = prev
                && let Some(n) = inner.nodes.get_mut(pid)
            {
                n.focus.focused = false;
                if let Some(b) = &n.focus.focused_binding {
                    bindings_to_update.push((b.clone(), false));
                }
                inner.mark_dirty(pid, DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY);
            }
            if let Some(tid) = target
                && let Some(n) = inner.nodes.get_mut(tid)
            {
                n.focus.focused = true;
                if let Some(b) = &n.focus.focused_binding {
                    bindings_to_update.push((b.clone(), true));
                }
                inner.mark_dirty(tid, DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY);
            }
        });

        runtime.enter(|| {
            for (b, val) in bindings_to_update {
                b.set(val);
            }
        });
    }

    pub fn focus_next(&mut self, runtime: &Runtime, root: NodeId) -> Option<NodeId> {
        let ring = self.focus_ring(runtime, root);
        if ring.is_empty() {
            self.set_focus(runtime, None);
            return None;
        }
        let next = match self.focused.and_then(|f| ring.iter().position(|&n| n == f)) {
            Some(idx) => ring[(idx + 1) % ring.len()],
            None => ring[0],
        };
        self.set_focus(runtime, Some(next));
        Some(next)
    }

    pub fn focus_prev(&mut self, runtime: &Runtime, root: NodeId) -> Option<NodeId> {
        let ring = self.focus_ring(runtime, root);
        if ring.is_empty() {
            self.set_focus(runtime, None);
            return None;
        }
        let prev = match self.focused.and_then(|f| ring.iter().position(|&n| n == f)) {
            Some(idx) => ring[(idx + ring.len() - 1) % ring.len()],
            None => ring[ring.len() - 1],
        };
        self.set_focus(runtime, Some(prev));
        Some(prev)
    }
}

fn collect_focusable(inner: &RuntimeInner, node_id: NodeId, out: &mut Vec<NodeId>) {
    let Some(node) = inner.nodes.get(node_id) else {
        return;
    };
    if node.visual_props.hidden {
        return;
    }
    if node.focus.focusable && !matches!(&node.kind, NodeKind::Control(c) if c.is_disabled) {
        out.push(node_id);
    }
    for &cid in &node.children {
        collect_focusable(inner, cid, out);
    }
}

/// Recognizes pointer hover, capture, tap, double/triple click, long press, drag, scroll, and pinch.
#[derive(Debug)]
pub struct GestureRecognizer {
    hovered_node: Option<NodeId>,
    captured_node: Option<NodeId>,
    press_origin: Option<Point>,
    last_pointer_pos: Point,
    press_time: Option<Instant>,
    is_dragging: bool,
    last_click_time: Option<Instant>,
    last_click_pos: Point,
    click_count: u32,
}

impl Default for GestureRecognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureRecognizer {
    pub fn new() -> Self {
        Self {
            hovered_node: None,
            captured_node: None,
            press_origin: None,
            last_pointer_pos: Point::ZERO,
            press_time: None,
            is_dragging: false,
            last_click_time: None,
            last_click_pos: Point::ZERO,
            click_count: 0,
        }
    }

    pub fn handle_pointer(
        &mut self,
        runtime: &Runtime,
        root: NodeId,
        focus: &mut FocusManager,
        mut event: PointerEvent,
    ) {
        let now = Instant::now();
        match event.phase {
            PointerPhase::Move => {
                if let Some(captured) = self.captured_node {
                    let delta = event.position - self.last_pointer_pos;
                    self.last_pointer_pos = event.position;
                    let origin = self.press_origin.unwrap_or(event.position);
                    if !self.is_dragging && origin.distance_to(event.position) >= 4.0 {
                        self.is_dragging = true;
                        self.dispatch_drag(
                            runtime,
                            captured,
                            DragEvent {
                                start: origin,
                                current: event.position,
                                delta,
                                phase: PointerPhase::Down,
                            },
                        );
                    } else if self.is_dragging {
                        self.dispatch_drag(
                            runtime,
                            captured,
                            DragEvent {
                                start: origin,
                                current: event.position,
                                delta,
                                phase: PointerPhase::Move,
                            },
                        );
                    }
                    self.dispatch_raw_pointer(runtime, captured, event);
                } else {
                    let hit = HitTester::hit_test(runtime, root, event.position);
                    if hit != self.hovered_node {
                        if let Some(old) = self.hovered_node {
                            self.set_hover(runtime, old, false);
                        }
                        self.hovered_node = hit;
                        if let Some(new_hit) = hit {
                            self.set_hover(runtime, new_hit, true);
                        }
                    }
                    if let Some(target) = hit {
                        self.dispatch_raw_pointer(runtime, target, event);
                    }
                }
            }
            PointerPhase::Down => {
                let is_double_window = self
                    .last_click_time
                    .is_some_and(|t| now.duration_since(t) < Duration::from_millis(450))
                    && self.last_click_pos.distance_to(event.position) < 6.0;
                self.click_count = if is_double_window {
                    self.click_count + 1
                } else {
                    1
                };
                self.last_click_time = Some(now);
                self.last_click_pos = event.position;
                event.click_count = self.click_count;

                let hit = HitTester::hit_test(runtime, root, event.position);
                self.captured_node = hit;
                self.press_origin = Some(event.position);
                self.last_pointer_pos = event.position;
                self.press_time = Some(now);
                self.is_dragging = false;

                if let Some(target) = hit {
                    // Focus nearest focusable ancestor (or self).
                    let chain = HitTester::ancestor_chain(runtime, target);
                    let focus_target = chain.iter().copied().find(|&id| {
                        runtime.with_inner(|inner| {
                            inner.nodes.get(id).is_some_and(|n| n.focus.focusable)
                        })
                    });
                    if let Some(ft) = focus_target {
                        focus.set_focus(runtime, Some(ft));
                    } else {
                        focus.set_focus(runtime, None);
                    }

                    self.set_pressed(runtime, target, true);
                    self.dispatch_raw_pointer(runtime, target, event);
                } else {
                    focus.set_focus(runtime, None);
                }
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                if let Some(captured) = self.captured_node.take() {
                    self.set_pressed(runtime, captured, false);
                    if self.is_dragging {
                        let origin = self.press_origin.unwrap_or(event.position);
                        self.dispatch_drag(
                            runtime,
                            captured,
                            DragEvent {
                                start: origin,
                                current: event.position,
                                delta: event.position - self.last_pointer_pos,
                                phase: PointerPhase::Up,
                            },
                        );
                    } else if event.phase == PointerPhase::Up {
                        let hit = HitTester::hit_test(runtime, root, event.position);
                        if hit == Some(captured)
                            || HitTester::ancestor_chain(runtime, hit.unwrap_or(captured))
                                .contains(&captured)
                        {
                            let held_long = self
                                .press_time
                                .is_some_and(|t| now.duration_since(t) >= Duration::from_millis(500));
                            if held_long {
                                self.dispatch_long_press(runtime, captured);
                            } else if self.click_count >= 2 {
                                self.dispatch_double_tap(runtime, captured);
                            }
                            if event.button == PointerButton::Primary {
                                self.dispatch_tap(runtime, captured);
                            }
                        }
                    }
                    self.dispatch_raw_pointer(runtime, captured, event);
                }
                self.is_dragging = false;
                self.press_origin = None;
                self.press_time = None;
            }
        }
    }

    pub fn handle_scroll(&mut self, runtime: &Runtime, root: NodeId, event: ScrollEvent) {
        let target = HitTester::hit_test(runtime, root, event.position).unwrap_or(root);
        let chain = HitTester::ancestor_chain(runtime, target);
        for id in chain {
            let cb = runtime.with_inner(|inner| {
                inner
                    .nodes
                    .get(id)
                    .and_then(|n| n.events.on_scroll.clone())
            });
            if let Some(f) = cb {
                let handled = runtime.enter(|| f(event));
                if handled {
                    return;
                }
            }
            // Built-in scroll / virtual list fallback
            let handled = runtime.with_inner_mut(|inner| {
                let Some(n) = inner.nodes.get_mut(id) else {
                    return false;
                };
                let bounds = n.bounds();
                match &mut n.kind {
                    NodeKind::Scroll(s) => {
                        let max_x = (s.content_size.width - bounds.width).max(0.0);
                        let max_y = (s.content_size.height - bounds.height).max(0.0);
                        match s.axes {
                            ScrollAxes::Vertical => {
                                s.offset.y = (s.offset.y - event.delta_y).clamp(0.0, max_y);
                            }
                            ScrollAxes::Horizontal => {
                                s.offset.x = (s.offset.x - event.delta_x).clamp(0.0, max_x);
                            }
                            ScrollAxes::Both => {
                                s.offset.x = (s.offset.x - event.delta_x).clamp(0.0, max_x);
                                s.offset.y = (s.offset.y - event.delta_y).clamp(0.0, max_y);
                            }
                        }
                        s.velocity = Point::new(event.delta_x, event.delta_y);
                        inner.mark_dirty(id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
                        true
                    }
                    NodeKind::VirtualList(vl) => {
                        let pitch = vl.estimated_row_height + vl.spacing;
                        let total_h = vl.item_count as f32 * pitch;
                        let max_y = (total_h - bounds.height).max(0.0);
                        vl.scroll_offset = (vl.scroll_offset - event.delta_y).clamp(0.0, max_y);
                        inner.mark_dirty(id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
                        true
                    }
                    _ => false,
                }
            });
            if handled {
                return;
            }
        }
    }

    pub fn handle_pinch(&mut self, runtime: &Runtime, root: NodeId, event: PinchEvent) {
        let target = HitTester::hit_test(runtime, root, event.position).unwrap_or(root);
        for id in HitTester::ancestor_chain(runtime, target) {
            let cb = runtime.with_inner(|inner| {
                inner
                    .nodes
                    .get(id)
                    .and_then(|n| n.events.on_pinch.clone())
            });
            if let Some(f) = cb {
                runtime.enter(|| f(event));
                return;
            }
        }
    }

    pub fn handle_key(
        &mut self,
        runtime: &Runtime,
        root: NodeId,
        focus: &mut FocusManager,
        event: KeyEvent,
    ) -> bool {
        if let Some(focused) = focus.focused_node() {
            for id in HitTester::ancestor_chain(runtime, focused) {
                let cb = runtime.with_inner(|inner| {
                    inner.nodes.get(id).and_then(|n| n.events.on_key.clone())
                });
                if let Some(f) = cb {
                    let handled = runtime.enter(|| f(event.clone()));
                    if handled {
                        return true;
                    }
                }
            }
        } else {
            // Check root-level key listener if nothing is focused.
            let cb = runtime.with_inner(|inner| {
                inner
                    .nodes
                    .get(root)
                    .and_then(|n| n.events.on_key.clone())
            });
            if let Some(f) = cb
                && runtime.enter(|| f(event.clone()))
            {
                return true;
            }
        }

        // Default Tab / Shift+Tab focus traversal when unhandled.
        if event.is_down && event.key == KeyCode::Tab {
            if event.modifiers.shift {
                focus.focus_prev(runtime, root);
            } else {
                focus.focus_next(runtime, root);
            }
            return true;
        }
        false
    }

    pub fn handle_ime_preedit(
        &self,
        runtime: &Runtime,
        focus: &FocusManager,
        text: String,
        cursor: Option<(usize, usize)>,
    ) {
        let Some(focused) = focus.focused_node() else {
            return;
        };
        let cb = runtime.with_inner(|inner| {
            inner
                .nodes
                .get(focused)
                .and_then(|n| n.events.on_ime_preedit.clone())
        });
        if let Some(f) = cb {
            runtime.enter(|| f(text, cursor));
        }
    }

    pub fn handle_ime_commit(&self, runtime: &Runtime, focus: &FocusManager, text: String) {
        let Some(focused) = focus.focused_node() else {
            return;
        };
        let cb = runtime.with_inner(|inner| {
            inner
                .nodes
                .get(focused)
                .and_then(|n| n.events.on_ime_commit.clone())
        });
        if let Some(f) = cb {
            runtime.enter(|| f(text));
        }
    }

    fn set_hover(&self, runtime: &Runtime, node_id: NodeId, hovered: bool) {
        let (cb, mark) = runtime.with_inner_mut(|inner| {
            let Some(n) = inner.nodes.get_mut(node_id) else {
                return (None, false);
            };
            let mut changed = false;
            if let NodeKind::Control(c) = &mut n.kind {
                c.is_hovered = hovered;
                changed = true;
            }
            (n.events.on_hover.clone(), changed)
        });
        if mark {
            runtime.with_inner_mut(|inner| inner.mark_dirty(node_id, DirtyFlags::PAINT));
        }
        if let Some(f) = cb {
            runtime.enter(|| f(hovered));
        }
    }

    fn set_pressed(&self, runtime: &Runtime, node_id: NodeId, pressed: bool) {
        runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(node_id)
                && let NodeKind::Control(c) = &mut n.kind
            {
                c.is_pressed = pressed;
                inner.mark_dirty(node_id, DirtyFlags::PAINT);
            }
        });
    }

    fn dispatch_raw_pointer(&self, runtime: &Runtime, node_id: NodeId, event: PointerEvent) {
        for id in HitTester::ancestor_chain(runtime, node_id) {
            let cb = runtime.with_inner(|inner| {
                inner
                    .nodes
                    .get(id)
                    .and_then(|n| n.events.on_pointer.clone())
            });
            if let Some(f) = cb {
                runtime.enter(|| f(event));
            }
        }
    }

    fn dispatch_drag(&self, runtime: &Runtime, node_id: NodeId, event: DragEvent) {
        for id in HitTester::ancestor_chain(runtime, node_id) {
            let cb = runtime.with_inner(|inner| {
                inner.nodes.get(id).and_then(|n| n.events.on_drag.clone())
            });
            if let Some(f) = cb {
                runtime.enter(|| f(event));
                return;
            }
        }
    }

    fn dispatch_tap(&self, runtime: &Runtime, node_id: NodeId) {
        for id in HitTester::ancestor_chain(runtime, node_id) {
            let cb = runtime.with_inner(|inner| {
                inner.nodes.get(id).and_then(|n| n.events.on_tap.clone())
            });
            if let Some(f) = cb {
                runtime.enter(|| f());
                return;
            }
        }
    }

    fn dispatch_double_tap(&self, runtime: &Runtime, node_id: NodeId) {
        for id in HitTester::ancestor_chain(runtime, node_id) {
            let cb = runtime.with_inner(|inner| {
                inner
                    .nodes
                    .get(id)
                    .and_then(|n| n.events.on_double_tap.clone())
            });
            if let Some(f) = cb {
                runtime.enter(|| f());
                return;
            }
        }
    }

    fn dispatch_long_press(&self, runtime: &Runtime, node_id: NodeId) {
        for id in HitTester::ancestor_chain(runtime, node_id) {
            let cb = runtime.with_inner(|inner| {
                inner
                    .nodes
                    .get(id)
                    .and_then(|n| n.events.on_long_press.clone())
            });
            if let Some(f) = cb {
                runtime.enter(|| f());
                return;
            }
        }
    }
}

/// Helper to synthesize a standard [`KeyEvent`] for tests and shortcuts.
pub fn key_down(key: KeyCode, modifiers: Modifiers, text: Option<&str>) -> KeyEvent {
    KeyEvent {
        key,
        physical_code: String::new(),
        text: text.map(str::to_owned),
        modifiers,
        is_repeat: false,
        is_down: true,
    }
}
