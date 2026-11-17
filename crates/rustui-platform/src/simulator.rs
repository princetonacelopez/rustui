//! Synthetic event driver and headless frame runner for interaction, visual, and accessibility tests.

use std::time::Instant;

use accesskit::{ActionRequest, TreeUpdate};
use rustui_core::{
    BuildCx, Color, KeyCode, Modifiers, NodeId, NodeKind, PinchEvent, Point, PointerButton,
    PointerEvent, PointerPhase, Rect, Runtime, ScrollEvent, Size, View,
};
use rustui_layout::LayoutEngine;
use rustui_render::{
    DamageTracker, DisplayList, GlyphAtlas, InspectorOverlay, ResourceLoader, WgpuRenderer,
    emit_display_list, encode_rgba_to_png,
};
use rustui_text::{ParleyTextMeasurer, ParleyTextPainter, TextSystem};

use crate::accessibility::AccessKitBridge;
use crate::input::{FocusManager, GestureRecognizer, key_down};

/// Summary returned after executing one headless frame tick.
#[derive(Debug, Clone, Copy)]
pub struct FrameSummary {
    pub reactive_updates: usize,
    pub rebuilt_nodes: usize,
    pub laid_out_nodes: usize,
    pub effects_run: usize,
    pub paint_commands: usize,
    pub redrawn: bool,
}

/// Synthetic UI test driver running the exact 6-stage RustUI frame pipeline without opening a window.
pub struct HeadlessSimulator {
    pub runtime: Runtime,
    pub root: NodeId,
    pub viewport: Size<f32>,
    pub scale_factor: f32,
    pub title: String,
    pub clear_color: Color,
    pub focus: FocusManager,
    pub gestures: GestureRecognizer,
    pub layout_engine: LayoutEngine,
    pub text_system: TextSystem,
    pub atlas: GlyphAtlas,
    pub resources: ResourceLoader,
    pub damage: DamageTracker,
    pub inspector: InspectorOverlay,
    pub renderer: WgpuRenderer,
}

impl std::fmt::Debug for HeadlessSimulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeadlessSimulator")
            .field("viewport", &self.viewport)
            .field("scale_factor", &self.scale_factor)
            .field("title", &self.title)
            .finish()
    }
}

impl HeadlessSimulator {
    /// Mounts a [`View`] into a fresh [`Runtime`] and executes the initial layout frame.
    pub fn new<V: View>(view: V, viewport: Size<f32>) -> Self {
        let runtime = Runtime::new();
        let mut bcx = BuildCx::new(runtime.clone(), None);
        let root = runtime.enter(|| view.build_node(&mut bcx));
        runtime.with_inner_mut(|inner| {
            inner.root = Some(root);
        });
        let mut sim = Self {
            runtime,
            root,
            viewport,
            scale_factor: 1.0,
            title: "RustUI Headless".to_owned(),
            clear_color: Color::from_hex(0xf8fafc),
            focus: FocusManager::new(),
            gestures: GestureRecognizer::new(),
            layout_engine: LayoutEngine::new(),
            text_system: TextSystem::new(),
            atlas: GlyphAtlas::default(),
            resources: ResourceLoader::new(),
            damage: DamageTracker::new(),
            inspector: InspectorOverlay::new(false),
            renderer: WgpuRenderer::new(),
        };
        sim.step_frame();
        sim
    }

    pub fn with_scale_factor(mut self, scale_factor: f32) -> Self {
        self.scale_factor = scale_factor.max(1.0);
        self
    }

    pub fn with_inspector(mut self, enabled: bool) -> Self {
        self.inspector = InspectorOverlay::new(enabled);
        self
    }

    pub fn with_clear_color(mut self, color: Color) -> Self {
        self.clear_color = color;
        self
    }

    /// Executes one full frame pipeline (tasks -> signals -> animations -> layout -> effects).
    /// Returns `redrawn: false` when idle (zero redraws).
    pub fn step_frame(&mut self) -> FrameSummary {
        let frame_start = Instant::now();
        self.runtime.poll_tasks();
        self.runtime.tick_timers(Instant::now());
        let (reactive_updates, rebuilt_nodes) = self.runtime.flush_reactive();

        let _ = self.runtime.tick_animations(Instant::now());

        if !self.runtime.needs_redraw() && reactive_updates == 0 && rebuilt_nodes == 0 {
            return FrameSummary {
                reactive_updates: 0,
                rebuilt_nodes: 0,
                laid_out_nodes: 0,
                effects_run: 0,
                paint_commands: 0,
                redrawn: false,
            };
        }

        let mut measurer = ParleyTextMeasurer {
            system: &mut self.text_system,
            scale_factor: self.scale_factor,
        };
        let laid_out_nodes =
            self.layout_engine
                .layout(&self.runtime, self.root, self.viewport, &mut measurer);

        let effects_run = self.runtime.flush_effects();

        self.runtime.with_inner_mut(|inner| {
            inner.scheduler.clear_redraw();
            inner.scheduler.metrics.frame_number += 1;
            inner.scheduler.metrics.redraws_requested += 1;
            inner.scheduler.metrics.total_frame_time = frame_start.elapsed();
        });

        let paint_commands = self
            .runtime
            .with_inner(|inner| inner.scheduler.metrics.paint_commands);
        FrameSummary {
            reactive_updates,
            rebuilt_nodes,
            laid_out_nodes,
            effects_run,
            paint_commands,
            redrawn: true,
        }
    }

    /// Emits the current [`DisplayList`] (used for rendering or inspecting draw commands).
    pub fn build_display_list(&mut self) -> DisplayList {
        let viewport_rect = Rect::new(0.0, 0.0, self.viewport.width, self.viewport.height);
        self.damage.collect(&self.runtime, viewport_rect);
        let mut painter = ParleyTextPainter {
            system: &mut self.text_system,
            scale_factor: self.scale_factor,
        };
        let mut list = emit_display_list(
            &self.runtime,
            self.root,
            self.scale_factor,
            &mut self.resources,
            Some(&mut painter),
        );
        self.inspector
            .emit(&self.runtime, &self.damage, &mut list, viewport_rect);
        list
    }

    /// Renders the current frame to an RGBA8 pixel buffer.
    pub fn render_rgba(&mut self) -> Vec<u8> {
        self.step_frame();
        let list = self.build_display_list();
        let w = (self.viewport.width * self.scale_factor).round().max(1.0) as u32;
        let h = (self.viewport.height * self.scale_factor).round().max(1.0) as u32;
        self.renderer.render_offscreen(
            &list,
            &mut self.atlas,
            &self.resources,
            w,
            h,
            self.clear_color,
        )
    }

    /// Renders the current frame to PNG bytes.
    pub fn render_png(&mut self) -> Result<Vec<u8>, String> {
        let rgba = self.render_rgba();
        let w = (self.viewport.width * self.scale_factor).round().max(1.0) as u32;
        let h = (self.viewport.height * self.scale_factor).round().max(1.0) as u32;
        encode_rgba_to_png(&rgba, w, h)
    }

    /// Simulates a primary mouse click at `(x, y)` and flushes the frame.
    pub fn click(&mut self, x: f32, y: f32) -> FrameSummary {
        let pos = Point::new(x, y);
        self.gestures.handle_pointer(
            &self.runtime,
            self.root,
            &mut self.focus,
            PointerEvent {
                position: pos,
                button: PointerButton::Primary,
                phase: PointerPhase::Down,
                click_count: 1,
                modifiers: Modifiers::NONE,
            },
        );
        self.gestures.handle_pointer(
            &self.runtime,
            self.root,
            &mut self.focus,
            PointerEvent {
                position: pos,
                button: PointerButton::Primary,
                phase: PointerPhase::Up,
                click_count: 1,
                modifiers: Modifiers::NONE,
            },
        );
        self.step_frame()
    }

    /// Simulates clicking the center of `node_id`.
    pub fn click_node(&mut self, node_id: NodeId) -> FrameSummary {
        let b = self.node_bounds(node_id).expect("node has bounds");
        self.click(b.mid_x(), b.mid_y())
    }

    /// Simulates moving the pointer over `(x, y)` and flushes the frame.
    pub fn hover(&mut self, x: f32, y: f32) -> FrameSummary {
        self.gestures.handle_pointer(
            &self.runtime,
            self.root,
            &mut self.focus,
            PointerEvent {
                position: Point::new(x, y),
                button: PointerButton::Primary,
                phase: PointerPhase::Move,
                click_count: 0,
                modifiers: Modifiers::NONE,
            },
        );
        self.step_frame()
    }

    /// Simulates a secondary (right-button) click at `(x, y)` and flushes the frame.
    pub fn right_click(&mut self, x: f32, y: f32) -> FrameSummary {
        let pos = Point::new(x, y);
        self.gestures.handle_pointer(
            &self.runtime,
            self.root,
            &mut self.focus,
            PointerEvent {
                position: pos,
                button: PointerButton::Secondary,
                phase: PointerPhase::Down,
                click_count: 1,
                modifiers: Modifiers::NONE,
            },
        );
        self.gestures.handle_pointer(
            &self.runtime,
            self.root,
            &mut self.focus,
            PointerEvent {
                position: pos,
                button: PointerButton::Secondary,
                phase: PointerPhase::Up,
                click_count: 1,
                modifiers: Modifiers::NONE,
            },
        );
        self.step_frame()
    }

    /// Simulates a touch-down contact `id` at `(x, y)`.
    pub fn touch_down(&mut self, id: u64, x: f32, y: f32) -> FrameSummary {
        self.gestures.handle_touch(
            &self.runtime,
            self.root,
            &mut self.focus,
            id,
            PointerPhase::Down,
            Point::new(x, y),
            Modifiers::NONE,
        );
        self.step_frame()
    }

    /// Simulates a touch-move contact `id` to `(x, y)`.
    pub fn touch_move(&mut self, id: u64, x: f32, y: f32) -> FrameSummary {
        self.gestures.handle_touch(
            &self.runtime,
            self.root,
            &mut self.focus,
            id,
            PointerPhase::Move,
            Point::new(x, y),
            Modifiers::NONE,
        );
        self.step_frame()
    }

    /// Simulates a touch-up contact `id` at `(x, y)`.
    pub fn touch_up(&mut self, id: u64, x: f32, y: f32) -> FrameSummary {
        self.gestures.handle_touch(
            &self.runtime,
            self.root,
            &mut self.focus,
            id,
            PointerPhase::Up,
            Point::new(x, y),
            Modifiers::NONE,
        );
        self.step_frame()
    }

    /// Simulates a trackpad or touch pinch gesture centered at `position`.
    pub fn pinch(&mut self, position: Point, scale_delta: f32) -> FrameSummary {
        self.gestures.handle_pinch(
            &self.runtime,
            self.root,
            PinchEvent {
                position,
                scale_delta,
                phase: PointerPhase::Move,
            },
        );
        self.step_frame()
    }

    /// Sets reduced-motion mode on the runtime scheduler and advances one frame.
    pub fn set_reduced_motion(&mut self, reduced: bool) -> FrameSummary {
        self.runtime.set_reduced_motion(reduced);
        self.step_frame()
    }

    /// Simulates dropping OS files onto `position`.
    pub fn drop_files(&mut self, position: Point, paths: Vec<std::path::PathBuf>) -> FrameSummary {
        self.gestures.dispatch_drop(
            &self.runtime,
            self.root,
            rustui_core::DropEvent {
                position,
                payload: rustui_core::DragPayload::Files(paths),
            },
        );
        self.step_frame()
    }

    /// Simulates dragging from a draggable node at `from` and dropping onto a drop target at `to`.
    pub fn drag_and_drop(&mut self, from: Point, to: Point) -> FrameSummary {
        self.drag(from, to)
    }

    /// Simulates a drag gesture from `from` to `to`.
    pub fn drag(&mut self, from: Point, to: Point) -> FrameSummary {
        self.gestures.handle_pointer(
            &self.runtime,
            self.root,
            &mut self.focus,
            PointerEvent {
                position: from,
                button: PointerButton::Primary,
                phase: PointerPhase::Down,
                click_count: 1,
                modifiers: Modifiers::NONE,
            },
        );
        let mid = Point::new((from.x + to.x) * 0.5, (from.y + to.y) * 0.5);
        for pt in [mid, to] {
            self.gestures.handle_pointer(
                &self.runtime,
                self.root,
                &mut self.focus,
                PointerEvent {
                    position: pt,
                    button: PointerButton::Primary,
                    phase: PointerPhase::Move,
                    click_count: 1,
                    modifiers: Modifiers::NONE,
                },
            );
        }
        self.gestures.handle_pointer(
            &self.runtime,
            self.root,
            &mut self.focus,
            PointerEvent {
                position: to,
                button: PointerButton::Primary,
                phase: PointerPhase::Up,
                click_count: 1,
                modifiers: Modifiers::NONE,
            },
        );
        self.step_frame()
    }

    /// Simulates a scroll wheel / trackpad gesture.
    pub fn scroll(&mut self, position: Point, delta_x: f32, delta_y: f32) -> FrameSummary {
        self.gestures.handle_scroll(
            &self.runtime,
            self.root,
            ScrollEvent {
                position,
                delta_x,
                delta_y,
                is_momentum: false,
            },
        );
        self.step_frame()
    }

    /// Dispatches a synthetic key press.
    pub fn press_key(&mut self, key: KeyCode, modifiers: Modifiers) -> FrameSummary {
        let ev = key_down(key, modifiers, None);
        self.gestures
            .handle_key(&self.runtime, self.root, &mut self.focus, ev);
        self.step_frame()
    }

    /// Types text into the currently focused node character by character.
    pub fn type_text(&mut self, text: &str) -> FrameSummary {
        for ch in text.chars() {
            let s = ch.to_string();
            let ev = key_down(KeyCode::Character(s.clone()), Modifiers::NONE, Some(&s));
            self.gestures
                .handle_key(&self.runtime, self.root, &mut self.focus, ev);
        }
        self.step_frame()
    }

    /// Simulates IME pre-edit composition on the focused node.
    pub fn ime_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) -> FrameSummary {
        self.gestures
            .handle_ime_preedit(&self.runtime, &self.focus, text.to_owned(), cursor);
        self.step_frame()
    }

    /// Simulates IME commit on the focused node.
    pub fn ime_commit(&mut self, text: &str) -> FrameSummary {
        self.gestures
            .handle_ime_commit(&self.runtime, &self.focus, text.to_owned());
        self.step_frame()
    }

    pub fn focus_next(&mut self) -> Option<NodeId> {
        let next = self.focus.focus_next(&self.runtime, self.root);
        self.step_frame();
        next
    }

    pub fn focus_prev(&mut self) -> Option<NodeId> {
        let prev = self.focus.focus_prev(&self.runtime, self.root);
        self.step_frame();
        prev
    }

    pub fn focused_node(&self) -> Option<NodeId> {
        self.focus.focused_node()
    }

    pub fn accessibility_tree(&self) -> TreeUpdate {
        AccessKitBridge::build_tree_update(
            &self.runtime,
            self.root,
            &self.title,
            self.focus.focused_node(),
        )
    }

    pub fn dispatch_accessibility_action(&mut self, req: ActionRequest) -> bool {
        let handled = AccessKitBridge::handle_action_request(&self.runtime, &mut self.focus, &req);
        self.step_frame();
        handled
    }

    pub fn node_bounds(&self, id: NodeId) -> Option<Rect> {
        self.runtime
            .with_inner(|inner| inner.nodes.get(id).map(|n| n.bounds()))
    }

    /// Finds the first node whose text or accessibility label matches `needle`.
    pub fn find_node_by_label(&self, needle: &str) -> Option<NodeId> {
        self.runtime.with_inner(|inner| {
            for (id, node) in &inner.nodes {
                if node.accessibility.label.as_deref() == Some(needle) {
                    return Some(id);
                }
                if let NodeKind::Text(t) = &node.kind
                    && t.text == needle
                {
                    return Some(id);
                }
            }
            None
        })
    }
}
