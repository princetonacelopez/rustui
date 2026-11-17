//! Cross-platform windowing, input, focus, clipboard, menus, AccessKit bridge, and headless simulator.

pub mod accessibility;
pub mod input;
pub mod services;
pub mod simulator;
pub mod window;

pub use accessibility::{AccessKitBridge, WINDOW_AK_ROOT_ID, from_ak_id, to_ak_id};
pub use input::{
    ChordMatch, FocusManager, GestureRecognizer, HitTester, KeyChord, KeyContext, KeyStroke,
    KeybindingEntry, KeybindingRegistry, key_down,
};
pub use services::{
    DesktopPlatform, FileDialogOptions, HotReloadHost, MenuItemSpec, MenuSpec, NotificationSpec,
    Platform, PlatformCapabilities, detect_platform_capabilities, detect_reduced_motion,
    detect_system_theme,
};
pub use simulator::{FrameSummary, HeadlessSimulator};
pub use window::{
    AppEvent, SystemTheme, WindowManager, WindowOptions, WindowSpec, run_desktop_windows,
};

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use accesskit::{Action, ActionRequest, Role, TreeId};
    use rustui_core::{
        AccessibilityRole, BuildCx, Color, ContainerKind, EmptyView, Frame, HorizontalAlignment,
        KeyCode, Modifiers, NodeId, NodeKind, Point, Primitive, Size, TextNodeData, View, ViewExt,
    };

    use super::*;

    struct TestButton {
        label: &'static str,
        on_tap: Rc<dyn Fn()>,
    }

    impl Primitive for TestButton {
        fn build(self, cx: &mut BuildCx) -> NodeId {
            let id = cx.alloc_node(NodeKind::Control(rustui_core::ControlNodeData {
                kind: "button",
                ..Default::default()
            }));
            let tap_cb = self.on_tap.clone();
            let tap_for_key = self.on_tap;
            let lbl = self.label.to_owned();
            cx.runtime.with_inner_mut(|inner| {
                let n = inner.nodes.get_mut(id).unwrap();
                n.layout_props.frame = Some(Frame::fixed(120.0, 36.0));
                n.visual_props.background = Some(Color::BLUE);
                n.focus.focusable = true;
                n.accessibility.role = AccessibilityRole::Button;
                n.accessibility.label = Some(lbl);
                n.accessibility.supports_click = true;
                n.accessibility.supports_focus = true;
                n.events.on_tap = Some(tap_cb);
                n.events.on_key = Some(Rc::new(move |ev| {
                    if ev.is_down && matches!(ev.key, KeyCode::Enter | KeyCode::Space) {
                        tap_for_key();
                        return true;
                    }
                    false
                }));
            });
            id
        }
    }

    impl View for TestButton {
        fn body(self, _cx: &mut rustui_core::Cx) -> impl View {
            EmptyView
        }
        fn build_node(self, cx: &mut BuildCx) -> NodeId {
            Primitive::build(self, cx)
        }
    }

    struct TestStack<C>(C);

    impl<C: rustui_core::ViewSequence> Primitive for TestStack<C> {
        fn build(self, cx: &mut BuildCx) -> NodeId {
            let id = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
                spacing: 8.0,
                alignment: HorizontalAlignment::Leading,
            }));
            self.0.build_all(cx, id);
            id
        }
    }

    impl<C: rustui_core::ViewSequence> View for TestStack<C> {
        fn body(self, _cx: &mut rustui_core::Cx) -> impl View {
            EmptyView
        }
        fn build_node(self, cx: &mut BuildCx) -> NodeId {
            Primitive::build(self, cx)
        }
    }

    #[test]
    fn keyboard_only_tab_traversal_and_activation_via_simulator() {
        let count_a = Rc::new(Cell::new(0));
        let count_b = Rc::new(Cell::new(0));
        let ca = count_a.clone();
        let cb = count_b.clone();

        let view = TestStack((
            TestButton {
                label: "First",
                on_tap: Rc::new(move || ca.set(ca.get() + 1)),
            },
            TestButton {
                label: "Second",
                on_tap: Rc::new(move || cb.set(cb.get() + 1)),
            },
        ));

        let mut sim = HeadlessSimulator::new(view, Size::new(400.0, 300.0));

        // Tab -> focuses First -> Enter activates First
        sim.press_key(KeyCode::Tab, Modifiers::NONE);
        sim.press_key(KeyCode::Enter, Modifiers::NONE);
        assert_eq!(count_a.get(), 1);
        assert_eq!(count_b.get(), 0);

        // Tab -> focuses Second -> Space activates Second
        sim.press_key(KeyCode::Tab, Modifiers::NONE);
        sim.press_key(KeyCode::Space, Modifiers::NONE);
        assert_eq!(count_a.get(), 1);
        assert_eq!(count_b.get(), 1);

        // Shift+Tab -> focuses First again
        sim.press_key(
            KeyCode::Tab,
            Modifiers {
                shift: true,
                ..Modifiers::NONE
            },
        );
        sim.press_key(KeyCode::Enter, Modifiers::NONE);
        assert_eq!(count_a.get(), 2);
    }

    #[test]
    fn hit_testing_respects_clip_bounds_and_dispatches_accesskit_actions() {
        let clicked = Rc::new(Cell::new(false));
        let cl = clicked.clone();

        let view = TestStack((
            TestButton {
                label: "Submit",
                on_tap: Rc::new(move || cl.set(true)),
            },
            TestButton {
                label: "Clipped",
                on_tap: Rc::new(|| {}),
            }
            .offset(0, 200)
            .frame(Frame::fixed(120.0, 20.0))
            .clipped(),
        ));

        let mut sim = HeadlessSimulator::new(view, Size::new(300.0, 200.0));
        let submit_id = sim.find_node_by_label("Submit").expect("Submit node");
        let clipped_id = sim.find_node_by_label("Clipped").expect("Clipped node");

        // Clipped button was offset outside its clipped container's bounds; hit test at y=250 returns None.
        assert_ne!(
            HitTester::hit_test(&sim.runtime, sim.root, Point::new(60.0, 250.0)),
            Some(clipped_id)
        );

        // Inspect AccessKit tree & dispatch screen-reader Click action on Submit!
        let tree = sim.accessibility_tree();
        let has_submit_button = tree.nodes.iter().any(|(ak_id, ak_node)| {
            *ak_id == to_ak_id(submit_id)
                && ak_node.role() == Role::Button
                && ak_node.label() == Some("Submit")
        });
        assert!(has_submit_button);

        let handled = sim.dispatch_accessibility_action(ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: to_ak_id(submit_id),
            data: None,
        });
        assert!(handled);
        assert!(clicked.get());
    }

    #[test]
    fn idle_frame_does_not_redraw_and_clipboard_fallback_works() {
        let view = TestButton {
            label: "Idle",
            on_tap: Rc::new(|| {}),
        };
        let mut sim = HeadlessSimulator::new(view, Size::new(200.0, 120.0));
        let second = sim.step_frame();
        assert!(!second.redrawn, "idle app must produce 0 redraws");

        let png = sim.render_png().expect("headless png");
        assert_eq!(&png[0..8], b"\x89PNG\r\n\x1a\n");

        let platform = DesktopPlatform::headless();
        platform.clipboard_set("RustUI Clipboard");
        assert_eq!(
            platform.clipboard_get().as_deref(),
            Some("RustUI Clipboard")
        );
    }

    #[test]
    fn hit_test_checks_text_node_when_tappable() {
        let tapped = Rc::new(Cell::new(false));
        let tc = tapped.clone();

        struct TappableText(Rc<Cell<bool>>);
        impl Primitive for TappableText {
            fn build(self, cx: &mut BuildCx) -> NodeId {
                let id = cx.alloc_node(NodeKind::Text(TextNodeData {
                    text: "Click Me".into(),
                    ..Default::default()
                }));
                let cb = self.0;
                cx.runtime.with_inner_mut(|inner| {
                    inner.nodes.get_mut(id).unwrap().events.on_tap =
                        Some(Rc::new(move || cb.set(true)));
                });
                id
            }
        }
        impl View for TappableText {
            fn body(self, _cx: &mut rustui_core::Cx) -> impl View {
                EmptyView
            }
            fn build_node(self, cx: &mut BuildCx) -> NodeId {
                Primitive::build(self, cx)
            }
        }

        let mut sim = HeadlessSimulator::new(TappableText(tc), Size::new(200.0, 80.0));
        let node = sim.find_node_by_label("Click Me").unwrap();
        sim.click_node(node);
        assert!(tapped.get());
    }

    #[test]
    fn keybinding_chords_dnd_hot_reload_and_extended_accessibility_work() {
        let mut reg = KeybindingRegistry::new();
        reg.bind_str("Ctrl+K Ctrl+S", "open_shortcuts", KeyContext::Global)
            .unwrap();
        let toml = reg.to_toml_string();
        let mut parsed = KeybindingRegistry::from_toml_str(&toml).unwrap();
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        assert_eq!(
            parsed.match_event(
                &key_down(KeyCode::Character("k".into()), ctrl, None),
                KeyContext::Global
            ),
            ChordMatch::PendingPrefix
        );
        assert_eq!(
            parsed.match_event(
                &key_down(KeyCode::Character("s".into()), ctrl, None),
                KeyContext::Global
            ),
            ChordMatch::Command("open_shortcuts".into())
        );

        let dropped: Rc<std::cell::RefCell<Option<String>>> =
            Rc::new(std::cell::RefCell::new(None));
        let dr = dropped.clone();
        let custom_ran = Rc::new(Cell::new(false));
        let cr = custom_ran.clone();

        struct DndView {
            dr: Rc<std::cell::RefCell<Option<String>>>,
            cr: Rc<Cell<bool>>,
        }
        impl View for DndView {
            fn body(self, _cx: &mut rustui_core::Cx) -> impl View {
                let d = self.dr;
                let c = self.cr;
                TestButton {
                    label: "DropHere",
                    on_tap: Rc::new(|| {}),
                }
                .accessibility_hint("Drops files onto target")
                .accessibility_live(rustui_core::AccessibilityLive::Polite)
                .accessibility_custom_action("ClearTarget", move || c.set(true))
                .on_drop(move |ev| {
                    if let rustui_core::DragPayload::Files(paths) = ev.payload {
                        *d.borrow_mut() = Some(paths[0].display().to_string());
                        return true;
                    }
                    false
                })
            }
        }

        let mut sim = HeadlessSimulator::new(DndView { dr, cr }, Size::new(240.0, 120.0));
        let target = sim.find_node_by_label("DropHere").unwrap();
        let tb = sim.node_bounds(target).unwrap();
        sim.drop_files(
            Point::new(tb.mid_x(), tb.mid_y()),
            vec![std::path::PathBuf::from("/tmp/demo.rs")],
        );
        assert_eq!(dropped.borrow().as_deref(), Some("/tmp/demo.rs"));

        let tree = sim.accessibility_tree();
        let ak_node = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == to_ak_id(target))
            .map(|(_, n)| n)
            .unwrap();
        assert_eq!(ak_node.description(), Some("Drops files onto target"));
        assert_eq!(ak_node.live(), Some(accesskit::Live::Polite));
        assert!(sim.dispatch_accessibility_action(ActionRequest {
            action: Action::CustomAction,
            target_tree: TreeId::ROOT,
            target_node: to_ak_id(target),
            data: Some(accesskit::ActionData::CustomAction(0)),
        }));
        assert!(custom_ran.get());

        // HotReloadHost replaces children cleanly.
        let mut reloader = HotReloadHost::new();
        let _diag = reloader.reload_view(
            &sim.runtime,
            sim.root,
            TestButton {
                label: "Reloaded",
                on_tap: Rc::new(|| {}),
            },
        );
        sim.step_frame();
        assert!(sim.find_node_by_label("Reloaded").is_some());
    }
}
