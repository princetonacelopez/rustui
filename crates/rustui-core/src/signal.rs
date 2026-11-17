//! Fine-grained reactive signals (`State`, `Binding`, `Memo`, `Effect`, `Observable`)
//! and the single-UI-thread `Runtime` arena.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use slotmap::{SlotMap, new_key_type};

use crate::node::{DirtyFlags, ForEachRebuilderFn, Node, NodeBuilderFn, NodeId, SwitchBuilderFn};
use crate::scheduler::{AnimProperty, AnimValue, Scheduler, current_animation};

static NEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);

new_key_type! {
    /// Arena key for a reactive [`State`] slot.
    pub struct StateId;
    /// Arena key for a derived [`Memo`] slot.
    pub struct MemoId;
    /// Arena key for a post-layout [`EffectHandle`] slot.
    pub struct EffectId;
    /// Arena key for a reactive node updater binding.
    pub struct ReactiveId;
    /// Arena key for a managed [`TimerId`] scoped to a runtime or node.
    pub struct TimerId;
}

/// Identifies any reactive signal source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignalKey {
    State(StateId),
    Memo(MemoId),
}

/// Subscriber notified when a signal changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subscriber {
    Memo(MemoId),
    Effect(EffectId),
    NodeUpdater(ReactiveId),
    NodeRebuild(NodeId),
}

#[derive(Debug, Clone, Copy)]
enum TrackingTarget {
    Memo(MemoId),
    Effect(EffectId),
    NodeUpdater(ReactiveId),
    NodeRebuild(NodeId),
}

struct StateEntry {
    value: Option<Box<dyn Any>>,
    subscribers: Vec<Subscriber>,
    version: u64,
}

struct MemoEntry {
    compute: Rc<dyn Fn() -> Box<dyn Any>>,
    cached: Option<Box<dyn Any>>,
    dirty: bool,
    depth: u32,
    deps: Vec<SignalKey>,
    subscribers: Vec<Subscriber>,
    version: u64,
}

type EffectRunFn = Rc<RefCell<dyn FnMut() -> Option<Box<dyn FnOnce()>>>>;

struct EffectEntry {
    run: EffectRunFn,
    cleanup: Option<Box<dyn FnOnce()>>,
    deps: Vec<SignalKey>,
}

struct TimerEntry {
    interval: Duration,
    repeating: bool,
    next_fire: Instant,
    callback: Rc<dyn Fn()>,
}

/// Type-erased reactive property applicator closure.
pub type ReactiveApplyFn = Rc<dyn Fn(&mut Node, Box<dyn Any>) -> DirtyFlags>;
type ReactiveInnerPostFn = Rc<dyn Fn(&mut RuntimeInner, NodeId)>;
type SpawnCallback = Box<dyn FnOnce(Box<dyn Any + Send>)>;

/// Computes a new typed value from reactive signals and applies it to a retained [`Node`].
pub struct NodeReactiveEntry {
    pub node: NodeId,
    pub compute: Rc<dyn Fn() -> Box<dyn Any>>,
    pub apply: ReactiveApplyFn,
    post_inner: Option<ReactiveInnerPostFn>,
    pub deps: Vec<SignalKey>,
}

impl std::fmt::Debug for NodeReactiveEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeReactiveEntry")
            .field("node", &self.node)
            .field("deps", &self.deps)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirtyMemoItem {
    depth: u32,
    id: MemoId,
}

impl Ord for DirtyMemoItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Min-heap by depth (lowest depth runs first for glitch-free topological order).
        other.depth.cmp(&self.depth)
    }
}

impl PartialOrd for DirtyMemoItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

type UiTask = Box<dyn FnOnce(&Runtime) + Send>;

pub struct RuntimeInner {
    pub id: u64,
    pub nodes: SlotMap<NodeId, Node>,
    pub root: Option<NodeId>,
    states: SlotMap<StateId, StateEntry>,
    memos: SlotMap<MemoId, MemoEntry>,
    effects: SlotMap<EffectId, EffectEntry>,
    timers: SlotMap<TimerId, TimerEntry>,
    reactives: SlotMap<ReactiveId, NodeReactiveEntry>,
    tracking_stack: Vec<(TrackingTarget, Vec<SignalKey>)>,
    evaluating_memos: Vec<MemoId>,
    dirty_memos: BinaryHeap<DirtyMemoItem>,
    dirty_memo_set: HashSet<MemoId>,
    pending_updaters: Vec<ReactiveId>,
    pending_updater_set: HashSet<ReactiveId>,
    pending_rebuilds: Vec<NodeId>,
    pending_rebuild_set: HashSet<NodeId>,
    pending_effects: Vec<EffectId>,
    pending_effect_set: HashSet<EffectId>,
    pub scheduler: Scheduler,
    pub high_contrast: bool,
    pub forced_colors: bool,
    pub text_scale_factor: f32,
    pub hot_reload_diagnostics: Vec<String>,
    task_tx: mpsc::Sender<UiTask>,
    task_rx: mpsc::Receiver<UiTask>,
    wake_callback: Option<Arc<dyn Fn() + Send + Sync>>,
    next_updater_uid: u64,
    spawn_callbacks: HashMap<u64, SpawnCallback>,
    next_spawn_id: u64,
}

impl std::fmt::Debug for RuntimeInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeInner")
            .field("id", &self.id)
            .field("nodes", &self.nodes.len())
            .field("states", &self.states.len())
            .finish()
    }
}

impl RuntimeInner {
    fn new() -> Self {
        let (task_tx, task_rx) = mpsc::channel();
        Self {
            id: NEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed),
            nodes: SlotMap::with_key(),
            root: None,
            states: SlotMap::with_key(),
            memos: SlotMap::with_key(),
            effects: SlotMap::with_key(),
            timers: SlotMap::with_key(),
            reactives: SlotMap::with_key(),
            tracking_stack: Vec::new(),
            evaluating_memos: Vec::new(),
            dirty_memos: BinaryHeap::new(),
            dirty_memo_set: HashSet::new(),
            pending_updaters: Vec::new(),
            pending_updater_set: HashSet::new(),
            pending_rebuilds: Vec::new(),
            pending_rebuild_set: HashSet::new(),
            pending_effects: Vec::new(),
            pending_effect_set: HashSet::new(),
            scheduler: Scheduler::new(),
            high_contrast: false,
            forced_colors: false,
            text_scale_factor: 1.0,
            hot_reload_diagnostics: Vec::new(),
            task_tx,
            task_rx,
            wake_callback: None,
            next_updater_uid: 1,
            spawn_callbacks: HashMap::new(),
            next_spawn_id: 1,
        }
    }

    fn record_dependency(&mut self, key: SignalKey) -> Option<Subscriber> {
        let (target, deps) = self.tracking_stack.last_mut()?;
        if !deps.contains(&key) {
            deps.push(key);
        }
        let sub = match *target {
            TrackingTarget::Memo(id) => Subscriber::Memo(id),
            TrackingTarget::Effect(id) => Subscriber::Effect(id),
            TrackingTarget::NodeUpdater(id) => Subscriber::NodeUpdater(id),
            TrackingTarget::NodeRebuild(id) => Subscriber::NodeRebuild(id),
        };
        Some(sub)
    }

    fn add_subscriber_to_signal(&mut self, key: SignalKey, sub: Subscriber) {
        match key {
            SignalKey::State(id) => {
                if let Some(entry) = self.states.get_mut(id)
                    && !entry.subscribers.contains(&sub)
                {
                    entry.subscribers.push(sub);
                }
            }
            SignalKey::Memo(id) => {
                if let Some(entry) = self.memos.get_mut(id)
                    && !entry.subscribers.contains(&sub)
                {
                    entry.subscribers.push(sub);
                }
            }
        }
    }

    fn remove_subscriber_from_signal(&mut self, key: SignalKey, sub: Subscriber) {
        match key {
            SignalKey::State(id) => {
                if let Some(entry) = self.states.get_mut(id) {
                    entry.subscribers.retain(|s| *s != sub);
                }
            }
            SignalKey::Memo(id) => {
                if let Some(entry) = self.memos.get_mut(id) {
                    entry.subscribers.retain(|s| *s != sub);
                }
            }
        }
    }

    fn notify_signal(&mut self, key: SignalKey) {
        let subs = match key {
            SignalKey::State(id) => self
                .states
                .get(id)
                .map(|s| s.subscribers.clone())
                .unwrap_or_default(),
            SignalKey::Memo(id) => self
                .memos
                .get(id)
                .map(|m| m.subscribers.clone())
                .unwrap_or_default(),
        };

        for sub in subs {
            match sub {
                Subscriber::Memo(memo_id) => {
                    if let Some(memo) = self.memos.get_mut(memo_id) {
                        let was_dirty = memo.dirty;
                        memo.dirty = true;
                        let depth = memo.depth;
                        if !was_dirty && self.dirty_memo_set.insert(memo_id) {
                            self.dirty_memos.push(DirtyMemoItem { depth, id: memo_id });
                        }
                    }
                    self.notify_signal(SignalKey::Memo(memo_id));
                }
                Subscriber::Effect(effect_id) => {
                    if self.pending_effect_set.insert(effect_id) {
                        self.pending_effects.push(effect_id);
                    }
                }
                Subscriber::NodeUpdater(reactive_id) => {
                    if self.pending_updater_set.insert(reactive_id) {
                        self.pending_updaters.push(reactive_id);
                    }
                    self.scheduler.request_redraw();
                }
                Subscriber::NodeRebuild(node_id) => {
                    if self.pending_rebuild_set.insert(node_id) {
                        self.pending_rebuilds.push(node_id);
                    }
                    self.mark_dirty(
                        node_id,
                        DirtyFlags::REBUILD | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
                    );
                }
            }
        }
    }

    pub fn mark_dirty(&mut self, node_id: NodeId, flags: DirtyFlags) {
        let Some(node) = self.nodes.get_mut(node_id) else {
            return;
        };
        node.dirty.insert(flags);
        if flags.intersects(DirtyFlags::PAINT) {
            node.paint_dirty_generation = node.paint_dirty_generation.wrapping_add(1);
        }
        self.scheduler.request_redraw();

        let had_layout = flags.intersects(DirtyFlags::LAYOUT);
        let mut propagate_layout = had_layout;
        let propagate_paint = flags.intersects(DirtyFlags::PAINT);
        let propagate_a11y = flags.intersects(DirtyFlags::ACCESSIBILITY);
        let mut cursor = node.parent;
        if node.layout_props.is_fixed_size_boundary() {
            propagate_layout = false;
            self.scheduler.metrics.fixed_boundary_stops = self
                .scheduler
                .metrics
                .fixed_boundary_stops
                .saturating_add(1);
        }

        while let Some(parent_id) = cursor {
            let Some(parent) = self.nodes.get_mut(parent_id) else {
                break;
            };
            if propagate_layout {
                parent.dirty.insert(DirtyFlags::LAYOUT | DirtyFlags::PAINT);
                if parent.layout_props.is_fixed_size_boundary() {
                    propagate_layout = false;
                    self.scheduler.metrics.fixed_boundary_stops = self
                        .scheduler
                        .metrics
                        .fixed_boundary_stops
                        .saturating_add(1);
                }
            } else {
                if had_layout {
                    parent.dirty.insert(DirtyFlags::DESCENDANT_LAYOUT);
                }
                if propagate_paint {
                    parent.dirty.insert(DirtyFlags::PAINT);
                }
            }
            if propagate_a11y {
                parent.dirty.insert(DirtyFlags::ACCESSIBILITY);
            }
            if propagate_paint {
                parent.paint_dirty_generation = parent.paint_dirty_generation.wrapping_add(1);
            }
            cursor = parent.parent;
        }
    }
}

thread_local! {
    static ACTIVE_RUNTIME: Cell<Option<*const RefCell<RuntimeInner>>> = const { Cell::new(None) };
}

/// Handle to the RustUI reactive and node-tree runtime.
///
/// There is no hidden global state: all nodes, signals, memos, effects, and scheduler
/// state live inside this [`Runtime`] instance owned by the application or test.
#[derive(Clone)]
pub struct Runtime {
    inner: Rc<RefCell<RuntimeInner>>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.borrow();
        f.debug_struct("Runtime")
            .field("id", &inner.id)
            .field("nodes", &inner.nodes.len())
            .field("states", &inner.states.len())
            .field("memos", &inner.memos.len())
            .finish()
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    /// Creates an isolated [`Runtime`] instance.
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(RuntimeInner::new())),
        }
    }

    /// Returns the unique runtime identifier.
    pub fn id(&self) -> u64 {
        self.inner.borrow().id
    }

    /// Sets a wake callback invoked when background tasks deliver messages via `cx.spawn`.
    pub fn set_wake_callback(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.inner.borrow_mut().wake_callback = Some(wake);
    }

    /// Returns a cloneable sender for delivering state-update closures from background threads.
    pub fn task_sender(&self) -> TaskSender {
        let inner = self.inner.borrow();
        TaskSender {
            tx: inner.task_tx.clone(),
            wake: inner.wake_callback.clone(),
        }
    }

    /// Enters this runtime's scope on the UI thread for the duration of `f`.
    pub fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        let ptr: *const RefCell<RuntimeInner> = Rc::as_ptr(&self.inner);
        let prev = ACTIVE_RUNTIME.with(|c| {
            let old = c.get();
            c.set(Some(ptr));
            old
        });
        struct Guard(Option<*const RefCell<RuntimeInner>>);
        impl Drop for Guard {
            fn drop(&mut self) {
                ACTIVE_RUNTIME.with(|c| c.set(self.0));
            }
        }
        let _guard = Guard(prev);
        f()
    }

    fn with_current<R>(expected_id: u64, f: impl FnOnce(&Runtime) -> R) -> R {
        let ptr = ACTIVE_RUNTIME
            .with(|c| c.get())
            .expect("RustUI signal accessed outside an active Runtime::enter scope");
        // SAFETY: `Runtime::enter` sets `ACTIVE_RUNTIME` only for the lifetime of `&self` on this thread,
        // and restores the previous value on drop (including panic unwind).
        let rc = unsafe {
            Rc::increment_strong_count(ptr);
            Rc::from_raw(ptr)
        };
        let rt = Runtime { inner: rc };
        assert_eq!(
            rt.id(),
            expected_id,
            "RustUI signal belongs to Runtime {}, but Runtime {} is active",
            expected_id,
            rt.id()
        );
        f(&rt)
    }

    /// Provides mutable access to [`RuntimeInner`] for layout/render/platform crates.
    pub fn with_inner_mut<R>(&self, f: impl FnOnce(&mut RuntimeInner) -> R) -> R {
        let mut guard = self.inner.borrow_mut();
        f(&mut guard)
    }

    /// Provides shared access to [`RuntimeInner`].
    pub fn with_inner<R>(&self, f: impl FnOnce(&RuntimeInner) -> R) -> R {
        let guard = self.inner.borrow();
        f(&guard)
    }

    /// Creates a new state slot owned optionally by `owner`.
    pub fn create_state<T: 'static>(&self, value: T, owner: Option<NodeId>) -> State<T> {
        let mut inner = self.inner.borrow_mut();
        let id = inner.states.insert(StateEntry {
            value: Some(Box::new(value)),
            subscribers: Vec::new(),
            version: 0,
        });
        if let Some(owner_id) = owner
            && let Some(node) = inner.nodes.get_mut(owner_id)
        {
            node.owned_states.push(id);
        }
        State {
            id,
            runtime_id: inner.id,
            _marker: PhantomData,
        }
    }

    /// Reads a state value with dependency tracking if inside a reactive scope.
    pub fn get_state<T: Clone + 'static>(&self, state: State<T>) -> T {
        let mut inner = self.inner.borrow_mut();
        assert_eq!(
            inner.id, state.runtime_id,
            "State accessed from wrong Runtime"
        );
        let val = inner
            .states
            .get(state.id)
            .and_then(|s| s.value.as_ref())
            .and_then(|b| b.downcast_ref::<T>())
            .cloned()
            .expect("Attempted to read a disposed or mismatched State<T>");
        let key = SignalKey::State(state.id);
        if let Some(sub) = inner.record_dependency(key) {
            inner.add_subscriber_to_signal(key, sub);
        }
        val
    }

    /// Reads a state value without subscribing to future changes.
    pub fn get_state_untracked<T: Clone + 'static>(&self, state: State<T>) -> T {
        let inner = self.inner.borrow();
        assert_eq!(
            inner.id, state.runtime_id,
            "State accessed from wrong Runtime"
        );
        inner
            .states
            .get(state.id)
            .and_then(|s| s.value.as_ref())
            .and_then(|b| b.downcast_ref::<T>())
            .cloned()
            .expect("Attempted to read a disposed or mismatched State<T>")
    }

    /// Replaces a state value and notifies subscribed memos, updaters, rebuilds, and effects.
    pub fn set_state<T: 'static>(&self, state: State<T>, value: T) {
        let mut inner = self.inner.borrow_mut();
        assert_eq!(
            inner.id, state.runtime_id,
            "State mutated from wrong Runtime"
        );
        if let Some(entry) = inner.states.get_mut(state.id) {
            entry.value = Some(Box::new(value));
            entry.version = entry.version.wrapping_add(1);
            inner.notify_signal(SignalKey::State(state.id));
        }
    }

    /// Mutates a state value in-place and notifies subscribers.
    pub fn update_state<T: 'static>(&self, state: State<T>, f: impl FnOnce(&mut T)) {
        let mut boxed = {
            let mut inner = self.inner.borrow_mut();
            assert_eq!(
                inner.id, state.runtime_id,
                "State updated from wrong Runtime"
            );
            inner
                .states
                .get_mut(state.id)
                .and_then(|e| e.value.take())
                .expect("Attempted to update a disposed or re-entrant State<T>")
        };

        let val_ref = boxed
            .downcast_mut::<T>()
            .expect("State<T> type mismatch during update");
        f(val_ref);

        let mut inner = self.inner.borrow_mut();
        if let Some(entry) = inner.states.get_mut(state.id) {
            entry.value = Some(boxed);
            entry.version = entry.version.wrapping_add(1);
            inner.notify_signal(SignalKey::State(state.id));
        }
    }

    /// Creates a derived [`Memo`] computed lazily/topologically and optionally owned by `owner`.
    pub fn create_memo<T: Clone + 'static>(
        &self,
        compute: impl Fn() -> T + 'static,
        owner: Option<NodeId>,
    ) -> Memo<T> {
        let compute_erased: Rc<dyn Fn() -> Box<dyn Any>> =
            Rc::new(move || Box::new(compute()) as Box<dyn Any>);
        let id = {
            let mut inner = self.inner.borrow_mut();
            let id = inner.memos.insert(MemoEntry {
                compute: compute_erased,
                cached: None,
                dirty: true,
                depth: 1,
                deps: Vec::new(),
                subscribers: Vec::new(),
                version: 0,
            });
            if let Some(owner_id) = owner
                && let Some(node) = inner.nodes.get_mut(owner_id)
            {
                node.owned_memos.push(id);
            }
            id
        };
        self.recompute_memo(id);
        let runtime_id = self.id();
        Memo {
            id,
            runtime_id,
            _marker: PhantomData,
        }
    }

    /// Reads a [`Memo`] with tracking if inside a reactive scope.
    pub fn get_memo<T: Clone + 'static>(&self, memo: Memo<T>) -> T {
        let needs_recompute = {
            let inner = self.inner.borrow();
            assert_eq!(
                inner.id, memo.runtime_id,
                "Memo accessed from wrong Runtime"
            );
            inner
                .memos
                .get(memo.id)
                .map(|m| m.dirty || m.cached.is_none())
                .expect("Disposed Memo<T>")
        };
        if needs_recompute {
            self.recompute_memo(memo.id);
        }
        let mut inner = self.inner.borrow_mut();
        let val = inner
            .memos
            .get(memo.id)
            .and_then(|m| m.cached.as_ref())
            .and_then(|b| b.downcast_ref::<T>())
            .cloned()
            .expect("Memo<T> value unavailable");
        let key = SignalKey::Memo(memo.id);
        if let Some(sub) = inner.record_dependency(key) {
            inner.add_subscriber_to_signal(key, sub);
        }
        val
    }

    /// Reads a [`Memo`] without subscribing to it.
    pub fn get_memo_untracked<T: Clone + 'static>(&self, memo: Memo<T>) -> T {
        let needs_recompute = {
            let inner = self.inner.borrow();
            assert_eq!(
                inner.id, memo.runtime_id,
                "Memo accessed from wrong Runtime"
            );
            inner
                .memos
                .get(memo.id)
                .map(|m| m.dirty || m.cached.is_none())
                .expect("Disposed Memo<T>")
        };
        if needs_recompute {
            self.recompute_memo(memo.id);
        }
        let inner = self.inner.borrow();
        inner
            .memos
            .get(memo.id)
            .and_then(|m| m.cached.as_ref())
            .and_then(|b| b.downcast_ref::<T>())
            .cloned()
            .expect("Memo<T> value unavailable")
    }

    fn recompute_memo(&self, memo_id: MemoId) {
        let (compute, old_deps) = {
            let mut inner = self.inner.borrow_mut();
            if inner.evaluating_memos.contains(&memo_id) {
                let cycle: Vec<String> = inner
                    .evaluating_memos
                    .iter()
                    .map(|id| format!("{id:?}"))
                    .collect();
                panic!(
                    "Memo dependency cycle detected: [{} -> {memo_id:?}]",
                    cycle.join(" -> ")
                );
            }
            let Some(entry) = inner.memos.get_mut(memo_id) else {
                return;
            };
            let compute = entry.compute.clone();
            let old_deps = std::mem::take(&mut entry.deps);
            for dep in &old_deps {
                inner.remove_subscriber_from_signal(*dep, Subscriber::Memo(memo_id));
            }
            inner.evaluating_memos.push(memo_id);
            inner
                .tracking_stack
                .push((TrackingTarget::Memo(memo_id), Vec::new()));
            (compute, old_deps)
        };
        let _ = old_deps;

        struct EvalGuard<'a> {
            rt: &'a Runtime,
            memo_id: MemoId,
        }
        impl Drop for EvalGuard<'_> {
            fn drop(&mut self) {
                let mut inner = self.rt.inner.borrow_mut();
                if let Some(pos) = inner
                    .evaluating_memos
                    .iter()
                    .rposition(|id| *id == self.memo_id)
                {
                    inner.evaluating_memos.remove(pos);
                }
            }
        }
        let _guard = EvalGuard { rt: self, memo_id };

        let value = self.enter(|| compute());

        let mut inner = self.inner.borrow_mut();
        let (_, new_deps) = inner
            .tracking_stack
            .pop()
            .expect("tracking_stack imbalanced during memo recompute");
        let mut max_depth = 0u32;
        for dep in &new_deps {
            if let SignalKey::Memo(up_id) = dep
                && let Some(up) = inner.memos.get(*up_id)
            {
                max_depth = max_depth.max(up.depth);
            }
        }
        if let Some(entry) = inner.memos.get_mut(memo_id) {
            entry.cached = Some(value);
            entry.dirty = false;
            entry.depth = max_depth + 1;
            entry.deps = new_deps;
            entry.version = entry.version.wrapping_add(1);
        }
        inner.dirty_memo_set.remove(&memo_id);
    }

    /// Creates a post-layout side [`EffectHandle`] owned optionally by `owner`.
    ///
    /// Rule: Effects run after layout, never during build.
    pub fn create_effect(
        &self,
        mut f: impl FnMut() + 'static,
        owner: Option<NodeId>,
    ) -> EffectHandle {
        self.create_effect_cleanup(
            move || {
                f();
                None
            },
            owner,
        )
    }

    /// Creates a post-layout side [`EffectHandle`] returning an optional cleanup closure
    /// that runs before re-execution and when `owner` is disposed.
    pub fn create_effect_cleanup(
        &self,
        f: impl FnMut() -> Option<Box<dyn FnOnce()>> + 'static,
        owner: Option<NodeId>,
    ) -> EffectHandle {
        let mut inner = self.inner.borrow_mut();
        let run: EffectRunFn = Rc::new(RefCell::new(f));
        let id = inner.effects.insert(EffectEntry {
            run,
            cleanup: None,
            deps: Vec::new(),
        });
        if let Some(owner_id) = owner
            && let Some(node) = inner.nodes.get_mut(owner_id)
        {
            node.owned_effects.push(id);
        }
        if inner.pending_effect_set.insert(id) {
            inner.pending_effects.push(id);
        }
        EffectHandle {
            id,
            runtime_id: inner.id,
        }
    }

    /// Schedules a managed one-shot or repeating timer scoped optionally to `owner`.
    pub fn create_timer(
        &self,
        interval: Duration,
        repeating: bool,
        callback: impl Fn() + 'static,
        owner: Option<NodeId>,
    ) -> TimerId {
        let mut inner = self.inner.borrow_mut();
        let id = inner.timers.insert(TimerEntry {
            interval,
            repeating,
            next_fire: Instant::now() + interval,
            callback: Rc::new(callback),
        });
        if let Some(owner_id) = owner
            && let Some(node) = inner.nodes.get_mut(owner_id)
        {
            node.owned_timers.push(id);
        }
        id
    }

    /// Cancels and removes a managed timer.
    pub fn remove_timer(&self, timer_id: TimerId) {
        self.inner.borrow_mut().timers.remove(timer_id);
    }

    /// Fires any timers whose deadline `<= now`, rescheduling repeating ones and pruning one-shots.
    pub fn tick_timers(&self, now: Instant) -> usize {
        let mut due_callbacks: Vec<Rc<dyn Fn()>> = Vec::new();
        let mut remove_ids = Vec::new();
        {
            let mut inner = self.inner.borrow_mut();
            for (id, entry) in &mut inner.timers {
                if now >= entry.next_fire {
                    due_callbacks.push(entry.callback.clone());
                    if entry.repeating {
                        let step = entry.interval.max(Duration::from_millis(1));
                        while entry.next_fire <= now {
                            entry.next_fire += step;
                        }
                    } else {
                        remove_ids.push(id);
                    }
                }
            }
            for id in remove_ids {
                inner.timers.remove(id);
            }
        }
        let fired = due_callbacks.len();
        for cb in due_callbacks {
            self.enter(|| cb());
        }
        fired
    }

    /// Returns whether high-contrast mode is enabled on this runtime.
    pub fn is_high_contrast(&self) -> bool {
        self.inner.borrow().high_contrast
    }

    /// Sets high-contrast accessibility mode and marks the root subtree for repaint.
    pub fn set_high_contrast(&self, enabled: bool) {
        let mut inner = self.inner.borrow_mut();
        if inner.high_contrast != enabled {
            inner.high_contrast = enabled;
            if let Some(root) = inner.root {
                inner.mark_dirty(root, DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY);
            }
        }
    }

    /// Returns whether forced-colors accessibility mode is active.
    pub fn is_forced_colors(&self) -> bool {
        self.inner.borrow().forced_colors
    }

    /// Sets forced-colors accessibility mode.
    pub fn set_forced_colors(&self, enabled: bool) {
        let mut inner = self.inner.borrow_mut();
        if inner.forced_colors != enabled {
            inner.forced_colors = enabled;
            if let Some(root) = inner.root {
                inner.mark_dirty(root, DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY);
            }
        }
    }

    /// Returns the system/app text scaling multiplier (`1.0` default, up to `2.5`).
    pub fn text_scale_factor(&self) -> f32 {
        self.inner.borrow().text_scale_factor
    }

    /// Updates the text scaling multiplier (`0.75..=2.5`) and marks the tree for re-layout.
    pub fn set_text_scale_factor(&self, scale: f32) {
        let clamped = scale.clamp(0.75, 2.5);
        let mut inner = self.inner.borrow_mut();
        if (inner.text_scale_factor - clamped).abs() > f32::EPSILON {
            inner.text_scale_factor = clamped;
            let all_nodes: Vec<NodeId> = inner.nodes.keys().collect();
            for nid in all_nodes {
                if let Some(n) = inner.nodes.get_mut(nid)
                    && matches!(n.kind, crate::node::NodeKind::Text(_))
                {
                    n.dirty
                        .insert(DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY);
                }
            }
            if let Some(root) = inner.root {
                inner.mark_dirty(
                    root,
                    DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY,
                );
            }
        }
    }

    /// Registers a reactive node updater closure that runs initially and whenever its dependencies change.
    pub fn register_node_updater<T: 'static>(
        &self,
        node: NodeId,
        compute: impl Fn() -> T + 'static,
        apply: impl Fn(&mut Node, T) -> DirtyFlags + 'static,
    ) -> ReactiveId {
        self.register_updater_internal(node, compute, apply, None)
    }

    /// Registers a reactive environment updater that propagates `T` to `node` and its subtree.
    pub fn register_env_updater<T: Clone + 'static>(
        &self,
        node: NodeId,
        compute: impl Fn() -> T + 'static,
    ) -> ReactiveId {
        let post: ReactiveInnerPostFn = Rc::new(|inner, node_id| {
            let val = inner.nodes.get(node_id).and_then(|n| n.env.get::<T>());
            if let Some(val) = val {
                let mut queue = inner
                    .nodes
                    .get(node_id)
                    .map(|n| n.children.clone())
                    .unwrap_or_default();
                while let Some(cid) = queue.pop() {
                    if let Some(c) = inner.nodes.get_mut(cid) {
                        c.env.insert(val.clone());
                        c.dirty.insert(DirtyFlags::PAINT);
                        queue.extend(c.children.iter().copied());
                    }
                }
            }
        });
        self.register_updater_internal(
            node,
            compute,
            |n, val| {
                n.env.insert(val);
                DirtyFlags::PAINT
            },
            Some(post),
        )
    }

    /// Returns whether reduced-motion mode is enabled on the scheduler.
    pub fn is_reduced_motion(&self) -> bool {
        self.inner.borrow().scheduler.reduced_motion
    }

    /// Enables or disables reduced-motion mode, completing any active animations immediately when enabled.
    pub fn set_reduced_motion(&self, reduced: bool) {
        let mut inner = self.inner.borrow_mut();
        inner.scheduler.reduced_motion = reduced;
        if reduced && !inner.scheduler.animations.is_empty() {
            let active: Vec<_> = inner.scheduler.animations.drain().collect();
            for ((nid, prop), anim) in active {
                if let Some(n) = inner.nodes.get_mut(nid) {
                    apply_anim_value(n, prop, anim.to);
                    inner.mark_dirty(nid, dirty_for_anim_prop(prop));
                }
            }
        }
    }

    /// Advances active node animations to `now` and marks updated nodes dirty.
    pub fn tick_animations(&self, now: Instant) -> bool {
        let updates = self.inner.borrow_mut().scheduler.tick_animations(now);
        if updates.is_empty() {
            return false;
        }
        let mut inner = self.inner.borrow_mut();
        for (nid, prop, val) in updates {
            if let Some(n) = inner.nodes.get_mut(nid) {
                apply_anim_value(n, prop, val);
                inner.mark_dirty(nid, dirty_for_anim_prop(prop));
            }
        }
        true
    }

    /// Starts a property animation on `node` unless reduced motion is enabled (in which case it applies `to` immediately).
    pub fn start_node_animation(
        &self,
        node: NodeId,
        property: AnimProperty,
        from: AnimValue,
        to: AnimValue,
        animation: crate::types::Animation,
    ) {
        let mut inner = self.inner.borrow_mut();
        if inner.scheduler.reduced_motion {
            if let Some(n) = inner.nodes.get_mut(node) {
                apply_anim_value(n, property, to);
                inner.mark_dirty(node, dirty_for_anim_prop(property));
            }
            return;
        }
        if let Some(n) = inner.nodes.get_mut(node) {
            apply_anim_value(n, property, from);
            inner.mark_dirty(node, dirty_for_anim_prop(property));
        }
        inner
            .scheduler
            .start_animation(node, property, from, to, animation, Instant::now());
    }

    fn register_updater_internal<T: 'static>(
        &self,
        node: NodeId,
        compute: impl Fn() -> T + 'static,
        apply: impl Fn(&mut Node, T) -> DirtyFlags + 'static,
        post_inner: Option<ReactiveInnerPostFn>,
    ) -> ReactiveId {
        let compute_erased: Rc<dyn Fn() -> Box<dyn Any>> =
            Rc::new(move || Box::new(compute()) as Box<dyn Any>);
        let apply_erased: ReactiveApplyFn = Rc::new(move |n, boxed| {
            let val = *boxed
                .downcast::<T>()
                .expect("Reactive updater value type mismatch");
            apply(n, val)
        });

        let id = {
            let mut inner = self.inner.borrow_mut();
            let uid = inner.next_updater_uid;
            inner.next_updater_uid = uid.wrapping_add(1);
            let id = inner.reactives.insert(NodeReactiveEntry {
                node,
                compute: compute_erased,
                apply: apply_erased,
                post_inner,
                deps: Vec::new(),
            });
            if let Some(n) = inner.nodes.get_mut(node) {
                n.owned_reactives.push(id);
            }
            let _ = uid;
            id
        };
        self.run_node_updater_inner(id, true);
        id
    }

    fn run_node_updater(&self, reactive_id: ReactiveId) {
        self.run_node_updater_inner(reactive_id, false);
    }

    fn run_node_updater_inner(&self, reactive_id: ReactiveId, is_initial: bool) {
        let (node_id, compute, apply, post_inner) = {
            let mut inner = self.inner.borrow_mut();
            let Some(entry) = inner.reactives.get_mut(reactive_id) else {
                return;
            };
            let node_id = entry.node;
            let compute = entry.compute.clone();
            let apply = entry.apply.clone();
            let post_inner = entry.post_inner.clone();
            let old_deps = std::mem::take(&mut entry.deps);
            if !inner.nodes.contains_key(node_id) {
                return;
            }
            for dep in old_deps {
                inner.remove_subscriber_from_signal(dep, Subscriber::NodeUpdater(reactive_id));
            }
            inner
                .tracking_stack
                .push((TrackingTarget::NodeUpdater(reactive_id), Vec::new()));
            (node_id, compute, apply, post_inner)
        };

        let val = self.enter(|| compute());

        let mut inner = self.inner.borrow_mut();
        let (_, new_deps) = inner
            .tracking_stack
            .pop()
            .expect("tracking_stack imbalanced during node updater");
        let mut owned_subs = Vec::new();
        for &dep in &new_deps {
            owned_subs.push((dep, Subscriber::NodeUpdater(reactive_id)));
        }
        if let Some(entry) = inner.reactives.get_mut(reactive_id) {
            entry.deps = new_deps;
        }
        let reduced_motion = inner.scheduler.reduced_motion;
        let mut dirty = DirtyFlags::NONE;
        let mut anims_to_start: Vec<(AnimProperty, AnimValue, AnimValue, crate::types::Animation)> =
            Vec::new();
        if let Some(node) = inner.nodes.get_mut(node_id) {
            for sub in owned_subs {
                if !node.subscriptions.contains(&sub) {
                    node.subscriptions.push(sub);
                }
            }
            let anim = if is_initial || reduced_motion {
                None
            } else {
                current_animation().or_else(|| node.animation.map(|a| a.animation))
            };
            let prev_opacity = node.visual_props.opacity;
            let prev_bg = node.visual_props.background;
            let prev_offset = node.layout_props.offset;
            let prev_radius = node.visual_props.corner_radii.top_left;
            dirty = apply(node, val);
            if let Some(animation) = anim {
                if (node.visual_props.opacity - prev_opacity).abs() > f32::EPSILON {
                    let target = node.visual_props.opacity;
                    node.visual_props.opacity = prev_opacity;
                    anims_to_start.push((
                        AnimProperty::Opacity,
                        AnimValue::Scalar(prev_opacity),
                        AnimValue::Scalar(target),
                        animation,
                    ));
                }
                if node.visual_props.background != prev_bg
                    && let (Some(from_c), Some(to_c)) = (prev_bg, node.visual_props.background)
                {
                    node.visual_props.background = Some(from_c);
                    anims_to_start.push((
                        AnimProperty::Color,
                        AnimValue::Color(from_c),
                        AnimValue::Color(to_c),
                        animation,
                    ));
                }
                if (node.layout_props.offset.x - prev_offset.x).abs() > f32::EPSILON {
                    let target_x = node.layout_props.offset.x;
                    node.layout_props.offset.x = prev_offset.x;
                    anims_to_start.push((
                        AnimProperty::OffsetX,
                        AnimValue::Scalar(prev_offset.x),
                        AnimValue::Scalar(target_x),
                        animation,
                    ));
                }
                if (node.layout_props.offset.y - prev_offset.y).abs() > f32::EPSILON {
                    let target_y = node.layout_props.offset.y;
                    node.layout_props.offset.y = prev_offset.y;
                    anims_to_start.push((
                        AnimProperty::OffsetY,
                        AnimValue::Scalar(prev_offset.y),
                        AnimValue::Scalar(target_y),
                        animation,
                    ));
                }
                if (node.visual_props.corner_radii.top_left - prev_radius).abs() > f32::EPSILON {
                    let target_r = node.visual_props.corner_radii.top_left;
                    node.visual_props.corner_radii = crate::types::CornerRadii::all(prev_radius);
                    anims_to_start.push((
                        AnimProperty::CornerRadius,
                        AnimValue::Scalar(prev_radius),
                        AnimValue::Scalar(target_r),
                        animation,
                    ));
                }
            }
        }
        if let Some(post) = post_inner {
            post(&mut inner, node_id);
        }
        let now = Instant::now();
        for (prop, from_v, to_v, animation) in anims_to_start {
            inner
                .scheduler
                .start_animation(node_id, prop, from_v, to_v, animation, now);
        }
        if !dirty.is_empty() {
            inner.mark_dirty(node_id, dirty);
        }
    }

    /// Evaluates a condition/selector closure while tracking its dependencies for `node_id` rebuilds.
    pub fn track_node_rebuild<R>(&self, node_id: NodeId, f: impl FnOnce() -> R) -> R {
        {
            let mut inner = self.inner.borrow_mut();
            let old_subs = inner
                .nodes
                .get_mut(node_id)
                .map(|n| std::mem::take(&mut n.subscriptions))
                .unwrap_or_default();
            let mut kept = Vec::new();
            for (sig, sub) in old_subs {
                if sub == Subscriber::NodeRebuild(node_id) {
                    inner.remove_subscriber_from_signal(sig, sub);
                } else {
                    kept.push((sig, sub));
                }
            }
            if let Some(n) = inner.nodes.get_mut(node_id) {
                n.subscriptions = kept;
            }
            inner
                .tracking_stack
                .push((TrackingTarget::NodeRebuild(node_id), Vec::new()));
        }

        let result = self.enter(f);

        let mut inner = self.inner.borrow_mut();
        let (_, new_deps) = inner
            .tracking_stack
            .pop()
            .expect("tracking_stack imbalanced during track_node_rebuild");
        if let Some(node) = inner.nodes.get_mut(node_id) {
            for dep in new_deps {
                let entry = (dep, Subscriber::NodeRebuild(node_id));
                if !node.subscriptions.contains(&entry) {
                    node.subscriptions.push(entry);
                }
            }
        }
        result
    }

    /// Spawns background work off the UI thread and delivers the result back onto the UI thread.
    pub fn spawn<R: Send + 'static>(
        &self,
        work: impl FnOnce() -> R + Send + 'static,
        on_complete: impl FnOnce(R) + 'static,
    ) {
        let spawn_id = {
            let mut inner = self.inner.borrow_mut();
            let id = inner.next_spawn_id;
            inner.next_spawn_id = id.wrapping_add(1);
            inner.spawn_callbacks.insert(
                id,
                Box::new(move |boxed| {
                    let val = *boxed.downcast::<R>().expect("spawn callback type mismatch");
                    on_complete(val);
                }),
            );
            id
        };
        let sender = self.task_sender();
        std::thread::spawn(move || {
            let result = work();
            sender.send(move |rt| {
                let cb = rt.with_inner_mut(|inner| inner.spawn_callbacks.remove(&spawn_id));
                if let Some(cb) = cb {
                    cb(Box::new(result));
                }
            });
        });
    }

    /// Runs `fut` on a caller-supplied [`AsyncExecutor`] and delivers the result to `on_complete` on the UI thread.
    pub fn spawn_future<E, F, R>(&self, executor: &E, fut: F, on_complete: impl FnOnce(R) + 'static)
    where
        E: AsyncExecutor,
        F: std::future::Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        let spawn_id = {
            let mut inner = self.inner.borrow_mut();
            let id = inner.next_spawn_id;
            inner.next_spawn_id = id.wrapping_add(1);
            inner.spawn_callbacks.insert(
                id,
                Box::new(move |boxed| {
                    let val = *boxed
                        .downcast::<R>()
                        .expect("spawn_future callback mismatch");
                    on_complete(val);
                }),
            );
            id
        };
        let sender = self.task_sender();
        executor.spawn(Box::pin(async move {
            let result = fut.await;
            sender.send(move |rt| {
                let cb = rt.with_inner_mut(|inner| inner.spawn_callbacks.remove(&spawn_id));
                if let Some(cb) = cb {
                    cb(Box::new(result));
                }
            });
        }));
    }

    /// Drains background tasks posted via `cx.spawn` or [`TaskSender`].
    pub fn poll_tasks(&self) -> usize {
        let mut tasks = Vec::new();
        {
            let inner = self.inner.borrow();
            while let Ok(task) = inner.task_rx.try_recv() {
                tasks.push(task);
            }
        }
        let count = tasks.len();
        for task in tasks {
            self.enter(|| task(self));
        }
        count
    }

    /// Flushes dirty memos (glitch-free topological order) and dirty node updaters/rebuilds.
    ///
    /// Returns `(reactive_updates, rebuilt_nodes)`.
    pub fn flush_reactive(&self) -> (usize, usize) {
        const MAX_MEMO_DEFERS: usize = 64;
        let start = Instant::now();
        let mut reactive_updates = 0usize;
        let mut rebuilt_nodes = 0usize;
        let mut defer_counts: HashMap<MemoId, usize> = HashMap::new();

        // 1. Evaluate all dirty memos at most once in dependency order.
        loop {
            let next_memo = {
                let mut inner = self.inner.borrow_mut();
                inner.dirty_memos.pop()
            };
            let Some(item) = next_memo else {
                break;
            };
            let should_run = {
                let inner = self.inner.borrow();
                inner.memos.get(item.id).is_some_and(|m| m.dirty)
            };
            if !should_run {
                continue;
            }
            // Check if any upstream memo dependency is still dirty; if so, defer with higher depth.
            let max_dirty_dep_depth = {
                let inner = self.inner.borrow();
                inner.memos.get(item.id).and_then(|m| {
                    m.deps
                        .iter()
                        .filter_map(|dep| match dep {
                            SignalKey::Memo(up_id) => inner
                                .memos
                                .get(*up_id)
                                .filter(|up| up.dirty)
                                .map(|up| up.depth),
                            SignalKey::State(_) => None,
                        })
                        .max()
                })
            };
            if let Some(up_depth) = max_dirty_dep_depth {
                let defers = defer_counts.entry(item.id).or_insert(0);
                *defers += 1;
                if *defers > MAX_MEMO_DEFERS {
                    panic!(
                        "Memo dependency cycle detected during flush involving {:?}",
                        item.id
                    );
                }
                let mut inner = self.inner.borrow_mut();
                if let Some(m) = inner.memos.get_mut(item.id) {
                    m.depth = up_depth + 1;
                }
                inner.dirty_memos.push(DirtyMemoItem {
                    depth: up_depth + 1,
                    id: item.id,
                });
                continue;
            }

            self.recompute_memo(item.id);
        }

        // 2. Run dirty structural rebuilds (`If`, `Switch`, `ForEach`) bounded to catch infinite loops.
        let mut rebuild_passes = 0usize;
        loop {
            let rebuilds = {
                let mut inner = self.inner.borrow_mut();
                inner.pending_rebuild_set.clear();
                std::mem::take(&mut inner.pending_rebuilds)
            };
            if rebuilds.is_empty() {
                break;
            }
            rebuild_passes += 1;
            if rebuild_passes > 64 {
                debug_assert!(
                    false,
                    "RustUI exceeded maximum reactive rebuild depth (64) in a single flush"
                );
                break;
            }
            for node_id in rebuilds {
                if self.rebuild_dynamic_node(node_id) {
                    rebuilt_nodes += 1;
                }
            }
        }

        // 3. Run dirty reactive node updaters (`Text`, property modifiers, etc.).
        let updaters = {
            let mut inner = self.inner.borrow_mut();
            inner.pending_updater_set.clear();
            std::mem::take(&mut inner.pending_updaters)
        };
        for reactive_id in updaters {
            self.run_node_updater(reactive_id);
            reactive_updates += 1;
        }

        let elapsed = start.elapsed();
        let mut inner = self.inner.borrow_mut();
        inner.scheduler.metrics.reactive_updates = reactive_updates;
        inner.scheduler.metrics.rebuilt_nodes = rebuilt_nodes;
        inner.scheduler.metrics.signal_update_time = elapsed;

        (reactive_updates, rebuilt_nodes)
    }

    /// Runs pending [`EffectHandle`] callbacks (invoking any previous cleanup first). Must be called AFTER layout, never during build.
    pub fn flush_effects(&self) -> usize {
        let effects = {
            let mut inner = self.inner.borrow_mut();
            inner.pending_effect_set.clear();
            std::mem::take(&mut inner.pending_effects)
        };
        let mut executed = 0usize;
        for effect_id in effects {
            let (run_opt, prev_cleanup) = {
                let mut inner = self.inner.borrow_mut();
                let Some(entry) = inner.effects.get_mut(effect_id) else {
                    continue;
                };
                let old_deps = std::mem::take(&mut entry.deps);
                let prev_cleanup = entry.cleanup.take();
                let run = entry.run.clone();
                for dep in old_deps {
                    inner.remove_subscriber_from_signal(dep, Subscriber::Effect(effect_id));
                }
                inner
                    .tracking_stack
                    .push((TrackingTarget::Effect(effect_id), Vec::new()));
                (Some(run), prev_cleanup)
            };

            if let Some(cleanup) = prev_cleanup {
                self.enter(cleanup);
            }

            if let Some(run) = run_opt {
                let next_cleanup = self.enter(|| (run.borrow_mut())());
                let mut inner = self.inner.borrow_mut();
                let (_, new_deps) = inner
                    .tracking_stack
                    .pop()
                    .expect("tracking_stack imbalanced during effect run");
                if let Some(entry) = inner.effects.get_mut(effect_id) {
                    entry.deps = new_deps;
                    entry.cleanup = next_cleanup;
                }
                executed += 1;
            }
        }
        executed
    }

    /// Rebuilds a dynamic `If`, `Switch`, or `ForEach` node in-place, preserving or disposing children.
    pub fn rebuild_dynamic_node(&self, node_id: NodeId) -> bool {
        use crate::node::{DynamicNodeData, NodeKind};
        use crate::view::BuildCx;

        enum Action {
            If {
                changed: bool,
                new_branch: bool,
                builder: Option<NodeBuilderFn>,
            },
            Switch {
                changed: bool,
                new_case: u64,
                builder: SwitchBuilderFn,
            },
            ForEach {
                keyed_children: Vec<(u64, NodeId)>,
                rebuilder: ForEachRebuilderFn,
            },
        }

        let action = {
            let inner = self.inner.borrow();
            let Some(node) = inner.nodes.get(node_id) else {
                return false;
            };
            match &node.kind {
                NodeKind::Dynamic(DynamicNodeData::If {
                    last_branch,
                    condition,
                    then_builder,
                    else_builder,
                }) => {
                    let cond = condition.clone();
                    let last = *last_branch;
                    let then_b = then_builder.clone();
                    let else_b = else_builder.clone();
                    drop(inner);
                    let new_branch = self.track_node_rebuild(node_id, || cond());
                    let changed = last != Some(new_branch);
                    let builder = if new_branch { Some(then_b) } else { else_b };
                    Action::If {
                        changed,
                        new_branch,
                        builder,
                    }
                }
                NodeKind::Dynamic(DynamicNodeData::Switch {
                    last_case,
                    selector,
                    builder,
                }) => {
                    let sel = selector.clone();
                    let last = *last_case;
                    let b = builder.clone();
                    drop(inner);
                    let new_case = self.track_node_rebuild(node_id, || sel());
                    let changed = last != Some(new_case);
                    Action::Switch {
                        changed,
                        new_case,
                        builder: b,
                    }
                }
                NodeKind::Dynamic(DynamicNodeData::ForEach {
                    keyed_children,
                    rebuilder,
                }) => Action::ForEach {
                    keyed_children: keyed_children.clone(),
                    rebuilder: rebuilder.clone(),
                },
                _ => return false,
            }
        };

        let mut bcx = BuildCx::new(self.clone(), Some(node_id));
        match action {
            Action::If {
                changed,
                new_branch,
                builder,
            } => {
                if !changed {
                    let mut inner = self.inner.borrow_mut();
                    if let Some(n) = inner.nodes.get_mut(node_id) {
                        n.dirty.remove(DirtyFlags::REBUILD);
                    }
                    return false;
                }
                let old_children = {
                    let mut inner = self.inner.borrow_mut();
                    let Some(n) = inner.nodes.get_mut(node_id) else {
                        return false;
                    };
                    if let NodeKind::Dynamic(DynamicNodeData::If { last_branch, .. }) = &mut n.kind
                    {
                        *last_branch = Some(new_branch);
                    }
                    std::mem::take(&mut n.children)
                };
                for old in old_children {
                    self.remove_node(old);
                }
                if let Some(b) = builder {
                    let child = self.enter(|| b(&mut bcx));
                    self.attach_child(node_id, child);
                }
                self.with_inner_mut(|inner| {
                    if let Some(n) = inner.nodes.get_mut(node_id) {
                        n.dirty.remove(DirtyFlags::REBUILD);
                    }
                    inner.mark_dirty(
                        node_id,
                        DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY,
                    );
                });
                true
            }
            Action::Switch {
                changed,
                new_case,
                builder,
            } => {
                if !changed {
                    let mut inner = self.inner.borrow_mut();
                    if let Some(n) = inner.nodes.get_mut(node_id) {
                        n.dirty.remove(DirtyFlags::REBUILD);
                    }
                    return false;
                }
                let old_children = {
                    let mut inner = self.inner.borrow_mut();
                    let Some(n) = inner.nodes.get_mut(node_id) else {
                        return false;
                    };
                    if let NodeKind::Dynamic(DynamicNodeData::Switch { last_case, .. }) =
                        &mut n.kind
                    {
                        *last_case = Some(new_case);
                    }
                    std::mem::take(&mut n.children)
                };
                for old in old_children {
                    self.remove_node(old);
                }
                let child = self.enter(|| builder(new_case, &mut bcx));
                self.attach_child(node_id, child);
                self.with_inner_mut(|inner| {
                    if let Some(n) = inner.nodes.get_mut(node_id) {
                        n.dirty.remove(DirtyFlags::REBUILD);
                    }
                    inner.mark_dirty(
                        node_id,
                        DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY,
                    );
                });
                true
            }
            Action::ForEach {
                mut keyed_children,
                rebuilder,
            } => {
                let new_keyed =
                    self.track_node_rebuild(node_id, || rebuilder(&mut bcx, &mut keyed_children));
                let kept_ids: HashSet<NodeId> = new_keyed.iter().map(|(_, id)| *id).collect();
                for (_, old_id) in keyed_children {
                    if !kept_ids.contains(&old_id) {
                        self.remove_node(old_id);
                    }
                }
                let new_children: Vec<NodeId> = new_keyed.iter().map(|(_, id)| *id).collect();
                self.with_inner_mut(|inner| {
                    if let Some(n) = inner.nodes.get_mut(node_id) {
                        n.children = new_children.clone();
                        if let NodeKind::Dynamic(DynamicNodeData::ForEach {
                            keyed_children, ..
                        }) = &mut n.kind
                        {
                            *keyed_children = new_keyed;
                        }
                        n.dirty.remove(DirtyFlags::REBUILD);
                    }
                    for &cid in &new_children {
                        if let Some(c) = inner.nodes.get_mut(cid) {
                            c.parent = Some(node_id);
                        }
                    }
                    inner.mark_dirty(
                        node_id,
                        DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY,
                    );
                });
                true
            }
        }
    }

    /// Allocates a node inheriting the parent's [`crate::node::Environment`].
    #[track_caller]
    pub fn alloc_node(&self, kind: crate::node::NodeKind, parent: Option<NodeId>) -> NodeId {
        let loc = std::panic::Location::caller();
        let mut inner = self.inner.borrow_mut();
        let inherited_env = parent
            .and_then(|pid| inner.nodes.get(pid))
            .map(|p| p.env.clone())
            .unwrap_or_default();
        let id = inner.nodes.insert_with_key(|id| {
            let mut n = Node::new(id, kind);
            n.parent = parent;
            n.env = inherited_env;
            n.source_location = Some((loc.file(), loc.line(), loc.column()));
            n
        });
        inner.scheduler.request_redraw();
        id
    }

    /// Attaches `child` to `parent` and ensures inherited environment updates.
    pub fn attach_child(&self, parent: NodeId, child: NodeId) {
        let mut inner = self.inner.borrow_mut();
        let parent_env = inner.nodes.get(parent).map(|p| p.env.clone());
        let already_attached = inner
            .nodes
            .get(child)
            .is_some_and(|c| c.attached_to_parent && c.parent == Some(parent));
        if !already_attached && let Some(p) = inner.nodes.get_mut(parent) {
            p.children.push(child);
        }
        if let Some(c) = inner.nodes.get_mut(child) {
            c.parent = Some(parent);
            c.attached_to_parent = true;
            if let Some(env) = parent_env {
                c.env.inherit_from(&env);
            }
        }
    }

    /// Propagates `parent`'s environment to `child` subtree if overridden on parent.
    pub fn propagate_environment<T: Clone + 'static>(&self, node_id: NodeId, value: T) {
        let mut inner = self.inner.borrow_mut();
        let mut queue = vec![node_id];
        while let Some(id) = queue.pop() {
            if let Some(n) = inner.nodes.get_mut(id) {
                n.env.insert(value.clone());
                queue.extend(n.children.iter().copied());
            }
        }
    }

    /// Fills `value` into descendants of `node_id` that do not already define `T`.
    pub fn inherit_environment<T: Clone + 'static>(&self, node_id: NodeId, value: T) {
        let mut inner = self.inner.borrow_mut();
        let mut queue = inner
            .nodes
            .get(node_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        while let Some(id) = queue.pop() {
            if let Some(n) = inner.nodes.get_mut(id)
                && !n.env.contains::<T>()
            {
                n.env.insert(value.clone());
                queue.extend(n.children.iter().copied());
            }
        }
    }

    /// Disposes `node_id` and its entire subtree, removing all owned states, memos,
    /// effects (running cleanups), timers, reactive updaters, animations, and upstream signal subscriptions.
    pub fn remove_node(&self, node_id: NodeId) {
        let mut cleanups = Vec::new();
        {
            let mut inner = self.inner.borrow_mut();
            Self::remove_node_inner(&mut inner, node_id, &mut cleanups);
        }
        if !cleanups.is_empty() {
            self.enter(|| {
                for cleanup in cleanups {
                    cleanup();
                }
            });
        }
    }

    fn remove_node_inner(
        inner: &mut RuntimeInner,
        node_id: NodeId,
        cleanups: &mut Vec<Box<dyn FnOnce()>>,
    ) {
        let Some(node) = inner.nodes.remove(node_id) else {
            return;
        };

        if let Some(parent_id) = node.parent
            && let Some(parent) = inner.nodes.get_mut(parent_id)
        {
            parent.children.retain(|c| *c != node_id);
        }

        // Remove animations and pending queues (O(1) guard avoids scanning empty/unrelated queues).
        inner.scheduler.remove_node_animations(node_id);
        if inner.pending_rebuild_set.remove(&node_id) {
            inner.pending_rebuilds.retain(|n| *n != node_id);
        }

        // Dispose subscriptions held by this node.
        for (signal, sub) in node.subscriptions {
            inner.remove_subscriber_from_signal(signal, sub);
            if let Subscriber::NodeUpdater(rid) = sub
                && let Some(entry) = inner.reactives.remove(rid)
            {
                for dep in entry.deps {
                    inner.remove_subscriber_from_signal(dep, Subscriber::NodeUpdater(rid));
                }
                if inner.pending_updater_set.remove(&rid) {
                    inner.pending_updaters.retain(|r| *r != rid);
                }
            }
        }

        // Remove all reactive entries owned by this node without scanning the global table.
        for rid in node.owned_reactives {
            if let Some(entry) = inner.reactives.remove(rid) {
                for dep in entry.deps {
                    inner.remove_subscriber_from_signal(dep, Subscriber::NodeUpdater(rid));
                }
                if inner.pending_updater_set.remove(&rid) {
                    inner.pending_updaters.retain(|r| *r != rid);
                }
            }
        }

        // Dispose owned states, memos, effects, and timers.
        for sid in node.owned_states {
            inner.states.remove(sid);
        }
        for mid in node.owned_memos {
            if let Some(m) = inner.memos.remove(mid) {
                for dep in m.deps {
                    inner.remove_subscriber_from_signal(dep, Subscriber::Memo(mid));
                }
            }
            inner.dirty_memo_set.remove(&mid);
        }
        for eid in node.owned_effects {
            if let Some(mut e) = inner.effects.remove(eid) {
                for dep in e.deps {
                    inner.remove_subscriber_from_signal(dep, Subscriber::Effect(eid));
                }
                if let Some(cleanup) = e.cleanup.take() {
                    cleanups.push(cleanup);
                }
            }
            if inner.pending_effect_set.remove(&eid) {
                inner.pending_effects.retain(|e| *e != eid);
            }
        }
        for tid in node.owned_timers {
            inner.timers.remove(tid);
        }

        for child in node.children {
            Self::remove_node_inner(inner, child, cleanups);
        }
    }

    /// Replaces `container_id`'s children with `build_new` while preserving hook `State` values
    /// whenever the root node's `HookSlot` types match identically in order.
    pub fn reload_subtree(
        &self,
        container_id: NodeId,
        build_new: impl FnOnce(&mut crate::view::BuildCx) -> NodeId,
    ) -> Vec<String> {
        let mut diagnostics = Vec::new();
        let saved_slots: Vec<(std::any::TypeId, Box<dyn Any>)> = {
            let mut inner = self.inner.borrow_mut();
            let first_child = inner
                .nodes
                .get(container_id)
                .and_then(|n| n.children.first().copied());
            if let Some(cid) = first_child
                && let Some(cnode) = inner.nodes.get(cid)
            {
                let slots = cnode.hook_slots.clone();
                let mut out = Vec::with_capacity(slots.len());
                for slot in slots {
                    if let Some(st) = inner.states.get_mut(slot.state_id)
                        && let Some(val) = st.value.take()
                    {
                        out.push((slot.type_id, val));
                    }
                }
                out
            } else {
                Vec::new()
            }
        };

        let old_children = self.with_inner_mut(|inner| {
            inner
                .nodes
                .get_mut(container_id)
                .map(|n| std::mem::take(&mut n.children))
                .unwrap_or_default()
        });
        for old in old_children {
            self.remove_node(old);
        }

        let mut bcx = crate::view::BuildCx::new(self.clone(), Some(container_id));
        let new_root = self.enter(|| build_new(&mut bcx));
        self.attach_child(container_id, new_root);

        if !saved_slots.is_empty() {
            let mut inner = self.inner.borrow_mut();
            let new_slots = inner
                .nodes
                .get(new_root)
                .map(|n| n.hook_slots.clone())
                .unwrap_or_default();
            let matches_types = new_slots.len() == saved_slots.len()
                && new_slots
                    .iter()
                    .zip(saved_slots.iter())
                    .all(|(ns, (tid, _))| ns.type_id == *tid);
            if matches_types {
                for (ns, (_, val)) in new_slots.into_iter().zip(saved_slots) {
                    if let Some(st) = inner.states.get_mut(ns.state_id) {
                        st.value = Some(val);
                    }
                    inner.notify_signal(SignalKey::State(ns.state_id));
                }
            } else {
                diagnostics.push(format!(
                    "Hot reload reset subtree state slots (hook layout changed: {} -> {} slots)",
                    saved_slots.len(),
                    new_slots.len()
                ));
            }
            inner.hot_reload_diagnostics = diagnostics.clone();
            inner.mark_dirty(
                container_id,
                DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY,
            );
        } else {
            self.with_inner_mut(|inner| {
                inner.hot_reload_diagnostics.clear();
                inner.mark_dirty(
                    container_id,
                    DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY,
                );
            });
        }
        diagnostics
    }

    /// Returns total live nodes in the arena.
    pub fn node_count(&self) -> usize {
        self.inner.borrow().nodes.len()
    }

    /// Returns total live states in the arena.
    pub fn state_count(&self) -> usize {
        self.inner.borrow().states.len()
    }

    /// Returns total live memos in the arena.
    pub fn memo_count(&self) -> usize {
        self.inner.borrow().memos.len()
    }

    /// Returns total live effects in the arena.
    pub fn effect_count(&self) -> usize {
        self.inner.borrow().effects.len()
    }

    /// Returns total live timers in the arena.
    pub fn timer_count(&self) -> usize {
        self.inner.borrow().timers.len()
    }

    /// Returns true when either nodes are dirty or animations are running.
    pub fn needs_redraw(&self) -> bool {
        let inner = self.inner.borrow();
        inner.scheduler.needs_redraw()
            || !inner.pending_updaters.is_empty()
            || !inner.pending_rebuilds.is_empty()
            || !inner.dirty_memos.is_empty()
    }
}

/// Thread-safe sender used by background tasks (`cx.spawn`) to update UI state on the UI thread.
#[derive(Clone)]
pub struct TaskSender {
    tx: mpsc::Sender<UiTask>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl std::fmt::Debug for TaskSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskSender").finish_non_exhaustive()
    }
}

impl TaskSender {
    /// Enqueues a closure to run on the UI thread inside the runtime scope.
    pub fn send(&self, f: impl FnOnce(&Runtime) + Send + 'static) {
        let _ = self.tx.send(Box::new(f));
        if let Some(wake) = &self.wake {
            wake();
        }
    }
}

/// `Copy` handle into a [`Runtime`]'s state slotmap arena.
pub struct State<T: 'static> {
    pub(crate) id: StateId,
    pub(crate) runtime_id: u64,
    pub(crate) _marker: PhantomData<fn() -> T>,
}

impl<T: 'static> Copy for State<T> {}

impl<T: 'static> Clone for State<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> std::fmt::Debug for State<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("id", &self.id)
            .field("runtime_id", &self.runtime_id)
            .finish()
    }
}

impl<T: 'static> State<T> {
    /// Reads the value, subscribing the current tracking scope (if any).
    pub fn get(self) -> T
    where
        T: Clone,
    {
        Runtime::with_current(self.runtime_id, |rt| rt.get_state(self))
    }

    /// Reads the value without subscribing.
    pub fn get_untracked(self) -> T
    where
        T: Clone,
    {
        Runtime::with_current(self.runtime_id, |rt| rt.get_state_untracked(self))
    }

    /// Sets a new value and notifies subscribers.
    pub fn set(self, value: T) {
        Runtime::with_current(self.runtime_id, |rt| rt.set_state(self, value))
    }

    /// Mutates the value in place and notifies subscribers.
    pub fn update(self, f: impl FnOnce(&mut T)) {
        Runtime::with_current(self.runtime_id, |rt| rt.update_state(self, f))
    }

    /// Returns a two-way [`Binding<T>`] pointing to this state.
    pub fn binding(self) -> Binding<T>
    where
        T: Clone,
    {
        Binding::new(move || self.get(), move |v| self.set(v))
    }

    /// Returns the underlying arena key.
    pub fn id(self) -> StateId {
        self.id
    }
}

/// Two-way read/write projection of reactive state.
pub struct Binding<T: 'static> {
    getter: Rc<dyn Fn() -> T>,
    setter: Rc<dyn Fn(T)>,
}

impl<T: 'static> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            getter: self.getter.clone(),
            setter: self.setter.clone(),
        }
    }
}

impl<T: 'static> std::fmt::Debug for Binding<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Binding").finish_non_exhaustive()
    }
}

impl<T: Clone + 'static> Binding<T> {
    /// Creates a binding from explicit getter and setter closures.
    pub fn new(get: impl Fn() -> T + 'static, set: impl Fn(T) + 'static) -> Self {
        Self {
            getter: Rc::new(get),
            setter: Rc::new(set),
        }
    }

    /// Creates a read-only constant binding (writes are ignored).
    pub fn constant(value: T) -> Self {
        Self::new(move || value.clone(), |_| {})
    }

    /// Reads the bound value (subscribing if inside a reactive scope).
    pub fn get(&self) -> T {
        (self.getter)()
    }

    /// Writes a new value through the binding.
    pub fn set(&self, value: T) {
        (self.setter)(value);
    }

    /// Mutates the bound value using a closure.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let mut val = self.get();
        f(&mut val);
        self.set(val);
    }

    /// Derives a child [`Binding<U>`] via lens read/write projection functions.
    pub fn map<U: Clone + 'static>(
        self,
        read: impl Fn(T) -> U + 'static,
        write: impl Fn(&mut T, U) + 'static,
    ) -> Binding<U> {
        let get_parent = self.getter.clone();
        let set_parent = self.setter.clone();
        let get_parent_for_set = self.getter;
        Binding {
            getter: Rc::new(move || read((get_parent)())),
            setter: Rc::new(move |new_u| {
                let mut parent_val = (get_parent_for_set)();
                write(&mut parent_val, new_u);
                (set_parent)(parent_val);
            }),
        }
    }
}

/// Cached derived computation that re-evaluates at most once per flush in dependency order.
pub struct Memo<T: 'static> {
    id: MemoId,
    runtime_id: u64,
    _marker: PhantomData<fn() -> T>,
}

impl<T: 'static> Copy for Memo<T> {}

impl<T: 'static> Clone for Memo<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> std::fmt::Debug for Memo<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Memo")
            .field("id", &self.id)
            .field("runtime_id", &self.runtime_id)
            .finish()
    }
}

impl<T: Clone + 'static> Memo<T> {
    /// Reads the memoized value and subscribes to future updates.
    pub fn get(self) -> T {
        Runtime::with_current(self.runtime_id, |rt| rt.get_memo(self))
    }

    /// Reads the memoized value without subscribing.
    pub fn get_untracked(self) -> T {
        Runtime::with_current(self.runtime_id, |rt| rt.get_memo_untracked(self))
    }
}

/// Handle to a registered post-layout side effect.
#[derive(Debug, Clone, Copy)]
pub struct EffectHandle {
    pub id: EffectId,
    pub runtime_id: u64,
}

/// Trait for model structs composed of reactive per-field signals (`#[derive(Observable)]` / `cx.model(...)`).
pub trait ObservableModel {
    type Handle: Copy + Clone + 'static;
    fn into_model(self, rt: &Runtime, owner: Option<NodeId>) -> Self::Handle;
}

/// Pluggable async future executor trait used by `cx.spawn_future(...)`.
pub trait AsyncExecutor {
    fn spawn(&self, fut: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>);
}

/// Lifecycle state of an asynchronous [`Resource<T>`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceState<T> {
    Loading,
    Ready(T),
    Error(String),
}

/// Copyable handle to a background-fetched reactive value with loading, ready, and error states.
pub struct Resource<T: 'static> {
    state: State<ResourceState<T>>,
    reload_fn: State<Rc<dyn Fn()>>,
}

impl<T: 'static> Copy for Resource<T> {}

impl<T: 'static> Clone for Resource<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> std::fmt::Debug for Resource<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resource")
            .field("state", &self.state)
            .finish()
    }
}

impl<T: Clone + Send + 'static> Resource<T> {
    /// Reads the current [`ResourceState<T>`] and subscribes the caller.
    pub fn get(self) -> ResourceState<T> {
        self.state.get()
    }

    /// Reads without subscribing.
    pub fn get_untracked(self) -> ResourceState<T> {
        self.state.get_untracked()
    }

    /// Returns true while loading.
    pub fn is_loading(self) -> bool {
        matches!(self.state.get(), ResourceState::Loading)
    }

    /// Returns the resolved `T` value if ready.
    pub fn ready(self) -> Option<T> {
        match self.state.get() {
            ResourceState::Ready(v) => Some(v),
            _ => None,
        }
    }

    /// Returns the error message if the last fetch failed.
    pub fn error(self) -> Option<String> {
        match self.state.get() {
            ResourceState::Error(e) => Some(e),
            _ => None,
        }
    }

    /// Exposes the backing reactive `State<ResourceState<T>>`.
    pub fn state_handle(self) -> State<ResourceState<T>> {
        self.state
    }

    /// Re-triggers the background fetch.
    pub fn reload(self) {
        let f = self.reload_fn.get_untracked();
        f();
    }
}

impl Runtime {
    /// Fetches an asynchronous [`Resource<T>`] managed on a background thread and delivered to the UI thread.
    pub fn create_resource<T: Clone + Send + 'static>(
        &self,
        fetcher: impl Fn() -> Result<T, String> + Send + Sync + 'static,
        owner: Option<NodeId>,
    ) -> Resource<T> {
        let state = self.create_state(ResourceState::<T>::Loading, owner);
        let fetcher = Arc::new(fetcher);
        let rt_clone = self.clone();
        let trigger: Rc<dyn Fn()> = Rc::new(move || {
            rt_clone.set_state(state, ResourceState::Loading);
            let f = fetcher.clone();
            rt_clone.spawn(
                move || f(),
                move |res| match res {
                    Ok(v) => state.set(ResourceState::Ready(v)),
                    Err(e) => state.set(ResourceState::Error(e)),
                },
            );
        });
        trigger();
        let reload_fn = self.create_state(trigger, owner);
        Resource { state, reload_fn }
    }
}

/// Trait for legacy manual observable models.
pub trait Observable {
    fn create(cx: &mut crate::view::Cx) -> Self;
}

/// Generates a model struct whose fields are `State<Field>` handles initialized via `cx.state(...)`.
#[macro_export]
macro_rules! impl_observable {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $($fvis:vis $field:ident : $ty:ty = $init:expr),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy)]
        $vis struct $name {
            $($fvis $field: $crate::signal::State<$ty>,)*
        }

        impl $crate::signal::Observable for $name {
            fn create(cx: &mut $crate::view::Cx) -> Self {
                Self {
                    $($field: cx.state($init),)*
                }
            }
        }
    };
}

fn dirty_for_anim_prop(prop: AnimProperty) -> DirtyFlags {
    match prop {
        AnimProperty::OffsetX | AnimProperty::OffsetY | AnimProperty::Bounds => {
            DirtyFlags::LAYOUT | DirtyFlags::PAINT
        }
        AnimProperty::Opacity
        | AnimProperty::Color
        | AnimProperty::Scale
        | AnimProperty::CornerRadius => DirtyFlags::PAINT,
    }
}

fn apply_anim_value(node: &mut Node, prop: AnimProperty, val: AnimValue) {
    match (prop, val) {
        (AnimProperty::Opacity, AnimValue::Scalar(op)) => {
            node.visual_props.opacity = op.clamp(0.0, 1.0);
        }
        (AnimProperty::Color, AnimValue::Color(col)) => {
            node.visual_props.background = Some(col);
        }
        (AnimProperty::OffsetX, AnimValue::Scalar(x)) => {
            node.layout_props.offset.x = x;
        }
        (AnimProperty::OffsetY, AnimValue::Scalar(y)) => {
            node.layout_props.offset.y = y;
        }
        (AnimProperty::CornerRadius, AnimValue::Scalar(r)) => {
            node.visual_props.corner_radii = crate::types::CornerRadii::all(r.max(0.0));
        }
        (AnimProperty::Bounds, AnimValue::Rect(r)) => {
            node.layout_cache.bounds = r;
        }
        _ => {}
    }
}
