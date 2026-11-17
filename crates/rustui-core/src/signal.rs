//! Fine-grained reactive signals (`State`, `Binding`, `Memo`, `Effect`, `Observable`)
//! and the single-UI-thread `Runtime` arena.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;

use slotmap::{new_key_type, SlotMap};

use crate::node::{DirtyFlags, Node, NodeId};
use crate::scheduler::{current_animation, AnimProperty, AnimValue, Scheduler};

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

struct EffectEntry {
    run: Rc<RefCell<dyn FnMut()>>,
    deps: Vec<SignalKey>,
}

/// Computes a new typed value from reactive signals and applies it to a retained [`Node`].
pub struct NodeReactiveEntry {
    pub node: NodeId,
    pub compute: Rc<dyn Fn() -> Box<dyn Any>>,
    pub apply: Rc<dyn Fn(&mut Node, Box<dyn Any>) -> DirtyFlags>,
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
    reactives: SlotMap<ReactiveId, NodeReactiveEntry>,
    tracking_stack: Vec<(TrackingTarget, Vec<SignalKey>)>,
    dirty_memos: BinaryHeap<DirtyMemoItem>,
    dirty_memo_set: HashSet<MemoId>,
    pending_updaters: Vec<ReactiveId>,
    pending_updater_set: HashSet<ReactiveId>,
    pending_rebuilds: Vec<NodeId>,
    pending_rebuild_set: HashSet<NodeId>,
    pending_effects: Vec<EffectId>,
    pending_effect_set: HashSet<EffectId>,
    pub scheduler: Scheduler,
    task_tx: mpsc::Sender<UiTask>,
    task_rx: mpsc::Receiver<UiTask>,
    wake_callback: Option<Arc<dyn Fn() + Send + Sync>>,
    next_updater_uid: u64,
    spawn_callbacks: HashMap<u64, Box<dyn FnOnce(Box<dyn Any + Send>)>>,
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
            reactives: SlotMap::with_key(),
            tracking_stack: Vec::new(),
            dirty_memos: BinaryHeap::new(),
            dirty_memo_set: HashSet::new(),
            pending_updaters: Vec::new(),
            pending_updater_set: HashSet::new(),
            pending_rebuilds: Vec::new(),
            pending_rebuild_set: HashSet::new(),
            pending_effects: Vec::new(),
            pending_effect_set: HashSet::new(),
            scheduler: Scheduler::new(),
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
                    self.mark_dirty(node_id, DirtyFlags::REBUILD | DirtyFlags::LAYOUT | DirtyFlags::PAINT);
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

        let mut propagate_layout = flags.intersects(DirtyFlags::LAYOUT);
        let propagate_paint = flags.intersects(DirtyFlags::PAINT);
        let propagate_a11y = flags.intersects(DirtyFlags::ACCESSIBILITY);
        let mut cursor = node.parent;
        let mut hit_fixed_boundary = node.layout_props.is_fixed_size_boundary();
        if hit_fixed_boundary {
            propagate_layout = false;
        }

        while let Some(parent_id) = cursor {
            let Some(parent) = self.nodes.get_mut(parent_id) else {
                break;
            };
            if propagate_layout {
                parent.dirty.insert(DirtyFlags::LAYOUT | DirtyFlags::PAINT);
                if parent.layout_props.is_fixed_size_boundary() {
                    propagate_layout = false;
                    hit_fixed_boundary = true;
                    let _ = hit_fixed_boundary;
                }
            } else if propagate_paint {
                parent.dirty.insert(DirtyFlags::PAINT);
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
        assert_eq!(inner.id, state.runtime_id, "State accessed from wrong Runtime");
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
        assert_eq!(inner.id, state.runtime_id, "State accessed from wrong Runtime");
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
        assert_eq!(inner.id, state.runtime_id, "State mutated from wrong Runtime");
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
            assert_eq!(inner.id, state.runtime_id, "State updated from wrong Runtime");
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
            assert_eq!(inner.id, memo.runtime_id, "Memo accessed from wrong Runtime");
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
            assert_eq!(inner.id, memo.runtime_id, "Memo accessed from wrong Runtime");
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
            let Some(entry) = inner.memos.get_mut(memo_id) else {
                return;
            };
            let compute = entry.compute.clone();
            let old_deps = std::mem::take(&mut entry.deps);
            for dep in &old_deps {
                inner.remove_subscriber_from_signal(*dep, Subscriber::Memo(memo_id));
            }
            inner
                .tracking_stack
                .push((TrackingTarget::Memo(memo_id), Vec::new()));
            (compute, old_deps)
        };
        let _ = old_deps;

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
        f: impl FnMut() + 'static,
        owner: Option<NodeId>,
    ) -> EffectHandle {
        let mut inner = self.inner.borrow_mut();
        let run: Rc<RefCell<dyn FnMut()>> = Rc::new(RefCell::new(f));
        let id = inner.effects.insert(EffectEntry {
            run,
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

    /// Registers a reactive node updater closure that runs initially and whenever its dependencies change.
    pub fn register_node_updater<T: 'static>(
        &self,
        node: NodeId,
        compute: impl Fn() -> T + 'static,
        apply: impl Fn(&mut Node, T) -> DirtyFlags + 'static,
    ) -> ReactiveId {
        let compute_erased: Rc<dyn Fn() -> Box<dyn Any>> =
            Rc::new(move || Box::new(compute()) as Box<dyn Any>);
        let apply_erased: Rc<dyn Fn(&mut Node, Box<dyn Any>) -> DirtyFlags> =
            Rc::new(move |n, boxed| {
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
                deps: Vec::new(),
            });
            let _ = uid;
            id
        };
        self.run_node_updater(id);
        id
    }

    fn run_node_updater(&self, reactive_id: ReactiveId) {
        let (node_id, compute, apply) = {
            let mut inner = self.inner.borrow_mut();
            let Some(entry) = inner.reactives.get_mut(reactive_id) else {
                return;
            };
            let node_id = entry.node;
            let compute = entry.compute.clone();
            let apply = entry.apply.clone();
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
            (node_id, compute, apply)
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
        let mut dirty = DirtyFlags::NONE;
        let mut anims_to_start: Vec<(AnimProperty, AnimValue, AnimValue, crate::types::Animation)> =
            Vec::new();
        if let Some(node) = inner.nodes.get_mut(node_id) {
            for sub in owned_subs {
                if !node.subscriptions.contains(&sub) {
                    node.subscriptions.push(sub);
                }
            }
            let anim = current_animation().or_else(|| node.animation.map(|a| a.animation));
            let prev_opacity = node.visual_props.opacity;
            let prev_bg = node.visual_props.background;
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
            }
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
                    let val = *boxed
                        .downcast::<R>()
                        .expect("spawn callback type mismatch");
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
        let start = Instant::now();
        let mut reactive_updates = 0usize;
        let mut rebuilt_nodes = 0usize;

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
                inner
                    .memos
                    .get(item.id)
                    .is_some_and(|m| m.dirty)
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

        // 2. Run dirty structural rebuilds (`If`, `Switch`, `ForEach`).
        let rebuilds = {
            let mut inner = self.inner.borrow_mut();
            inner.pending_rebuild_set.clear();
            std::mem::take(&mut inner.pending_rebuilds)
        };
        for node_id in rebuilds {
            if self.rebuild_dynamic_node(node_id) {
                rebuilt_nodes += 1;
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

    /// Runs pending [`EffectHandle`] callbacks. Must be called AFTER layout, never during build.
    pub fn flush_effects(&self) -> usize {
        let effects = {
            let mut inner = self.inner.borrow_mut();
            inner.pending_effect_set.clear();
            std::mem::take(&mut inner.pending_effects)
        };
        let mut executed = 0usize;
        for effect_id in effects {
            let run_opt = {
                let mut inner = self.inner.borrow_mut();
                let Some(entry) = inner.effects.get_mut(effect_id) else {
                    continue;
                };
                let old_deps = std::mem::take(&mut entry.deps);
                let run = entry.run.clone();
                for dep in old_deps {
                    inner.remove_subscriber_from_signal(dep, Subscriber::Effect(effect_id));
                }
                inner
                    .tracking_stack
                    .push((TrackingTarget::Effect(effect_id), Vec::new()));
                Some(run)
            };

            if let Some(run) = run_opt {
                self.enter(|| {
                    (run.borrow_mut())();
                });
                let mut inner = self.inner.borrow_mut();
                let (_, new_deps) = inner
                    .tracking_stack
                    .pop()
                    .expect("tracking_stack imbalanced during effect run");
                if let Some(entry) = inner.effects.get_mut(effect_id) {
                    entry.deps = new_deps;
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
                builder: Option<Rc<dyn Fn(&mut BuildCx) -> NodeId>>,
            },
            Switch {
                changed: bool,
                new_case: u64,
                builder: Rc<dyn Fn(u64, &mut BuildCx) -> NodeId>,
            },
            ForEach {
                keyed_children: Vec<(u64, NodeId)>,
                rebuilder: Rc<dyn Fn(&mut BuildCx, &mut Vec<(u64, NodeId)>) -> Vec<(u64, NodeId)>>,
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
                    if let NodeKind::Dynamic(DynamicNodeData::If { last_branch, .. }) = &mut n.kind {
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
                    inner.mark_dirty(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY);
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
                    inner.mark_dirty(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY);
                });
                true
            }
            Action::ForEach {
                mut keyed_children,
                rebuilder,
            } => {
                let new_keyed = self.track_node_rebuild(node_id, || rebuilder(&mut bcx, &mut keyed_children));
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
                    inner.mark_dirty(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY);
                });
                true
            }
        }
    }

    /// Allocates a node inheriting the parent's [`Environment`].
    pub fn alloc_node(&self, kind: crate::node::NodeKind, parent: Option<NodeId>) -> NodeId {
        let mut inner = self.inner.borrow_mut();
        let inherited_env = parent
            .and_then(|pid| inner.nodes.get(pid))
            .map(|p| p.env.clone())
            .unwrap_or_default();
        let id = inner.nodes.insert_with_key(|id| {
            let mut n = Node::new(id, kind);
            n.parent = parent;
            n.env = inherited_env;
            n
        });
        inner.scheduler.request_redraw();
        id
    }

    /// Attaches `child` to `parent` and ensures inherited environment updates.
    pub fn attach_child(&self, parent: NodeId, child: NodeId) {
        let mut inner = self.inner.borrow_mut();
        let parent_env = inner.nodes.get(parent).map(|p| p.env.clone());
        if let Some(p) = inner.nodes.get_mut(parent)
            && !p.children.contains(&child)
        {
            p.children.push(child);
        }
        if let Some(c) = inner.nodes.get_mut(child) {
            c.parent = Some(parent);
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

    /// Disposes `node_id` and its entire subtree, removing all owned states, memos,
    /// effects, reactive updaters, animations, and upstream signal subscriptions.
    pub fn remove_node(&self, node_id: NodeId) {
        let mut inner = self.inner.borrow_mut();
        Self::remove_node_inner(&mut inner, node_id);
    }

    fn remove_node_inner(inner: &mut RuntimeInner, node_id: NodeId) {
        let Some(node) = inner.nodes.remove(node_id) else {
            return;
        };

        if let Some(parent_id) = node.parent
            && let Some(parent) = inner.nodes.get_mut(parent_id)
        {
            parent.children.retain(|c| *c != node_id);
        }

        // Remove animations and pending queues.
        inner.scheduler.remove_node_animations(node_id);
        inner.pending_rebuild_set.remove(&node_id);
        inner.pending_rebuilds.retain(|n| *n != node_id);

        // Dispose subscriptions held by this node.
        for (signal, sub) in node.subscriptions {
            inner.remove_subscriber_from_signal(signal, sub);
            if let Subscriber::NodeUpdater(rid) = sub
                && let Some(entry) = inner.reactives.remove(rid)
            {
                for dep in entry.deps {
                    inner.remove_subscriber_from_signal(dep, Subscriber::NodeUpdater(rid));
                }
                inner.pending_updater_set.remove(&rid);
                inner.pending_updaters.retain(|r| *r != rid);
            }
        }

        // Also remove any reactive entries pointing to this node.
        let orphan_reactives: Vec<ReactiveId> = inner
            .reactives
            .iter()
            .filter_map(|(rid, r)| if r.node == node_id { Some(rid) } else { None })
            .collect();
        for rid in orphan_reactives {
            if let Some(entry) = inner.reactives.remove(rid) {
                for dep in entry.deps {
                    inner.remove_subscriber_from_signal(dep, Subscriber::NodeUpdater(rid));
                }
                inner.pending_updater_set.remove(&rid);
                inner.pending_updaters.retain(|r| *r != rid);
            }
        }

        // Dispose owned states, memos, and effects.
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
            if let Some(e) = inner.effects.remove(eid) {
                for dep in e.deps {
                    inner.remove_subscriber_from_signal(dep, Subscriber::Effect(eid));
                }
            }
            inner.pending_effect_set.remove(&eid);
            inner.pending_effects.retain(|e| *e != eid);
        }

        for child in node.children {
            Self::remove_node_inner(inner, child);
        }
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
        Binding::new(
            move || self.get(),
            move |v| self.set(v),
        )
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

/// Trait for model structs composed of reactive per-field signals.
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
