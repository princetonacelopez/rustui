//! Hit testing, Tab/Shift+Tab focus traversal, pointer capture, gestures, keyboard & IME routing.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rustui_core::{
    AccessibilityRole, DirtyFlags, DragEvent, KeyCode, KeyEvent, Modifiers, NodeId, NodeKind,
    PinchEvent, Point, PointerButton, PointerEvent, PointerPhase, Rect, Runtime, RuntimeInner,
    ScrollAxes, ScrollEvent,
};

/// Clip-aware, front-to-back hit tester on the retained node tree.
#[derive(Debug, Default)]
pub struct HitTester;

impl HitTester {
    /// Returns the front-most visible node containing `point` along with its ancestor path.
    pub fn hit_test(runtime: &Runtime, root: NodeId, point: Point) -> Option<NodeId> {
        runtime.with_inner(|inner| {
            let mut overlays = Vec::new();
            collect_popover_overlays(inner, root, &mut overlays);
            for overlay_id in overlays.into_iter().rev() {
                if let Some(hit) = hit_test_recursive(inner, overlay_id, point, None) {
                    return Some(hit);
                }
            }
            hit_test_recursive(inner, root, point, None)
        })
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

    /// Returns the preferred CSS/winit cursor semantic at `point` (`"text"`, `"ew-resize"`, `"pointer"`, or `"default"`).
    pub fn cursor_kind_at(runtime: &Runtime, root: NodeId, point: Point) -> &'static str {
        let Some(hit) = Self::hit_test(runtime, root, point) else {
            return "default";
        };
        let chain = Self::ancestor_chain(runtime, hit);
        runtime.with_inner(|inner| {
            for id in chain {
                let Some(n) = inner.nodes.get(id) else {
                    continue;
                };
                if n.accessibility.role == AccessibilityRole::Splitter {
                    return "ew-resize";
                }
                if let NodeKind::Control(c) = &n.kind {
                    if c.is_disabled {
                        return "default";
                    }
                    return match c.kind {
                        "textfield" | "texteditor" | "securefield" => "text",
                        _ => "pointer",
                    };
                }
                if n.events.on_tap.is_some() || n.events.on_drag.is_some() {
                    return "pointer";
                }
            }
            "default"
        })
    }
}

fn collect_popover_overlays(inner: &RuntimeInner, node_id: NodeId, out: &mut Vec<NodeId>) {
    let Some(node) = inner.nodes.get(node_id) else {
        return;
    };
    if node.visual_props.hidden {
        return;
    }
    if node.accessibility.role == rustui_core::AccessibilityRole::Popover {
        out.push(node_id);
    }
    for &cid in &node.children {
        collect_popover_overlays(inner, cid, out);
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
    active_touches: HashMap<u64, Point>,
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
            active_touches: HashMap::new(),
        }
    }

    /// Processes a single or multi-finger touch contact, routing taps, touch drags, scrolling, and 2-finger pinch.
    #[allow(clippy::too_many_arguments)]
    pub fn handle_touch(
        &mut self,
        runtime: &Runtime,
        root: NodeId,
        focus: &mut FocusManager,
        id: u64,
        phase: PointerPhase,
        position: Point,
        modifiers: Modifiers,
    ) {
        match phase {
            PointerPhase::Down => {
                self.active_touches.insert(id, position);
                if self.active_touches.len() == 1 {
                    self.handle_pointer(
                        runtime,
                        root,
                        focus,
                        PointerEvent {
                            position,
                            button: PointerButton::Primary,
                            phase: PointerPhase::Down,
                            click_count: 1,
                            modifiers,
                        },
                    );
                }
            }
            PointerPhase::Move => {
                let prev = self.active_touches.insert(id, position).unwrap_or(position);
                if self.active_touches.len() == 2 {
                    if let Some((_, &other)) = self.active_touches.iter().find(|&(&k, _)| k != id) {
                        let old_dist = prev.distance_to(other);
                        let new_dist = position.distance_to(other);
                        if old_dist > 1.0 && (new_dist - old_dist).abs() > 0.1 {
                            let scale = (new_dist / old_dist).clamp(0.5, 2.0);
                            let mid = Point::new(
                                (position.x + other.x) * 0.5,
                                (position.y + other.y) * 0.5,
                            );
                            self.handle_pinch(
                                runtime,
                                root,
                                PinchEvent {
                                    position: mid,
                                    scale_delta: scale - 1.0,
                                    phase: PointerPhase::Move,
                                },
                            );
                        }
                    }
                    return;
                }
                if self.active_touches.len() == 1 {
                    let dx = position.x - prev.x;
                    let dy = position.y - prev.y;
                    let has_drag_control = self.captured_node.is_some_and(|cap| {
                        HitTester::ancestor_chain(runtime, cap).into_iter().any(|nid| {
                            runtime.with_inner(|inner| {
                                inner.nodes.get(nid).is_some_and(|n| {
                                    n.events.on_drag.is_some()
                                        || matches!(
                                            &n.kind,
                                            NodeKind::Control(c)
                                                if matches!(
                                                    c.kind,
                                                    "slider" | "textfield" | "texteditor" | "securefield"
                                                )
                                        )
                                })
                            })
                        })
                    });
                    if !has_drag_control && (dx.abs() > 0.0 || dy.abs() > 0.0) {
                        self.handle_scroll(
                            runtime,
                            root,
                            ScrollEvent {
                                position,
                                delta_x: dx,
                                delta_y: dy,
                                is_momentum: false,
                            },
                        );
                    }
                    self.handle_pointer(
                        runtime,
                        root,
                        focus,
                        PointerEvent {
                            position,
                            button: PointerButton::Primary,
                            phase: PointerPhase::Move,
                            click_count: 0,
                            modifiers,
                        },
                    );
                }
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                self.active_touches.remove(&id);
                if self.active_touches.is_empty() {
                    self.handle_pointer(
                        runtime,
                        root,
                        focus,
                        PointerEvent {
                            position,
                            button: PointerButton::Primary,
                            phase,
                            click_count: 1,
                            modifiers,
                        },
                    );
                }
            }
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
                        let drag_payload = runtime.with_inner(|inner| {
                            HitTester::ancestor_chain(runtime, captured)
                                .into_iter()
                                .find_map(|nid| {
                                    inner
                                        .nodes
                                        .get(nid)
                                        .and_then(|n| n.events.on_drag_payload.clone())
                                })
                        });
                        if let Some(payload_fn) = drag_payload {
                            let payload = runtime.enter(|| payload_fn());
                            self.dispatch_drop(
                                runtime,
                                root,
                                rustui_core::DropEvent {
                                    position: event.position,
                                    payload,
                                },
                            );
                        }
                    } else if event.phase == PointerPhase::Up {
                        let hit = HitTester::hit_test(runtime, root, event.position);
                        if hit == Some(captured)
                            || HitTester::ancestor_chain(runtime, hit.unwrap_or(captured))
                                .contains(&captured)
                        {
                            let held_long = self.press_time.is_some_and(|t| {
                                now.duration_since(t) >= Duration::from_millis(500)
                            });
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
            let cb = runtime
                .with_inner(|inner| inner.nodes.get(id).and_then(|n| n.events.on_scroll.clone()));
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
            let cb = runtime
                .with_inner(|inner| inner.nodes.get(id).and_then(|n| n.events.on_pinch.clone()));
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
                let cb = runtime
                    .with_inner(|inner| inner.nodes.get(id).and_then(|n| n.events.on_key.clone()));
                if let Some(f) = cb {
                    let handled = runtime.enter(|| f(event.clone()));
                    if handled {
                        return true;
                    }
                }
            }
        } else {
            // Check root-level key listener if nothing is focused.
            let cb = runtime
                .with_inner(|inner| inner.nodes.get(root).and_then(|n| n.events.on_key.clone()));
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
        let chain = HitTester::ancestor_chain(runtime, node_id);
        let callbacks: Vec<_> = runtime.with_inner_mut(|inner| {
            let mut out = Vec::new();
            let mut dirty_nodes = Vec::new();
            let mut found_control = false;
            for &id in &chain {
                if let Some(n) = inner.nodes.get_mut(id) {
                    if !found_control
                        && let NodeKind::Control(c) = &mut n.kind
                        && !c.is_disabled
                    {
                        c.is_hovered = hovered;
                        found_control = true;
                        dirty_nodes.push(id);
                    }
                    if let Some(cb) = &n.events.on_hover {
                        out.push(cb.clone());
                    }
                }
            }
            for id in dirty_nodes {
                inner.mark_dirty(id, DirtyFlags::PAINT);
            }
            out
        });
        for f in callbacks {
            runtime.enter(|| f(hovered));
        }
    }

    fn set_pressed(&self, runtime: &Runtime, node_id: NodeId, pressed: bool) {
        let chain = HitTester::ancestor_chain(runtime, node_id);
        runtime.with_inner_mut(|inner| {
            for id in chain {
                if let Some(n) = inner.nodes.get_mut(id)
                    && let NodeKind::Control(c) = &mut n.kind
                {
                    if !c.is_disabled {
                        c.is_pressed = pressed;
                        inner.mark_dirty(id, DirtyFlags::PAINT);
                    }
                    break;
                }
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
            let cb = runtime
                .with_inner(|inner| inner.nodes.get(id).and_then(|n| n.events.on_drag.clone()));
            if let Some(f) = cb {
                runtime.enter(|| f(event));
                return;
            }
        }
    }

    fn dispatch_tap(&self, runtime: &Runtime, node_id: NodeId) {
        for id in HitTester::ancestor_chain(runtime, node_id) {
            let cb = runtime
                .with_inner(|inner| inner.nodes.get(id).and_then(|n| n.events.on_tap.clone()));
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

    /// Dispatches a file/text/custom drop event to the top-most `.on_drop(...)` handler under `event.position`.
    pub fn dispatch_drop(
        &self,
        runtime: &Runtime,
        root: NodeId,
        event: rustui_core::DropEvent,
    ) -> bool {
        let Some(hit) = HitTester::hit_test(runtime, root, event.position) else {
            return false;
        };
        for id in HitTester::ancestor_chain(runtime, hit) {
            let cb = runtime
                .with_inner(|inner| inner.nodes.get(id).and_then(|n| n.events.on_drop.clone()));
            if let Some(f) = cb
                && runtime.enter(|| f(event.clone()))
            {
                return true;
            }
        }
        false
    }
}

/// Single key stroke with modifiers (`Ctrl+K`, `Cmd/Ctrl+Shift+P`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyStroke {
    pub key: KeyCode,
    pub modifiers: Modifiers,
}

impl KeyStroke {
    pub fn new(key: KeyCode, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }

    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut mods = Modifiers::NONE;
        let mut key: Option<KeyCode> = None;
        for part in spec.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "cmd" | "meta" | "super" => mods.meta = true,
                "cmdorctrl" => mods.ctrl = true,
                "alt" | "option" => mods.alt = true,
                "shift" => mods.shift = true,
                "enter" | "return" => key = Some(KeyCode::Enter),
                "tab" => key = Some(KeyCode::Tab),
                "space" => key = Some(KeyCode::Space),
                "esc" | "escape" => key = Some(KeyCode::Escape),
                "backspace" => key = Some(KeyCode::Backspace),
                "delete" | "del" => key = Some(KeyCode::Delete),
                "left" => key = Some(KeyCode::ArrowLeft),
                "right" => key = Some(KeyCode::ArrowRight),
                "up" => key = Some(KeyCode::ArrowUp),
                "down" => key = Some(KeyCode::ArrowDown),
                "home" => key = Some(KeyCode::Home),
                "end" => key = Some(KeyCode::End),
                other if other.starts_with('f') && other.len() <= 3 => {
                    if let Ok(n) = other[1..].parse::<u8>() {
                        key = Some(KeyCode::F(n));
                    } else {
                        key = Some(KeyCode::Character(other.to_owned()));
                    }
                }
                other => {
                    key = Some(KeyCode::Character(other.to_ascii_lowercase()));
                }
            }
        }
        key.map(|k| Self {
            key: k,
            modifiers: mods,
        })
        .ok_or_else(|| format!("invalid keystroke '{spec}'"))
    }

    pub fn matches_event(&self, ev: &KeyEvent) -> bool {
        if self.modifiers.ctrl != ev.modifiers.ctrl
            || self.modifiers.shift != ev.modifiers.shift
            || self.modifiers.alt != ev.modifiers.alt
            || self.modifiers.meta != ev.modifiers.meta
        {
            return false;
        }
        match (&self.key, &ev.key) {
            (KeyCode::Character(a), KeyCode::Character(b)) => a.eq_ignore_ascii_case(b),
            _ => self.key == ev.key,
        }
    }

    pub fn format(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.ctrl {
            parts.push("Ctrl".to_owned());
        }
        if self.modifiers.alt {
            parts.push("Alt".to_owned());
        }
        if self.modifiers.shift {
            parts.push("Shift".to_owned());
        }
        if self.modifiers.meta {
            parts.push("Meta".to_owned());
        }
        let k = match &self.key {
            KeyCode::Character(c) => c.to_ascii_uppercase(),
            KeyCode::Enter => "Enter".into(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::Space => "Space".into(),
            KeyCode::Escape => "Escape".into(),
            KeyCode::Backspace => "Backspace".into(),
            KeyCode::Delete => "Delete".into(),
            KeyCode::ArrowLeft => "Left".into(),
            KeyCode::ArrowRight => "Right".into(),
            KeyCode::ArrowUp => "Up".into(),
            KeyCode::ArrowDown => "Down".into(),
            KeyCode::Home => "Home".into(),
            KeyCode::End => "End".into(),
            KeyCode::F(n) => format!("F{n}"),
            other => format!("{other:?}"),
        };
        parts.push(k);
        parts.join("+")
    }
}

/// Single stroke or two-stroke key chord (`Ctrl+K Ctrl+S`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChord {
    pub strokes: Vec<KeyStroke>,
}

impl KeyChord {
    pub fn single(stroke: KeyStroke) -> Self {
        Self {
            strokes: vec![stroke],
        }
    }

    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut strokes = Vec::new();
        for tok in spec.split_whitespace() {
            strokes.push(KeyStroke::parse(tok)?);
        }
        if strokes.is_empty() || strokes.len() > 2 {
            return Err(format!("key chord must have 1 or 2 strokes: '{spec}'"));
        }
        Ok(Self { strokes })
    }

    pub fn format(&self) -> String {
        self.strokes
            .iter()
            .map(KeyStroke::format)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Optional focus/scope context restriction for a keybinding entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyContext {
    #[default]
    Global,
    EditorFocused,
    TerminalFocused,
    PaletteClosed,
}

/// Bound shortcut mapping a [`KeyChord`] and [`KeyContext`] to a named command ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingEntry {
    pub chord: KeyChord,
    pub command: String,
    pub context: KeyContext,
}

/// Outcome returned when advancing the stateful chord matcher with a [`KeyEvent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordMatch {
    None,
    PendingPrefix,
    Command(String),
}

/// Registry resolving single shortcuts and multi-stroke chords (`Ctrl+K Ctrl+S`) with TOML persistence.
#[derive(Debug, Clone, Default)]
pub struct KeybindingRegistry {
    bindings: Vec<KeybindingEntry>,
    pending_first: Option<KeyStroke>,
}

impl KeybindingRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(&mut self, chord: KeyChord, command: impl Into<String>, context: KeyContext) {
        self.bindings.push(KeybindingEntry {
            chord,
            command: command.into(),
            context,
        });
    }

    pub fn bind_str(
        &mut self,
        chord_spec: &str,
        command: impl Into<String>,
        context: KeyContext,
    ) -> Result<(), String> {
        let chord = KeyChord::parse(chord_spec)?;
        self.bind(chord, command, context);
        Ok(())
    }

    pub fn entries(&self) -> &[KeybindingEntry] {
        &self.bindings
    }

    pub fn match_event(&mut self, ev: &KeyEvent, active_ctx: KeyContext) -> ChordMatch {
        if !ev.is_down {
            return ChordMatch::None;
        }
        let ctx_ok =
            |entry_ctx: KeyContext| entry_ctx == KeyContext::Global || entry_ctx == active_ctx;

        if let Some(first) = self.pending_first.take() {
            for entry in &self.bindings {
                if ctx_ok(entry.context)
                    && entry.chord.strokes.len() == 2
                    && entry.chord.strokes[0] == first
                    && entry.chord.strokes[1].matches_event(ev)
                {
                    return ChordMatch::Command(entry.command.clone());
                }
            }
        }

        // Check if this keystroke completes a 1-stroke binding or starts a 2-stroke chord.
        for entry in &self.bindings {
            if ctx_ok(entry.context)
                && entry.chord.strokes.len() == 2
                && entry.chord.strokes[0].matches_event(ev)
            {
                self.pending_first = Some(entry.chord.strokes[0].clone());
                return ChordMatch::PendingPrefix;
            }
        }

        for entry in &self.bindings {
            if ctx_ok(entry.context)
                && entry.chord.strokes.len() == 1
                && entry.chord.strokes[0].matches_event(ev)
            {
                return ChordMatch::Command(entry.command.clone());
            }
        }

        ChordMatch::None
    }

    /// Exports bindings as simple TOML tables (`[[bindings]]`).
    pub fn to_toml_string(&self) -> String {
        let mut out = String::new();
        for b in &self.bindings {
            let ctx = match b.context {
                KeyContext::Global => "global",
                KeyContext::EditorFocused => "editor",
                KeyContext::TerminalFocused => "terminal",
                KeyContext::PaletteClosed => "palette_closed",
            };
            out.push_str(&format!(
                "[[bindings]]\nchord = \"{}\"\ncommand = \"{}\"\ncontext = \"{ctx}\"\n\n",
                b.chord.format(),
                b.command.replace('"', "\\\"")
            ));
        }
        out
    }

    /// Parses `[[bindings]]` entries produced by [`KeybindingRegistry::to_toml_string`].
    pub fn from_toml_str(src: &str) -> Result<Self, String> {
        let mut reg = Self::new();
        let mut chord: Option<String> = None;
        let mut command: Option<String> = None;
        let mut context = KeyContext::Global;
        let flush = |reg: &mut Self,
                     ch: &mut Option<String>,
                     cmd: &mut Option<String>,
                     ctx: KeyContext|
         -> Result<(), String> {
            if let (Some(c), Some(k)) = (ch.take(), cmd.take()) {
                reg.bind_str(&c, k, ctx)?;
            }
            Ok(())
        };

        for raw in src.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[[bindings]]" {
                flush(&mut reg, &mut chord, &mut command, context)?;
                context = KeyContext::Global;
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "chord" => chord = Some(v.to_owned()),
                    "command" => command = Some(v.to_owned()),
                    "context" => {
                        context = match v {
                            "editor" => KeyContext::EditorFocused,
                            "terminal" => KeyContext::TerminalFocused,
                            "palette_closed" => KeyContext::PaletteClosed,
                            _ => KeyContext::Global,
                        };
                    }
                    _ => {}
                }
            }
        }
        flush(&mut reg, &mut chord, &mut command, context)?;
        Ok(reg)
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
