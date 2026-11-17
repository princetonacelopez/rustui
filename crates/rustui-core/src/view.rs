//! Declarative `View`, `Primitive`, `ViewExt`, `Cx`, dynamic control flow (`If`, `Switch`, `ForEach`),
//! and hook-order diagnostics.

use std::any::TypeId;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::node::{
    DirtyFlags, DynamicNodeData, ForEachRebuilderFn, HookSlot, Node, NodeAnimation, NodeBuilderFn,
    NodeId, NodeKind, SwitchBuilderFn,
};
use crate::signal::{Binding, EffectHandle, Memo, Runtime, State};
use crate::types::{
    AccessibilityRole, Animation, Color, ContentMode, CornerRadii, DragEvent, Edge, Frame,
    IntoEdges, IntoPx, KeyEvent, PinchEvent, Point, ScrollEvent, Transition,
};

/// Context passed to composite [`View::body`] implementations.
pub struct Cx {
    runtime: Runtime,
    node_id: NodeId,
    hook_cursor: usize,
}

impl std::fmt::Debug for Cx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cx")
            .field("node_id", &self.node_id)
            .field("hook_cursor", &self.hook_cursor)
            .finish()
    }
}

impl Cx {
    pub fn new(runtime: Runtime, node_id: NodeId) -> Self {
        Self {
            runtime,
            node_id,
            hook_cursor: 0,
        }
    }

    /// Allocates or retrieves a hook-slotted local [`State<T>`] owned by this view node.
    ///
    /// Hook calls must occur unconditionally in the same order on every rebuild of the same node.
    #[track_caller]
    pub fn state<T: 'static>(&mut self, init: T) -> State<T> {
        let caller = std::panic::Location::caller();
        let slot_idx = self.hook_cursor;
        self.hook_cursor += 1;
        let expected_type = TypeId::of::<T>();

        let existing = self.runtime.with_inner(|inner| {
            let node = inner.nodes.get(self.node_id).expect("Cx node disappeared");
            node.hook_slots.get(slot_idx).cloned()
        });

        if let Some(slot) = existing {
            if slot.type_id != expected_type {
                panic!(
                    "RustUI hook order error at {}:{}:{}: hook #{} expected TypeId {:?} (originally declared at {}:{}:{}), but `cx.state(...)` was called with a different type. Ensure `cx.state(...)` is never called inside conditionals or loops.",
                    caller.file(),
                    caller.line(),
                    caller.column(),
                    slot_idx,
                    slot.type_id,
                    slot.location.file(),
                    slot.location.line(),
                    slot.location.column(),
                );
            }
            let runtime_id = self.runtime.id();
            return State {
                id: slot.state_id,
                runtime_id,
                _marker: std::marker::PhantomData,
            };
        }

        let state = self.runtime.create_state(init, Some(self.node_id));
        self.runtime.with_inner_mut(|inner| {
            if let Some(node) = inner.nodes.get_mut(self.node_id) {
                node.hook_slots.push(HookSlot {
                    state_id: state.id,
                    type_id: expected_type,
                    location: caller,
                });
            }
        });
        state
    }

    /// Creates a cached derived [`Memo<T>`] scoped to this node's lifetime.
    pub fn memo<T: Clone + 'static>(&mut self, f: impl Fn() -> T + 'static) -> Memo<T> {
        self.runtime.create_memo(f, Some(self.node_id))
    }

    /// Registers a post-layout side effect scoped to this node's lifetime.
    pub fn effect(&mut self, f: impl FnMut() + 'static) -> EffectHandle {
        self.runtime.create_effect(f, Some(self.node_id))
    }

    /// Reads an inherited environment value set by an ancestor `.environment(...)` modifier.
    pub fn env<T: Clone + 'static>(&self) -> Option<T> {
        self.runtime.with_inner(|inner| {
            let mut cursor = Some(self.node_id);
            while let Some(id) = cursor {
                let node = inner.nodes.get(id)?;
                if let Some(val) = node.env.get::<T>() {
                    return Some(val);
                }
                cursor = node.parent;
            }
            None
        })
    }

    /// Reads an inherited environment value or falls back to `T::default()`.
    pub fn env_or_default<T: Clone + Default + 'static>(&self) -> T {
        self.env::<T>().unwrap_or_default()
    }

    /// Spawns background work off the UI thread and delivers the result back to `on_complete`
    /// on the UI thread.
    pub fn spawn<R: Send + 'static>(
        &self,
        work: impl FnOnce() -> R + Send + 'static,
        on_complete: impl FnOnce(R) + 'static,
    ) {
        self.runtime.spawn(work, on_complete);
    }

    /// Returns a reference to the underlying [`Runtime`].
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Returns the retained [`NodeId`] for this composite view.
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }
}

/// Compile-time lint macro rejecting conditional `cx.state(...)` calls in a view body block.
#[macro_export]
macro_rules! rustui_check_hooks {
    (@scan_branch { $($body:tt)* }) => {
        $crate::rustui_check_hooks!(@scan_branch $($body)*);
    };
    (@scan_branch cx . state ( $($args:tt)* ) $($rest:tt)*) => {
        compile_error!("RustUI hook lint: `cx.state(...)` must not be called conditionally inside `if` branches.");
    };
    (@scan_branch $first:tt $($rest:tt)*) => {
        $crate::rustui_check_hooks!(@scan_branch $($rest)*);
    };
    (@scan_branch) => {};
    (@scan if $($rest:tt)*) => {
        $crate::rustui_check_hooks!(@seek_brace $($rest)*);
    };
    (@scan $first:tt $($rest:tt)*) => {
        $crate::rustui_check_hooks!(@scan $($rest)*);
    };
    (@scan) => {};
    (@seek_brace { $($then:tt)* } $($rest:tt)*) => {
        $crate::rustui_check_hooks!(@scan_branch $($then)*);
        $crate::rustui_check_hooks!(@scan $($rest)*);
    };
    (@seek_brace $tok:tt $($rest:tt)*) => {
        $crate::rustui_check_hooks!(@seek_brace $($rest)*);
    };
    (@seek_brace) => {};
    ($($tt:tt)*) => {{
        $crate::rustui_check_hooks!(@scan $($tt)*);
        $($tt)*
    }};
}

/// Context passed to [`Primitive::build`] when creating retained tree nodes.
pub struct BuildCx {
    pub runtime: Runtime,
    pub parent: Option<NodeId>,
}

impl std::fmt::Debug for BuildCx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildCx")
            .field("parent", &self.parent)
            .finish()
    }
}

impl BuildCx {
    pub fn new(runtime: Runtime, parent: Option<NodeId>) -> Self {
        Self { runtime, parent }
    }

    pub fn alloc_node(&self, kind: NodeKind) -> NodeId {
        self.runtime.alloc_node(kind, self.parent)
    }

    pub fn alloc_composite_node(&self) -> NodeId {
        self.runtime.alloc_node(NodeKind::Composite, self.parent)
    }

    pub fn build_child<V: View>(&self, parent: NodeId, view: V) -> NodeId {
        let mut child_cx = BuildCx::new(self.runtime.clone(), Some(parent));
        let child_id = self.runtime.enter(|| view.build_node(&mut child_cx));
        self.runtime.attach_child(parent, child_id);
        child_id
    }

    pub fn env<T: Clone + 'static>(&self) -> Option<T> {
        let mut cursor = self.parent;
        self.runtime.with_inner(|inner| {
            while let Some(id) = cursor {
                let node = inner.nodes.get(id)?;
                if let Some(val) = node.env.get::<T>() {
                    return Some(val);
                }
                cursor = node.parent;
            }
            None
        })
    }
}

/// Core declarative view trait.
///
/// Composite views implement [`View::body`], consuming `self` as specified in Section 4.
pub trait View: Sized + 'static {
    fn body(self, cx: &mut Cx) -> impl View;

    #[doc(hidden)]
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        let node = cx.alloc_composite_node();
        let mut view_cx = Cx::new(cx.runtime.clone(), node);
        let built = cx.runtime.enter(|| self.body(&mut view_cx));
        let mut child_bcx = BuildCx::new(cx.runtime.clone(), Some(node));
        let child_id = built.build_node(&mut child_bcx);
        cx.runtime.attach_child(node, child_id);
        node
    }
}

/// Low-level primitive view trait that constructs a retained [`NodeId`] directly.
pub trait Primitive: Sized + 'static {
    fn build(self, cx: &mut BuildCx) -> NodeId;
}

/// An empty, zero-size view.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyView;

impl Primitive for EmptyView {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        cx.alloc_node(NodeKind::Container(crate::node::ContainerKind::Fragment))
    }
}

impl View for EmptyView {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Type-erased view box used at container boundaries to reduce generic monomorphization.
pub struct AnyView {
    builder: Box<dyn FnOnce(&mut BuildCx) -> NodeId>,
}

impl std::fmt::Debug for AnyView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyView").finish_non_exhaustive()
    }
}

impl AnyView {
    pub fn new<V: View>(view: V) -> Self {
        Self {
            builder: Box::new(move |cx| view.build_node(cx)),
        }
    }
}

impl Primitive for AnyView {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        (self.builder)(cx)
    }
}

impl View for AnyView {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Trait implemented by child collections accepted by stack/grid containers (`View`, tuples up to 12, `Vec<V>`).
pub trait ViewSequence: 'static {
    fn build_all(self, cx: &mut BuildCx, parent: NodeId);
}

impl<V: View> ViewSequence for V {
    fn build_all(self, cx: &mut BuildCx, parent: NodeId) {
        cx.build_child(parent, self);
    }
}

impl ViewSequence for () {
    fn build_all(self, _cx: &mut BuildCx, _parent: NodeId) {}
}

impl<V: View> ViewSequence for Option<V> {
    fn build_all(self, cx: &mut BuildCx, parent: NodeId) {
        if let Some(v) = self {
            cx.build_child(parent, v);
        }
    }
}

impl<V: View> ViewSequence for Vec<V> {
    fn build_all(self, cx: &mut BuildCx, parent: NodeId) {
        for v in self {
            cx.build_child(parent, v);
        }
    }
}

impl<V: View, const N: usize> ViewSequence for [V; N] {
    fn build_all(self, cx: &mut BuildCx, parent: NodeId) {
        for v in self {
            cx.build_child(parent, v);
        }
    }
}

macro_rules! impl_view_tuple {
    ($($name:ident : $idx:tt),+) => {
        impl<$($name: View),+> ViewSequence for ($($name,)+) {
            fn build_all(self, cx: &mut BuildCx, parent: NodeId) {
                $(cx.build_child(parent, self.$idx);)+
            }
        }
    };
}

impl_view_tuple!(V0: 0);
impl_view_tuple!(V0: 0, V1: 1);
impl_view_tuple!(V0: 0, V1: 1, V2: 2);
impl_view_tuple!(V0: 0, V1: 1, V2: 2, V3: 3);
impl_view_tuple!(V0: 0, V1: 1, V2: 2, V3: 3, V4: 4);
impl_view_tuple!(V0: 0, V1: 1, V2: 2, V3: 3, V4: 4, V5: 5);
impl_view_tuple!(V0: 0, V1: 1, V2: 2, V3: 3, V4: 4, V5: 5, V6: 6);
impl_view_tuple!(V0: 0, V1: 1, V2: 2, V3: 3, V4: 4, V5: 5, V6: 6, V7: 7);
impl_view_tuple!(V0: 0, V1: 1, V2: 2, V3: 3, V4: 4, V5: 5, V6: 6, V7: 7, V8: 8);
impl_view_tuple!(V0: 0, V1: 1, V2: 2, V3: 3, V4: 4, V5: 5, V6: 6, V7: 7, V8: 8, V9: 9);
impl_view_tuple!(V0: 0, V1: 1, V2: 2, V3: 3, V4: 4, V5: 5, V6: 6, V7: 7, V8: 8, V9: 9, V10: 10);
impl_view_tuple!(V0: 0, V1: 1, V2: 2, V3: 3, V4: 4, V5: 5, V6: 6, V7: 7, V8: 8, V9: 9, V10: 10, V11: 11);

/// Source for text widgets and labels supporting both static strings and reactive closures.
pub enum TextSource {
    Static(String),
    Reactive(Rc<dyn Fn() -> String>),
}

impl std::fmt::Debug for TextSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Static(s) => f.debug_tuple("Static").field(s).finish(),
            Self::Reactive(_) => f.debug_tuple("Reactive").finish_non_exhaustive(),
        }
    }
}

/// Ergonomic conversion into [`TextSource`] for `&'static str`, `String`, and `Fn() -> String`.
pub trait IntoTextSource {
    fn into_text_source(self) -> TextSource;
}

impl IntoTextSource for &'static str {
    fn into_text_source(self) -> TextSource {
        TextSource::Static(self.to_owned())
    }
}

impl IntoTextSource for String {
    fn into_text_source(self) -> TextSource {
        TextSource::Static(self)
    }
}

impl<F: Fn() -> String + 'static> IntoTextSource for F {
    fn into_text_source(self) -> TextSource {
        TextSource::Reactive(Rc::new(self))
    }
}

impl IntoTextSource for TextSource {
    fn into_text_source(self) -> TextSource {
        self
    }
}

/// Reactive conditional view primitive (`If`).
pub struct If {
    condition: Rc<dyn Fn() -> bool>,
    then_builder: NodeBuilderFn,
    else_builder: Option<NodeBuilderFn>,
}

impl std::fmt::Debug for If {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("If").finish_non_exhaustive()
    }
}

impl If {
    pub fn new<V: View>(
        condition: impl Fn() -> bool + 'static,
        then_view: impl Fn() -> V + 'static,
    ) -> Self {
        Self {
            condition: Rc::new(condition),
            then_builder: Rc::new(move |cx| then_view().build_node(cx)),
            else_builder: None,
        }
    }

    pub fn else_view<V: View>(mut self, else_view: impl Fn() -> V + 'static) -> Self {
        self.else_builder = Some(Rc::new(move |cx| else_view().build_node(cx)));
        self
    }
}

impl Primitive for If {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let node = cx.alloc_node(NodeKind::Dynamic(DynamicNodeData::If {
            last_branch: None,
            condition: self.condition,
            then_builder: self.then_builder,
            else_builder: self.else_builder,
        }));
        cx.runtime.rebuild_dynamic_node(node);
        node
    }
}

impl View for If {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Reactive multi-branch switch primitive (`Switch`).
pub struct Switch {
    selector: Rc<dyn Fn() -> u64>,
    cases: HashMap<u64, NodeBuilderFn>,
    default: Option<NodeBuilderFn>,
}

impl std::fmt::Debug for Switch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Switch")
            .field("case_count", &self.cases.len())
            .finish()
    }
}

fn hash_key<K: Hash>(key: &K) -> u64 {
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    h.finish()
}

impl Switch {
    pub fn new<K: Hash + 'static>(selector: impl Fn() -> K + 'static) -> Self {
        Self {
            selector: Rc::new(move || hash_key(&selector())),
            cases: HashMap::new(),
            default: None,
        }
    }

    pub fn case<K: Hash, V: View>(mut self, key: K, builder: impl Fn() -> V + 'static) -> Self {
        self.cases
            .insert(hash_key(&key), Rc::new(move |cx| builder().build_node(cx)));
        self
    }

    pub fn default<V: View>(mut self, builder: impl Fn() -> V + 'static) -> Self {
        self.default = Some(Rc::new(move |cx| builder().build_node(cx)));
        self
    }
}

impl Primitive for Switch {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let cases = self.cases;
        let default = self.default;
        let builder: SwitchBuilderFn = Rc::new(move |case_key, bcx| {
            if let Some(b) = cases.get(&case_key) {
                b(bcx)
            } else if let Some(d) = &default {
                d(bcx)
            } else {
                EmptyView.build_node(bcx)
            }
        });
        let node = cx.alloc_node(NodeKind::Dynamic(DynamicNodeData::Switch {
            last_case: None,
            selector: self.selector,
            builder,
        }));
        cx.runtime.rebuild_dynamic_node(node);
        node
    }
}

impl View for Switch {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Keyed dynamic collection view primitive (`ForEach`) that reuses and moves nodes on reorder.
pub struct ForEach {
    rebuilder: ForEachRebuilderFn,
}

impl std::fmt::Debug for ForEach {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForEach").finish_non_exhaustive()
    }
}

impl ForEach {
    pub fn new<Item, K, V>(
        items: impl Fn() -> Vec<Item> + 'static,
        id: impl Fn(&Item) -> K + 'static,
        builder: impl Fn(Item) -> V + 'static,
    ) -> Self
    where
        Item: 'static,
        K: Hash + 'static,
        V: View,
    {
        let rebuilder: ForEachRebuilderFn = Rc::new(move |cx, existing| {
            let list = items();
            let mut existing_map: HashMap<u64, NodeId> = existing.drain(..).collect();
            let mut seen = HashSet::with_capacity(list.len());
            let mut next = Vec::with_capacity(list.len());

            for item in list {
                let mut key = hash_key(&id(&item));
                while !seen.insert(key) {
                    key = key.wrapping_add(1);
                }
                let node_id = if let Some(reused_id) = existing_map.remove(&key) {
                    reused_id
                } else {
                    let built = builder(item);
                    let rt = cx.runtime.clone();
                    rt.enter(|| built.build_node(cx))
                };
                next.push((key, node_id));
            }

            // Restore unused old entries back into `existing` so the caller can dispose them.
            for (k, old_id) in existing_map {
                existing.push((k, old_id));
            }
            next
        });

        Self { rebuilder }
    }
}

impl Primitive for ForEach {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let node = cx.alloc_node(NodeKind::Dynamic(DynamicNodeData::ForEach {
            keyed_children: Vec::new(),
            rebuilder: self.rebuilder,
        }));
        cx.runtime.rebuild_dynamic_node(node);
        node
    }
}

impl View for ForEach {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

fn propagate_corner_radii_down(
    inner: &mut crate::signal::RuntimeInner,
    start: NodeId,
    radii: CornerRadii,
) {
    let mut cur = Some(start);
    while let Some(id) = cur {
        let next = if let Some(n) = inner.nodes.get_mut(id) {
            if matches!(n.kind, NodeKind::Modifier) {
                n.visual_props.corner_radii = radii;
                if n.layout_props.padding == crate::types::Edges::ZERO
                    && n.layout_props.frame.is_none()
                {
                    n.children.first().copied()
                } else {
                    None
                }
            } else {
                if matches!(n.kind, NodeKind::Image(_)) {
                    n.visual_props.corner_radii = radii;
                }
                None
            }
        } else {
            None
        };
        cur = next;
    }
}

fn inherit_child_corner_radii(
    inner: &mut crate::signal::RuntimeInner,
    wrapper: NodeId,
    child: NodeId,
) {
    let child_radii = inner.nodes.get(child).and_then(|c| {
        if matches!(c.kind, NodeKind::Modifier)
            && c.layout_props.padding == crate::types::Edges::ZERO
            && c.visual_props.corner_radii != CornerRadii::ZERO
        {
            Some(c.visual_props.corner_radii)
        } else {
            None
        }
    });
    if let Some(radii) = child_radii
        && let Some(w) = inner.nodes.get_mut(wrapper)
        && w.visual_props.corner_radii == CornerRadii::ZERO
    {
        w.visual_props.corner_radii = radii;
    }
}

fn build_modifier_wrapper<V: View>(
    inner: V,
    cx: &mut BuildCx,
    configure: impl FnOnce(&mut Node, &Runtime, NodeId),
) -> NodeId {
    let wrapper = cx.alloc_node(NodeKind::Modifier);
    cx.runtime.with_inner_mut(|rt_inner| {
        if let Some(node) = rt_inner.nodes.get_mut(wrapper) {
            configure(node, &cx.runtime, wrapper);
        }
    });
    let mut child_cx = BuildCx::new(cx.runtime.clone(), Some(wrapper));
    let child_id = inner.build_node(&mut child_cx);
    cx.runtime.attach_child(wrapper, child_id);
    cx.runtime.with_inner_mut(|rt_inner| {
        let wrapper_radii = rt_inner
            .nodes
            .get(wrapper)
            .map(|n| n.visual_props.corner_radii)
            .unwrap_or(CornerRadii::ZERO);
        if wrapper_radii != CornerRadii::ZERO {
            propagate_corner_radii_down(rt_inner, child_id, wrapper_radii);
        } else {
            inherit_child_corner_radii(rt_inner, wrapper, child_id);
        }
    });
    wrapper
}

/// Padding wrapper view.
#[derive(Debug)]
pub struct Padding<V: View> {
    inner: V,
    edges: crate::types::Edges,
}

impl<V: View> Primitive for Padding<V> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let edges = self.edges;
        build_modifier_wrapper(self.inner, cx, move |node, _, _| {
            node.layout_props.padding = edges;
        })
    }
}

impl<V: View> View for Padding<V> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Frame constraint wrapper view.
#[derive(Debug)]
pub struct Framed<V: View> {
    inner: V,
    frame: Frame,
}

impl<V: View> Primitive for Framed<V> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let frame = self.frame;
        build_modifier_wrapper(self.inner, cx, move |node, _, _| {
            node.layout_props.frame = Some(frame);
        })
    }
}

impl<V: View> View for Framed<V> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Background color wrapper view.
#[derive(Debug)]
pub struct Background<V: View> {
    inner: V,
    color: Color,
}

impl<V: View> Primitive for Background<V> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let color = self.color;
        build_modifier_wrapper(self.inner, cx, move |node, _, _| {
            node.visual_props.background = Some(color);
        })
    }
}

impl<V: View> View for Background<V> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Tap gesture wrapper view.
pub struct OnTap<V: View> {
    inner: V,
    handler: Rc<dyn Fn()>,
}

impl<V: View> std::fmt::Debug for OnTap<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnTap").finish_non_exhaustive()
    }
}

impl<V: View> Primitive for OnTap<V> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let handler = self.handler;
        build_modifier_wrapper(self.inner, cx, move |node, _, _| {
            node.events.on_tap = Some(handler);
            node.accessibility.supports_click = true;
        })
    }
}

impl<V: View> View for OnTap<V> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

type NodeConfigureFn = Box<dyn FnOnce(&mut Node, &Runtime, NodeId)>;
type NodePostBuildFn = Box<dyn FnOnce(&Runtime, NodeId)>;

/// Generic modifier wrapper shared by all remaining modifiers to avoid redundant boilerplate.
pub struct Modified<V: View> {
    inner: V,
    configure: NodeConfigureFn,
    post_build: Option<NodePostBuildFn>,
}

impl<V: View> std::fmt::Debug for Modified<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Modified").finish_non_exhaustive()
    }
}

impl<V: View> Modified<V> {
    fn new(inner: V, configure: impl FnOnce(&mut Node, &Runtime, NodeId) + 'static) -> Self {
        Self {
            inner,
            configure: Box::new(configure),
            post_build: None,
        }
    }

    fn with_post_build(
        inner: V,
        configure: impl FnOnce(&mut Node, &Runtime, NodeId) + 'static,
        post_build: impl FnOnce(&Runtime, NodeId) + 'static,
    ) -> Self {
        Self {
            inner,
            configure: Box::new(configure),
            post_build: Some(Box::new(post_build)),
        }
    }
}

impl<V: View> Primitive for Modified<V> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let node = build_modifier_wrapper(self.inner, cx, self.configure);
        if let Some(post) = self.post_build {
            post(&cx.runtime, node);
        }
        node
    }
}

impl<V: View> View for Modified<V> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Extension methods providing SwiftUI-style modifier chains on every [`View`].
pub trait ViewExt: View + Sized {
    fn padding(self, v: impl IntoEdges) -> Padding<Self> {
        Padding {
            inner: self,
            edges: v.into_edges(),
        }
    }

    fn frame(self, f: Frame) -> Framed<Self> {
        Framed {
            inner: self,
            frame: f,
        }
    }

    fn background(self, c: Color) -> Background<Self> {
        Background {
            inner: self,
            color: c,
        }
    }

    fn background_reactive(self, f: impl Fn() -> Color + 'static) -> Modified<Self> {
        Modified::with_post_build(
            self,
            |_, _, _| {},
            move |rt, node| {
                rt.register_node_updater(node, f, |n, color| {
                    n.visual_props.background = Some(color);
                    DirtyFlags::PAINT
                });
            },
        )
    }

    fn corner_radius(self, r: impl IntoPx) -> Modified<Self> {
        let r = r.into_px();
        Modified::new(self, move |n, _, _| {
            n.visual_props.corner_radii = CornerRadii::all(r);
        })
    }

    fn corner_radii(self, radii: CornerRadii) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.visual_props.corner_radii = radii;
        })
    }

    fn border(self, width: impl IntoPx, color: Color) -> Modified<Self> {
        let w = width.into_px();
        Modified::new(self, move |n, _, _| {
            n.visual_props.border = Some((w, color));
        })
    }

    fn opacity(self, opacity: f32) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.visual_props.opacity = opacity.clamp(0.0, 1.0);
        })
    }

    fn opacity_reactive(self, f: impl Fn() -> f32 + 'static) -> Modified<Self> {
        Modified::with_post_build(
            self,
            |_, _, _| {},
            move |rt, node| {
                rt.register_node_updater(node, f, |n, opacity| {
                    n.visual_props.opacity = opacity.clamp(0.0, 1.0);
                    DirtyFlags::PAINT
                });
            },
        )
    }

    fn shadow(self, color: Color, blur_radius: impl IntoPx, offset: Point) -> Modified<Self> {
        let blur_radius = blur_radius.into_px();
        Modified::new(self, move |n, _, _| {
            n.visual_props.shadow = Some(crate::types::BoxShadow {
                color,
                offset,
                blur_radius,
            });
        })
    }

    fn clipped(self) -> Modified<Self> {
        Modified::new(self, |n, _, _| {
            n.visual_props.clip_to_bounds = true;
        })
    }

    fn hidden(self, hidden: bool) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.visual_props.hidden = hidden;
        })
    }

    fn fixed_size(self, horizontal: bool, vertical: bool) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.layout_props.fixed_horizontal = horizontal;
            n.layout_props.fixed_vertical = vertical;
        })
    }

    fn aspect_ratio(self, ratio: f32, mode: ContentMode) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.layout_props.aspect_ratio = Some((ratio, mode));
        })
    }

    fn offset(self, x: impl IntoPx, y: impl IntoPx) -> Modified<Self> {
        let p = Point::new(x.into_px(), y.into_px());
        Modified::new(self, move |n, _, _| {
            n.layout_props.offset = p;
        })
    }

    fn layout_priority(self, priority: f64) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.layout_props.layout_priority = priority;
        })
    }

    fn alignment_guide(self, x: Option<f32>, y: Option<f32>) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.layout_props.custom_alignment_x = x;
            n.layout_props.custom_alignment_y = y;
        })
    }

    fn layout_value<T: Clone + 'static>(self, value: T) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.layout_props.layout_values.insert(value);
        })
    }

    fn text_direction(self, dir: crate::types::TextDirection) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.layout_props.direction_override = Some(dir);
        })
    }

    fn environment<T: Clone + 'static>(self, value: T) -> Modified<Self> {
        let value_post = value.clone();
        Modified::with_post_build(
            self,
            move |n, _, _| {
                n.env.insert(value);
            },
            move |rt, node| {
                rt.inherit_environment(node, value_post);
            },
        )
    }

    fn environment_reactive<T: Clone + 'static>(
        self,
        f: impl Fn() -> T + 'static,
    ) -> Modified<Self> {
        Modified::with_post_build(
            self,
            |_, _, _| {},
            move |rt, node| {
                rt.register_env_updater(node, f);
            },
        )
    }

    fn id<K: Hash>(self, key: K) -> Modified<Self> {
        let h = hash_key(&key);
        Modified::new(self, move |n, _, _| {
            n.explicit_key = Some(h);
        })
    }

    fn on_tap(self, f: impl Fn() + 'static) -> OnTap<Self> {
        OnTap {
            inner: self,
            handler: Rc::new(f),
        }
    }

    fn on_double_tap(self, f: impl Fn() + 'static) -> Modified<Self> {
        let f = Rc::new(f);
        Modified::new(self, move |n, _, _| {
            n.events.on_double_tap = Some(f);
        })
    }

    fn on_long_press(self, f: impl Fn() + 'static) -> Modified<Self> {
        let f = Rc::new(f);
        Modified::new(self, move |n, _, _| {
            n.events.on_long_press = Some(f);
        })
    }

    fn on_hover(self, f: impl Fn(bool) + 'static) -> Modified<Self> {
        let f = Rc::new(f);
        Modified::new(self, move |n, _, _| {
            n.events.on_hover = Some(f);
        })
    }

    fn on_drag(self, f: impl Fn(DragEvent) + 'static) -> Modified<Self> {
        let f = Rc::new(f);
        Modified::new(self, move |n, _, _| {
            n.events.on_drag = Some(f);
        })
    }

    fn on_scroll(self, f: impl Fn(ScrollEvent) -> bool + 'static) -> Modified<Self> {
        let f = Rc::new(f);
        Modified::new(self, move |n, _, _| {
            n.events.on_scroll = Some(f);
        })
    }

    fn on_pinch(self, f: impl Fn(PinchEvent) + 'static) -> Modified<Self> {
        let f = Rc::new(f);
        Modified::new(self, move |n, _, _| {
            n.events.on_pinch = Some(f);
        })
    }

    fn on_key(self, f: impl Fn(KeyEvent) -> bool + 'static) -> Modified<Self> {
        let f = Rc::new(f);
        Modified::new(self, move |n, _, _| {
            n.events.on_key = Some(f);
        })
    }

    fn focusable(self) -> Modified<Self> {
        Modified::new(self, |n, _, _| {
            n.focus.focusable = true;
            n.accessibility.supports_focus = true;
        })
    }

    fn focused(self, binding: Binding<bool>) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.focus.focusable = true;
            n.focus.focused_binding = Some(binding);
            n.accessibility.supports_focus = true;
        })
    }

    fn accessibility_label(self, label: impl Into<String>) -> Modified<Self> {
        let label = label.into();
        Modified::new(self, move |n, _, _| {
            n.accessibility.label = Some(label);
        })
    }

    fn accessibility_role(self, role: AccessibilityRole) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.accessibility.role = role;
        })
    }

    fn accessibility_value(self, value: impl Into<String>) -> Modified<Self> {
        let value = value.into();
        Modified::new(self, move |n, _, _| {
            n.accessibility.value = Some(value);
        })
    }

    fn accessibility_hidden(self, hidden: bool) -> Modified<Self> {
        Modified::new(self, move |n, _, _| {
            n.accessibility.hidden = hidden;
        })
    }

    fn animation<T: Clone + PartialEq + 'static>(
        self,
        animation: Animation,
        state: State<T>,
    ) -> Modified<Self> {
        Modified::with_post_build(
            self,
            move |n, _, _| {
                n.animation = Some(NodeAnimation {
                    animation,
                    transition: None,
                });
            },
            move |rt, node| {
                rt.register_node_updater(
                    node,
                    move || state.get(),
                    move |n, _val| {
                        n.animation = Some(NodeAnimation {
                            animation,
                            transition: n.animation.and_then(|a| a.transition),
                        });
                        DirtyFlags::PAINT
                    },
                );
            },
        )
    }

    fn transition(self, transition: Transition) -> Modified<Self> {
        Modified::with_post_build(
            self,
            move |n, _, _| {
                let anim = n
                    .animation
                    .map(|a| a.animation)
                    .unwrap_or_else(|| Animation::ease_out(0.18));
                n.animation = Some(NodeAnimation {
                    animation: anim,
                    transition: Some(transition),
                });
            },
            move |rt, node| {
                let anim = rt
                    .with_inner(|inner| {
                        inner
                            .nodes
                            .get(node)
                            .and_then(|n| n.animation.map(|a| a.animation))
                    })
                    .unwrap_or_else(|| Animation::ease_out(0.18));
                match transition {
                    Transition::Opacity | Transition::Scale => {
                        rt.start_node_animation(
                            node,
                            crate::scheduler::AnimProperty::Opacity,
                            crate::scheduler::AnimValue::Scalar(0.05),
                            crate::scheduler::AnimValue::Scalar(1.0),
                            anim,
                        );
                    }
                    Transition::Slide => {
                        rt.start_node_animation(
                            node,
                            crate::scheduler::AnimProperty::Opacity,
                            crate::scheduler::AnimValue::Scalar(0.05),
                            crate::scheduler::AnimValue::Scalar(1.0),
                            anim,
                        );
                        rt.start_node_animation(
                            node,
                            crate::scheduler::AnimProperty::OffsetY,
                            crate::scheduler::AnimValue::Scalar(10.0),
                            crate::scheduler::AnimValue::Scalar(0.0),
                            anim,
                        );
                    }
                    Transition::Move(edge) => {
                        rt.start_node_animation(
                            node,
                            crate::scheduler::AnimProperty::Opacity,
                            crate::scheduler::AnimValue::Scalar(0.05),
                            crate::scheduler::AnimValue::Scalar(1.0),
                            anim,
                        );
                        let (prop, start) = match edge {
                            Edge::Top => (crate::scheduler::AnimProperty::OffsetY, -12.0),
                            Edge::Bottom => (crate::scheduler::AnimProperty::OffsetY, 12.0),
                            Edge::Leading => (crate::scheduler::AnimProperty::OffsetX, -12.0),
                            Edge::Trailing => (crate::scheduler::AnimProperty::OffsetX, 12.0),
                        };
                        rt.start_node_animation(
                            node,
                            prop,
                            crate::scheduler::AnimValue::Scalar(start),
                            crate::scheduler::AnimValue::Scalar(0.0),
                            anim,
                        );
                    }
                }
            },
        )
    }

    fn into_any(self) -> AnyView {
        AnyView::new(self)
    }
}

impl<V: View> ViewExt for V {}
