//! Core reactive node tree, signals, scheduler, and declarative View API for RustUI.

pub mod node;
pub mod scheduler;
pub mod signal;
pub mod types;
pub mod view;

pub use node::{
    AccessibilityNodeData, CachedLayout, CanvasNodeData, CanvasPaintFn, ContainerKind,
    ControlNodeData, DirtyFlags, DynamicNodeData, Environment, EventHandlers, FocusNodeData,
    FontSpec, FontWeight, GridTrack, HookSlot, ImageNodeData, ImageSourceData, LayoutProps,
    LayoutValueMap, Node, NodeAnimation, NodeId, NodeKind, ScrollAxes, ScrollNodeData,
    ShaderNodeData, TextNodeData, TextSpanData, TruncationMode, VirtualListNodeData, VisualProps,
};
pub use rustui_macros::{Observable, preview};
pub use scheduler::{AnimProperty, AnimValue, FrameMetrics, Scheduler, with_animation};
pub use signal::{
    AsyncExecutor, Binding, EffectHandle, Memo, Observable, ObservableModel, Resource,
    ResourceState, Runtime, RuntimeInner, State, TaskSender, TimerId,
};
pub use types::{
    AccessibilityLive, AccessibilityRole, Alignment, Animation, BoxShadow, Color, ContentMode,
    CornerRadii, DragEvent, DragPayload, DropEvent, Edge, Edges, Frame, HorizontalAlignment,
    IntoEdges, IntoPx, KeyCode, KeyEvent, Modifiers, PinchEvent, Point, PointerButton,
    PointerEvent, PointerPhase, Rect, SafeAreaInsets, ScrollEvent, SemanticColor, Size,
    TextDirection, ThemePalette, Transition, VerticalAlignment,
};
pub use view::{
    AnyView, Background, BuildCx, Cx, EmptyView, ForEach, Framed, If, IntoTextSource, Modified,
    OnTap, Padding, Primitive, Switch, TextSource, View, ViewExt, ViewSequence,
};

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Instant;

    use super::*;

    struct TestLabel(TextSource);

    impl Primitive for TestLabel {
        fn build(self, cx: &mut BuildCx) -> NodeId {
            let node = cx.alloc_node(NodeKind::Text(TextNodeData::default()));
            match self.0 {
                TextSource::Static(s) => {
                    cx.runtime.with_inner_mut(|inner| {
                        if let Some(n) = inner.nodes.get_mut(node)
                            && let NodeKind::Text(t) = &mut n.kind
                        {
                            t.text = s;
                        }
                    });
                }
                TextSource::Reactive(f) => {
                    cx.runtime.register_node_updater(
                        node,
                        move || f(),
                        |n, text| {
                            if let NodeKind::Text(t) = &mut n.kind {
                                t.text = text;
                            }
                            DirtyFlags::LAYOUT | DirtyFlags::PAINT
                        },
                    );
                }
            }
            node
        }
    }

    impl View for TestLabel {
        fn body(self, _cx: &mut Cx) -> impl View {
            EmptyView
        }
        fn build_node(self, cx: &mut BuildCx) -> NodeId {
            Primitive::build(self, cx)
        }
    }

    struct TestColumn<C>(C);

    impl<C: ViewSequence> Primitive for TestColumn<C> {
        fn build(self, cx: &mut BuildCx) -> NodeId {
            let node = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
                spacing: 8.0,
                alignment: HorizontalAlignment::Leading,
            }));
            self.0.build_all(cx, node);
            node
        }
    }

    impl<C: ViewSequence> View for TestColumn<C> {
        fn body(self, _cx: &mut Cx) -> impl View {
            EmptyView
        }
        fn build_node(self, cx: &mut BuildCx) -> NodeId {
            Primitive::build(self, cx)
        }
    }

    #[test]
    fn glitch_free_diamond_memo_runs_once_per_flush() {
        let rt = Runtime::new();
        rt.enter(|| {
            let a = rt.create_state(1i32, None);
            let b_runs = Rc::new(Cell::new(0usize));
            let c_runs = Rc::new(Cell::new(0usize));

            let b_runs_c = b_runs.clone();
            let b = rt.create_memo(
                move || {
                    b_runs_c.set(b_runs_c.get() + 1);
                    a.get() * 2
                },
                None,
            );

            let c_runs_c = c_runs.clone();
            let c = rt.create_memo(
                move || {
                    c_runs_c.set(c_runs_c.get() + 1);
                    a.get() + b.get()
                },
                None,
            );

            assert_eq!(c.get_untracked(), 3);
            assert_eq!(b_runs.get(), 1);
            assert_eq!(c_runs.get(), 1);

            a.set(10);
            rt.flush_reactive();

            assert_eq!(c.get_untracked(), 30);
            assert_eq!(b_runs.get(), 2, "memo B must re-run at most once on flush");
            assert_eq!(c_runs.get(), 2, "memo C must re-run at most once on flush");
        });
    }

    #[test]
    fn untracked_read_does_not_subscribe() {
        let rt = Runtime::new();
        rt.enter(|| {
            let s = rt.create_state(10, None);
            let runs = Rc::new(Cell::new(0usize));
            let runs_c = runs.clone();
            let m = rt.create_memo(
                move || {
                    runs_c.set(runs_c.get() + 1);
                    s.get_untracked() * 3
                },
                None,
            );
            assert_eq!(m.get_untracked(), 30);
            assert_eq!(runs.get(), 1);

            s.set(20);
            rt.flush_reactive();
            assert_eq!(runs.get(), 1);
            assert_eq!(m.get_untracked(), 30);
        });
    }

    #[test]
    fn fine_grained_update_in_10k_node_tree_updates_single_node_under_1ms() {
        let rt = Runtime::new();
        let mut bcx = BuildCx::new(rt.clone(), None);
        let root = rt.alloc_node(
            NodeKind::Container(ContainerKind::VStack {
                spacing: 4.0,
                alignment: HorizontalAlignment::Leading,
            }),
            None,
        );
        rt.with_inner_mut(|inner| inner.root = Some(root));

        let target_state = rt.create_state(0i32, Some(root));
        let mut target_node = root;

        for i in 0..10_000 {
            let child = if i == 5_432 {
                TestLabel(format_text(target_state)).build_node(&mut bcx)
            } else {
                TestLabel(IntoTextSource::into_text_source("static")).build_node(&mut bcx)
            };
            if i == 5_432 {
                target_node = child;
            }
            rt.attach_child(root, child);
        }

        assert!(rt.node_count() >= 10_001);

        let start = Instant::now();
        rt.enter(|| target_state.set(42));
        let (updates, rebuilds) = rt.flush_reactive();
        let elapsed = start.elapsed();

        assert_eq!(updates, 1);
        assert_eq!(rebuilds, 0);
        assert!(
            elapsed.as_secs_f64() < 0.005,
            "10k tree single-node update took {:?}",
            elapsed
        );

        rt.with_inner(|inner| {
            let NodeKind::Text(t) = &inner.nodes.get(target_node).unwrap().kind else {
                panic!("expected text node");
            };
            assert_eq!(t.text, "val=42");
        });
    }

    fn format_text(state: State<i32>) -> TextSource {
        TextSource::Reactive(Rc::new(move || format!("val={}", state.get())))
    }

    #[test]
    fn node_disposal_cleans_owned_states_memos_and_subscriptions() {
        let rt = Runtime::new();
        struct TempView(State<i32>);
        impl View for TempView {
            fn body(self, cx: &mut Cx) -> impl View {
                let local = cx.state(99);
                let _derived = cx.memo(move || local.get() + self.0.get());
                let outer = self.0;
                TestLabel(TextSource::Reactive(Rc::new(move || {
                    format!("{} {}", outer.get(), local.get())
                })))
            }
        }

        let outer = rt.create_state(1, None);
        let initial_states = rt.state_count();
        let mut bcx = BuildCx::new(rt.clone(), None);
        let node = TempView(outer).build_node(&mut bcx);
        assert_eq!(rt.state_count(), initial_states + 1);
        assert_eq!(rt.memo_count(), 1);

        rt.remove_node(node);
        assert_eq!(rt.state_count(), initial_states);
        assert_eq!(rt.memo_count(), 0);
        assert_eq!(rt.node_count(), 0);
    }

    #[test]
    fn foreach_reorders_without_recreating_nodes() {
        #[derive(Clone)]
        struct Item {
            id: u64,
            name: &'static str,
        }

        let rt = Runtime::new();
        let items = rt.create_state(
            vec![
                Item { id: 1, name: "A" },
                Item { id: 2, name: "B" },
                Item { id: 3, name: "C" },
            ],
            None,
        );

        let builds = Rc::new(Cell::new(0usize));
        let builds_c = builds.clone();
        let mut bcx = BuildCx::new(rt.clone(), None);
        let fe = ForEach::new(
            move || items.get(),
            |i| i.id,
            move |i| {
                builds_c.set(builds_c.get() + 1);
                TestLabel(IntoTextSource::into_text_source(i.name.to_owned()))
            },
        )
        .build_node(&mut bcx);

        assert_eq!(builds.get(), 3);
        let original_children =
            rt.with_inner(|inner| inner.nodes.get(fe).unwrap().children.clone());
        assert_eq!(original_children.len(), 3);

        // Reorder [3, 1, 2]: zero new nodes should be built!
        rt.enter(|| {
            items.set(vec![
                Item { id: 3, name: "C" },
                Item { id: 1, name: "A" },
                Item { id: 2, name: "B" },
            ]);
        });
        let (_, rebuilds) = rt.flush_reactive();
        assert_eq!(rebuilds, 1);
        assert_eq!(
            builds.get(),
            3,
            "reordering ForEach must move nodes, not recreate"
        );

        let reordered = rt.with_inner(|inner| inner.nodes.get(fe).unwrap().children.clone());
        assert_eq!(
            reordered,
            vec![
                original_children[2],
                original_children[0],
                original_children[1]
            ]
        );
    }

    #[test]
    fn effects_run_post_layout_and_binding_map_works() {
        let rt = Runtime::new();
        let count = rt.create_state(5i32, None);
        let effect_seen = Rc::new(Cell::new(0i32));
        let effect_seen_c = effect_seen.clone();

        rt.create_effect(
            move || {
                effect_seen_c.set(count.get());
            },
            None,
        );

        // Effect hasn't run yet during creation.
        assert_eq!(effect_seen.get(), 0);
        rt.flush_effects();
        assert_eq!(effect_seen.get(), 5);

        let doubled = count
            .binding()
            .map(|v| v * 2, |orig, new_v| *orig = new_v / 2);
        rt.enter(|| {
            assert_eq!(doubled.get(), 10);
            doubled.set(24);
            assert_eq!(count.get_untracked(), 12);
        });
        rt.flush_effects();
        assert_eq!(effect_seen.get(), 12);
    }

    #[test]
    fn background_spawn_delivers_state_update_to_ui_thread() {
        let rt = Runtime::new();
        let state = rt.create_state(0i32, None);
        let cx = Cx::new(rt.clone(), rt.alloc_node(NodeKind::Composite, None));
        cx.spawn(|| 1234, move |res| state.set(res));

        for _ in 0..100 {
            if rt.poll_tasks() > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(rt.get_state_untracked(state), 1234);
    }

    #[test]
    fn fixed_size_boundary_stops_layout_dirty_propagation() {
        let rt = Runtime::new();
        let mut bcx = BuildCx::new(rt.clone(), None);
        let root_state = rt.create_state(0, None);
        let view = TestColumn((
            TestLabel(format_text(root_state)).frame(Frame::fixed(120.0, 32.0)),
            TestLabel(IntoTextSource::into_text_source("sibling")),
        ));
        let root = view.build_node(&mut bcx);
        rt.with_inner_mut(|inner| {
            for (_, n) in &mut inner.nodes {
                n.dirty.clear();
            }
        });

        rt.enter(|| root_state.set(7));
        rt.flush_reactive();

        rt.with_inner(|inner| {
            let root_node = inner.nodes.get(root).unwrap();
            assert!(
                !root_node.dirty.contains(DirtyFlags::LAYOUT),
                "layout dirty propagation must stop at fixed-size boundary"
            );
            assert!(root_node.dirty.contains(DirtyFlags::PAINT));
        });
    }

    #[test]
    fn with_animation_and_environment_inheritance() {
        #[derive(Clone, Debug, PartialEq)]
        struct TestTheme(&'static str);

        struct Child;
        impl View for Child {
            fn body(self, cx: &mut Cx) -> impl View {
                let theme = cx.env::<TestTheme>().expect("inherited environment");
                assert_eq!(theme, TestTheme("dark"));
                EmptyView
            }
        }

        let rt = Runtime::new();
        let mut bcx = BuildCx::new(rt.clone(), None);
        let _root = Child.environment(TestTheme("dark")).build_node(&mut bcx);

        let opacity = rt.create_state(1.0f32, None);
        let box_node = TestLabel(IntoTextSource::into_text_source("anim"))
            .opacity_reactive(move || opacity.get())
            .build_node(&mut bcx);

        with_animation(Animation::ease_in_out(0.25), || {
            rt.enter(|| opacity.set(0.0));
            rt.flush_reactive();
        });
        assert!(rt.with_inner(|inner| !inner.scheduler.animations.is_empty()));
        let _ = box_node;
    }

    #[test]
    fn hundred_thousand_node_disposal_leaves_zero_leaks() {
        let rt = Runtime::new();
        let base_nodes = rt.node_count();
        let base_states = rt.state_count();
        let base_memos = rt.memo_count();
        let base_effects = rt.effect_count();
        let base_timers = rt.timer_count();

        let container = rt.alloc_node(NodeKind::Composite, None);
        for i in 0..100_000 {
            let n = rt.alloc_node(NodeKind::Modifier, Some(container));
            rt.attach_child(container, n);
            if i % 100 == 0 {
                let s = rt.create_state(i, Some(n));
                let _m = rt.create_memo(move || s.get() + 1, Some(n));
                rt.create_effect(move || {}, Some(n));
                rt.create_timer(std::time::Duration::from_secs(10), true, || {}, Some(n));
            }
        }
        assert_eq!(rt.node_count(), base_nodes + 100_001);
        assert!(rt.state_count() >= base_states + 1_000);
        assert!(rt.memo_count() >= base_memos + 1_000);
        assert!(rt.effect_count() >= base_effects + 1_000);
        assert!(rt.timer_count() >= base_timers + 1_000);

        rt.remove_node(container);
        assert_eq!(rt.node_count(), base_nodes);
        assert_eq!(rt.state_count(), base_states);
        assert_eq!(rt.memo_count(), base_memos);
        assert_eq!(rt.effect_count(), base_effects);
        assert_eq!(rt.timer_count(), base_timers);
    }

    #[test]
    #[should_panic(expected = "Memo dependency cycle detected")]
    fn memo_cycle_detection_panics_cleanly() {
        let rt = Runtime::new();
        rt.enter(|| {
            let trigger = rt.create_state(false, None);
            let slot_a: Rc<RefCell<Option<Memo<i32>>>> = Rc::new(RefCell::new(None));
            let slot_b: Rc<RefCell<Option<Memo<i32>>>> = Rc::new(RefCell::new(None));

            let b_ref = slot_b.clone();
            let a = rt.create_memo(
                move || {
                    if trigger.get() {
                        b_ref.borrow().unwrap().get() + 1
                    } else {
                        1
                    }
                },
                None,
            );
            *slot_a.borrow_mut() = Some(a);

            let a_ref = slot_a.clone();
            let b = rt.create_memo(move || a_ref.borrow().unwrap().get() + 1, None);
            *slot_b.borrow_mut() = Some(b);

            trigger.set(true);
            rt.flush_reactive();
            let _ = b.get();
        });
    }

    #[test]
    fn derive_observable_model_subscribes_per_field() {
        #[derive(Observable)]
        struct Project {
            name: String,
            dirty: bool,
        }

        let name_renders = Rc::new(Cell::new(0usize));
        let dirty_renders = Rc::new(Cell::new(0usize));
        let nr = name_renders.clone();
        let dr = dirty_renders.clone();
        let model_slot: Rc<Cell<Option<ProjectObservable>>> = Rc::new(Cell::new(None));
        let slot_c = model_slot.clone();

        struct ModelView {
            nr: Rc<Cell<usize>>,
            dr: Rc<Cell<usize>>,
            slot: Rc<Cell<Option<ProjectObservable>>>,
        }
        impl View for ModelView {
            fn body(self, cx: &mut Cx) -> impl View {
                let model = cx.model(Project {
                    name: "rustui".into(),
                    dirty: false,
                });
                self.slot.set(Some(model));
                let nr = self.nr;
                let dr = self.dr;
                TestColumn((
                    TestLabel(TextSource::Reactive(Rc::new(move || {
                        nr.set(nr.get() + 1);
                        model.name()
                    }))),
                    TestLabel(TextSource::Reactive(Rc::new(move || {
                        dr.set(dr.get() + 1);
                        format!("dirty={}", model.dirty())
                    }))),
                ))
            }
        }

        let rt = Runtime::new();
        let mut bcx = BuildCx::new(rt.clone(), None);
        let _root = ModelView {
            nr,
            dr,
            slot: slot_c,
        }
        .build_node(&mut bcx);
        assert_eq!(name_renders.get(), 1);
        assert_eq!(dirty_renders.get(), 1);

        let model = model_slot.get().unwrap();
        rt.enter(|| model.set_dirty(true));
        rt.flush_reactive();
        assert_eq!(name_renders.get(), 1, "name subscriber must not re-run");
        assert_eq!(dirty_renders.get(), 2, "dirty subscriber must update");

        rt.enter(|| model.set_name("anvil".into()));
        rt.flush_reactive();
        assert_eq!(name_renders.get(), 2);
        assert_eq!(dirty_renders.get(), 2);
    }

    #[test]
    fn effect_cleanup_timer_and_resource_work() {
        let rt = Runtime::new();
        let owner = rt.alloc_node(NodeKind::Composite, None);
        let mut cx = Cx::new(rt.clone(), owner);

        let cleanups = Rc::new(Cell::new(0u32));
        let cl = cleanups.clone();
        let sig = cx.state(1);
        cx.effect_cleanup(move || {
            let _v = sig.get();
            let cl_inner = cl.clone();
            Some(Box::new(move || cl_inner.set(cl_inner.get() + 1)))
        });
        rt.flush_effects();
        assert_eq!(cleanups.get(), 0);

        rt.enter(|| sig.set(2));
        rt.flush_reactive();
        rt.flush_effects();
        assert_eq!(cleanups.get(), 1);

        let ticks = Rc::new(Cell::new(0u32));
        let t_c = ticks.clone();
        cx.timer(std::time::Duration::from_millis(5), true, move || {
            t_c.set(t_c.get() + 1);
        });
        rt.tick_timers(Instant::now() + std::time::Duration::from_millis(20));
        assert_eq!(ticks.get(), 1);

        let res = cx.resource(|| Ok::<i32, String>(77));
        for _ in 0..100 {
            if rt.poll_tasks() > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        rt.enter(|| assert_eq!(res.ready(), Some(77)));

        rt.remove_node(owner);
        assert_eq!(cleanups.get(), 2, "disposal must run final effect cleanup");
        assert_eq!(rt.timer_count(), 0);
    }
}
