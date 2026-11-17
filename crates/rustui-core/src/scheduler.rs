//! Frame scheduler, animations, reduced-motion handling, and frame metrics.

use std::cell::Cell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::node::NodeId;
use crate::types::{Animation, Color, Point, Rect};

thread_local! {
    static ACTIVE_ANIMATION: Cell<Option<Animation>> = const { Cell::new(None) };
}

/// Runs `f` with an implicit [`Animation`] context (`with_animation`).
///
/// Any animatable state mutations performed inside `f` trigger transitions using `animation`
/// unless `reduced_motion` is enabled in the scheduler.
pub fn with_animation<R>(animation: Animation, f: impl FnOnce() -> R) -> R {
    let prev = ACTIVE_ANIMATION.with(|c| {
        let old = c.get();
        c.set(Some(animation));
        old
    });
    struct Guard(Option<Animation>);
    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE_ANIMATION.with(|c| c.set(self.0));
        }
    }
    let _guard = Guard(prev);
    f()
}

/// Returns the active implicit animation if currently inside [`with_animation`].
pub fn current_animation() -> Option<Animation> {
    ACTIVE_ANIMATION.with(|c| c.get())
}

/// Key identifying an animatable property on a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimProperty {
    Opacity,
    OffsetX,
    OffsetY,
    Scale,
    Color,
    Bounds,
    CornerRadius,
}

/// Value interpolated by an active animation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimValue {
    Scalar(f32),
    Color(Color),
    Rect(Rect),
    Point(Point),
}

impl AnimValue {
    pub fn lerp(self, target: Self, t: f32) -> Self {
        match (self, target) {
            (Self::Scalar(a), Self::Scalar(b)) => Self::Scalar(a + (b - a) * t),
            (Self::Color(a), Self::Color(b)) => Self::Color(a.lerp(b, t)),
            (Self::Rect(a), Self::Rect(b)) => Self::Rect(Rect::new(
                a.x + (b.x - a.x) * t,
                a.y + (b.y - a.y) * t,
                a.width + (b.width - a.width) * t,
                a.height + (b.height - a.height) * t,
            )),
            (Self::Point(a), Self::Point(b)) => {
                Self::Point(Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t))
            }
            (_, other) => other,
        }
    }
}

/// Running property animation tracked by the [`Scheduler`].
#[derive(Debug, Clone)]
pub struct ActiveAnimation {
    pub node: NodeId,
    pub property: AnimProperty,
    pub from: AnimValue,
    pub to: AnimValue,
    pub current: AnimValue,
    pub animation: Animation,
    pub start_time: Instant,
}

/// Performance and diagnostic metrics captured per frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameMetrics {
    pub frame_number: u64,
    pub reactive_updates: usize,
    pub rebuilt_nodes: usize,
    pub laid_out_nodes: usize,
    pub layout_cache_hits: usize,
    pub layout_cache_misses: usize,
    pub fixed_boundary_stops: usize,
    pub paint_commands: usize,
    pub signal_update_time: Duration,
    pub layout_time: Duration,
    pub paint_time: Duration,
    pub total_frame_time: Duration,
    pub redraws_requested: u64,
}

/// Drives frame timing, animations, and idle redraw suppression.
#[derive(Debug)]
pub struct Scheduler {
    pub reduced_motion: bool,
    pub animations: HashMap<(NodeId, AnimProperty), ActiveAnimation>,
    pub redraw_needed: bool,
    pub metrics: FrameMetrics,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            reduced_motion: false,
            animations: HashMap::new(),
            redraw_needed: true,
            metrics: FrameMetrics::default(),
        }
    }

    pub fn request_redraw(&mut self) {
        self.redraw_needed = true;
    }

    pub fn clear_redraw(&mut self) {
        self.redraw_needed = false;
    }

    pub fn needs_redraw(&self) -> bool {
        self.redraw_needed || !self.animations.is_empty()
    }

    pub fn start_animation(
        &mut self,
        node: NodeId,
        property: AnimProperty,
        from: AnimValue,
        to: AnimValue,
        animation: Animation,
        now: Instant,
    ) {
        if self.reduced_motion {
            self.animations.remove(&(node, property));
            return;
        }
        self.animations.insert(
            (node, property),
            ActiveAnimation {
                node,
                property,
                from,
                to,
                current: from,
                animation,
                start_time: now,
            },
        );
        self.redraw_needed = true;
    }

    /// Advances all active animations to `now`, returning nodes that changed this tick.
    pub fn tick_animations(&mut self, now: Instant) -> Vec<(NodeId, AnimProperty, AnimValue)> {
        if self.animations.is_empty() {
            return Vec::new();
        }
        let mut updates = Vec::new();
        let mut finished = Vec::new();

        for (&key, anim) in &mut self.animations {
            let elapsed = now.saturating_duration_since(anim.start_time).as_secs_f32();
            let (t, is_done) = anim.animation.evaluate(elapsed);
            let value = if is_done {
                anim.to
            } else {
                anim.from.lerp(anim.to, t)
            };
            anim.current = value;
            updates.push((key.0, key.1, value));
            if is_done {
                finished.push(key);
            }
        }

        for key in finished {
            self.animations.remove(&key);
        }

        if !self.animations.is_empty() {
            self.redraw_needed = true;
        }

        updates
    }

    pub fn remove_node_animations(&mut self, node: NodeId) {
        self.animations.retain(|(n, _), _| *n != node);
    }
}
