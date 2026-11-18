//! Exhaustive end-to-end tests for RustUI stability across all layers and macOS SwiftUI parity.

use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionData, ActionRequest, Role, TreeId};
use rustui::core::{ContainerKind, DirtyFlags, NodeKind};
use rustui::platform::{HitTester, to_ak_id};
use rustui::prelude::*;
use rustui::render::DisplayCommand;

fn live_nodes(sim: &HeadlessSimulator) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![sim.root];
    sim.runtime.with_inner(|inner| {
        while let Some(id) = stack.pop() {
            let Some(node) = inner.nodes.get(id) else {
                continue;
            };
            if node.visual_props.hidden {
                continue;
            }
            out.push(id);
            for &c in node.children.iter().rev() {
                stack.push(c);
            }
        }
    });
    out
}

fn find_live_node<F>(sim: &HeadlessSimulator, predicate: F) -> Option<NodeId>
where
    F: Fn(&rustui::core::Node) -> bool,
{
    let mut stack = vec![sim.root];
    sim.runtime.with_inner(|inner| {
        while let Some(id) = stack.pop() {
            let Some(node) = inner.nodes.get(id) else {
                continue;
            };
            if node.visual_props.hidden {
                continue;
            }
            if predicate(node) {
                return Some(id);
            }
            for &c in node.children.iter().rev() {
                stack.push(c);
            }
        }
        None
    })
}

fn rect_center(r: Rect) -> Point {
    Point::new(r.x + r.width * 0.5, r.y + r.height * 0.5)
}

fn find_live_control(sim: &HeadlessSimulator, kind: &str, label_needle: &str) -> Option<NodeId> {
    find_live_node(sim, |node| {
        matches!(&node.kind, NodeKind::Control(c) if c.kind == kind)
            && (label_needle.is_empty()
                || node
                    .accessibility
                    .label
                    .as_deref()
                    .is_some_and(|l| l.contains(label_needle)))
    })
}

fn assert_layout_invariants(sim: &HeadlessSimulator, label: &str) {
    let vp = sim.viewport;
    let root_b = sim.node_bounds(sim.root).expect("root bounds");
    assert!(
        root_b.width <= vp.width + 1.0 && root_b.height <= vp.height + 1.0,
        "[{label}] root bounds {root_b:?} overflow viewport {vp:?}"
    );
    sim.runtime.with_inner(|inner| {
        for &id in &live_nodes(sim) {
            let Some(node) = inner.nodes.get(id) else {
                continue;
            };
            let b = node.bounds();
            assert!(
                b.x.is_finite() && b.y.is_finite() && b.width.is_finite() && b.height.is_finite(),
                "[{label}] non-finite bounds {b:?} on {:?}",
                node.kind
            );
            let is_zstack =
                matches!(node.kind, NodeKind::Container(ContainerKind::ZStack { .. }))
                    || node.accessibility.role == AccessibilityRole::Popover;
            if !is_zstack {
                let mut child_boxes = Vec::new();
                for &cid in &node.children {
                    if let Some(cn) = inner.nodes.get(cid)
                        && !cn.visual_props.hidden
                        && cn.accessibility.role != AccessibilityRole::Popover
                    {
                        let cb = cn.bounds();
                        if cb.width > 2.0 && cb.height > 2.0 {
                            child_boxes.push((cid, cb, &cn.kind));
                        }
                    }
                }
                for i in 0..child_boxes.len() {
                    for j in (i + 1)..child_boxes.len() {
                        let (_, ra, ka) = &child_boxes[i];
                        let (_, rb, kb) = &child_boxes[j];
                        if let Some(inter) = ra.intersection(rb)
                            && inter.width > 1.5
                            && inter.height > 1.5
                        {
                            panic!(
                                "[{label}] sibling overlap in {:?}: {ka:?} ({ra:?}) overlaps {kb:?} ({rb:?}) by {inter:?}",
                                node.kind
                            );
                        }
                    }
                }
            }
        }
    });
}

#[test]
fn reactive_button_syncs_accessibility_label_on_build_and_update() {
    struct ReactiveBtnView;
    impl View for ReactiveBtnView {
        fn body(self, cx: &mut Cx) -> impl View {
            let count = cx.state(0_i32);
            Button::new(
                move || format!("Counter {}", count.get()),
                move || count.update(|c| *c += 1),
            )
            .padding(16)
        }
    }

    let mut sim = HeadlessSimulator::new(ReactiveBtnView, Size::new(400.0, 200.0));
    let btn = find_live_control(&sim, "button", "Counter 0")
        .expect("reactive button must expose initial accessibility label");

    let tree_before = sim.accessibility_tree();
    assert!(
        tree_before
            .nodes
            .iter()
            .any(|(id, ak)| *id == to_ak_id(btn) && ak.label() == Some("Counter 0"))
    );

    sim.click_node(btn);

    assert!(
        find_live_control(&sim, "button", "Counter 1").is_some(),
        "button control accessibility.label must update reactively"
    );
    let tree_after = sim.accessibility_tree();
    assert!(
        tree_after
            .nodes
            .iter()
            .any(|(id, ak)| *id == to_ak_id(btn) && ak.label() == Some("Counter 1"))
    );
}

#[test]
fn external_binding_syncs_textfield_securefield_and_texteditor_accessibility_value() {
    #[derive(Clone, Default)]
    struct Handles {
        tf: Rc<RefCell<Option<State<String>>>>,
        sf: Rc<RefCell<Option<State<String>>>>,
        te: Rc<RefCell<Option<State<String>>>>,
    }
    struct ExternalSyncView(Handles);
    impl View for ExternalSyncView {
        fn body(self, cx: &mut Cx) -> impl View {
            let tf = cx.state("initial".to_owned());
            let sf = cx.state("abc".to_owned());
            let te = cx.state("hello".to_owned());
            *self.0.tf.borrow_mut() = Some(tf);
            *self.0.sf.borrow_mut() = Some(sf);
            *self.0.te.borrow_mut() = Some(te);
            VStack::new(
                10,
                (
                    TextField::new("Name", tf.binding()),
                    SecureField::new("Password", sf.binding()),
                    TextEditor::new(te.binding()),
                ),
            )
            .padding(16)
        }
    }

    let handles = Handles::default();
    let mut sim =
        HeadlessSimulator::new(ExternalSyncView(handles.clone()), Size::new(500.0, 420.0));

    sim.runtime.enter(|| {
        handles.tf.borrow().unwrap().set("renamed".to_owned());
        handles.sf.borrow().unwrap().set("secret9".to_owned());
        handles
            .te
            .borrow()
            .unwrap()
            .set("line 1\nline 2".to_owned());
    });
    sim.step_frame();

    let tf_node = find_live_control(&sim, "textfield", "Name").unwrap();
    let sf_node = find_live_control(&sim, "securefield", "Password").unwrap();
    let te_node = find_live_control(&sim, "texteditor", "Text Editor").unwrap();

    sim.runtime.with_inner(|inner| {
        assert_eq!(
            inner.nodes[tf_node].accessibility.value.as_deref(),
            Some("renamed")
        );
        assert_eq!(
            inner.nodes[sf_node].accessibility.value.as_deref(),
            Some("•••••••")
        );
        assert_eq!(
            inner.nodes[te_node].accessibility.value.as_deref(),
            Some("line 1\nline 2")
        );
    });
}

#[test]
fn disabled_controls_and_subtree_propagation_block_all_inputs() {
    #[derive(Clone, Default)]
    struct Handles {
        clicks: Rc<RefCell<Option<State<i32>>>>,
        toggled: Rc<RefCell<Option<State<bool>>>>,
        checked: Rc<RefCell<Option<State<bool>>>>,
        slider: Rc<RefCell<Option<State<f32>>>>,
        picked: Rc<RefCell<Option<State<usize>>>>,
        field: Rc<RefCell<Option<State<String>>>>,
        stepper: Rc<RefCell<Option<State<f32>>>>,
        color: Rc<RefCell<Option<State<Color>>>>,
        date: Rc<RefCell<Option<State<String>>>>,
        opened: Rc<RefCell<Option<State<i32>>>>,
    }
    struct DisabledFormView(Handles);
    impl View for DisabledFormView {
        fn body(self, cx: &mut Cx) -> impl View {
            let clicks = cx.state(0);
            let toggled = cx.state(false);
            let checked = cx.state(false);
            let slider = cx.state(25.0_f32);
            let picked = cx.state(0_usize);
            let field = cx.state("fixed".to_owned());
            let stepper = cx.state(2.0_f32);
            let color = cx.state(Color::from_hex(0x007AFF));
            let date = cx.state("2026-11-18".to_owned());
            let opened = cx.state(0);

            *self.0.clicks.borrow_mut() = Some(clicks);
            *self.0.toggled.borrow_mut() = Some(toggled);
            *self.0.checked.borrow_mut() = Some(checked);
            *self.0.slider.borrow_mut() = Some(slider);
            *self.0.picked.borrow_mut() = Some(picked);
            *self.0.field.borrow_mut() = Some(field);
            *self.0.stepper.borrow_mut() = Some(stepper);
            *self.0.color.borrow_mut() = Some(color);
            *self.0.date.borrow_mut() = Some(date);
            *self.0.opened.borrow_mut() = Some(opened);

            VStack::new(
                10,
                (
                    // Subtree disabled on Form overrides child enabled controls.
                    Form::new((
                        Button::new("SubtreeBtn", move || clicks.update(|c| *c += 1)),
                        Toggle::new("SubtreeToggle", toggled.binding()),
                        Checkbox::new("SubtreeCheck", checked.binding()),
                    ))
                    .disabled(true),
                    // Direct `.disabled(true)` on every control.
                    Slider::new(slider.binding(), 0.0, 100.0).disabled(true),
                    Picker::new(picked.binding(), vec!["A".into(), "B".into()])
                        .label("DirectPicker")
                        .disabled(true),
                    TextField::new("LockedField", field.binding()).disabled(true),
                    Stepper::new("LockedStepper", stepper.binding(), 0.0..=10.0).disabled(true),
                    ColorPicker::new("LockedColor", color.binding()).disabled(true),
                    DatePicker::new("LockedDate", date.binding()).disabled(true),
                    Link::new("LockedLink", "https://example.com")
                        .on_open(move |_| opened.update(|c| *c += 1))
                        .disabled(true),
                    // Explicit child `.disabled(true)` inside `.disabled(false)` parent stays disabled.
                    VStack::new(
                        4,
                        Button::new("NestedDisabledBtn", move || clicks.update(|c| *c += 1))
                            .disabled(true),
                    )
                    .disabled(false),
                ),
            )
            .padding(12)
        }
    }

    let handles = Handles::default();
    let mut sim =
        HeadlessSimulator::new(DisabledFormView(handles.clone()), Size::new(620.0, 820.0));

    let btn = find_live_control(&sim, "button", "SubtreeBtn").unwrap();
    let nested_btn = find_live_control(&sim, "button", "NestedDisabledBtn").unwrap();
    let tog = find_live_control(&sim, "toggle", "SubtreeToggle").unwrap();
    let chk = find_live_control(&sim, "checkbox", "SubtreeCheck").unwrap();
    let sld = find_live_control(&sim, "slider", "Slider").unwrap();
    let pck = find_live_control(&sim, "picker", "DirectPicker").unwrap();
    let fld = find_live_control(&sim, "textfield", "LockedField").unwrap();
    let stp = find_live_control(&sim, "stepper", "LockedStepper").unwrap();
    let col = find_live_control(&sim, "colorpicker", "LockedColor").unwrap();
    let dtp = find_live_control(&sim, "datepicker", "LockedDate").unwrap();
    let lnk = find_live_control(&sim, "link", "LockedLink").unwrap();

    for &id in &[btn, nested_btn, tog, chk, sld, pck, fld, stp, col, dtp, lnk] {
        sim.click_node(id);
        sim.press_key(KeyCode::Enter, Modifiers::NONE);
        sim.press_key(KeyCode::ArrowRight, Modifiers::NONE);
        assert!(
            sim.focus.focused_node() != Some(id),
            "disabled node {:?} must never take focus",
            id
        );
    }

    let sb = sim.node_bounds(sld).unwrap();
    let sc = rect_center(sb);
    sim.drag(sc, Point::new(sb.max_x(), sc.y));
    sim.click_node(fld);
    sim.type_text("should-not-insert");

    assert!(!sim.dispatch_accessibility_action(ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: to_ak_id(btn),
        data: None,
    }));
    assert!(!sim.dispatch_accessibility_action(ActionRequest {
        action: Action::Increment,
        target_tree: TreeId::ROOT,
        target_node: to_ak_id(sld),
        data: None,
    }));
    assert!(!sim.dispatch_accessibility_action(ActionRequest {
        action: Action::Increment,
        target_tree: TreeId::ROOT,
        target_node: to_ak_id(stp),
        data: None,
    }));
    assert!(!sim.dispatch_accessibility_action(ActionRequest {
        action: Action::SetValue,
        target_tree: TreeId::ROOT,
        target_node: to_ak_id(fld),
        data: Some(ActionData::Value("hacked".into())),
    }));

    sim.runtime.enter(|| {
        assert_eq!(handles.clicks.borrow().unwrap().get_untracked(), 0);
        assert!(!handles.toggled.borrow().unwrap().get_untracked());
        assert!(!handles.checked.borrow().unwrap().get_untracked());
        assert!((handles.slider.borrow().unwrap().get_untracked() - 25.0).abs() < f32::EPSILON);
        assert_eq!(handles.picked.borrow().unwrap().get_untracked(), 0);
        assert_eq!(handles.field.borrow().unwrap().get_untracked(), "fixed");
        assert!((handles.stepper.borrow().unwrap().get_untracked() - 2.0).abs() < f32::EPSILON);
        assert_eq!(
            handles.color.borrow().unwrap().get_untracked(),
            Color::from_hex(0x007AFF)
        );
        assert_eq!(handles.date.borrow().unwrap().get_untracked(), "2026-11-18");
        assert_eq!(handles.opened.borrow().unwrap().get_untracked(), 0);
    });

    let dl = sim.build_display_list();
    assert!(
        dl.commands
            .iter()
            .any(|cmd| matches!(cmd, DisplayCommand::PushLayer { opacity, .. } if *opacity < 0.6))
    );
}

#[test]
fn control_size_hierarchy_and_picker_styles_behave_like_macos_swiftui() {
    #[derive(Clone, Default)]
    struct Handles {
        seg: Rc<RefCell<Option<State<usize>>>>,
        radio: Rc<RefCell<Option<State<usize>>>>,
        menu: Rc<RefCell<Option<State<usize>>>>,
    }
    struct ControlAndPickerView(Handles);
    impl View for ControlAndPickerView {
        fn body(self, cx: &mut Cx) -> impl View {
            let seg = cx.state(0_usize);
            let radio = cx.state(0_usize);
            let menu = cx.state(0_usize);
            *self.0.seg.borrow_mut() = Some(seg);
            *self.0.radio.borrow_mut() = Some(radio);
            *self.0.menu.borrow_mut() = Some(menu);
            VStack::new(
                14,
                (
                    HStack::new(
                        10,
                        (
                            Button::new("MiniBtn", || {}).control_size(ControlSize::Mini),
                            Button::new("SmallBtn", || {}).control_size(ControlSize::Small),
                            Button::new("RegularBtn", || {}).control_size(ControlSize::Regular),
                            Button::new("LargeBtn", || {}).control_size(ControlSize::Large),
                        ),
                    ),
                    Picker::new(
                        seg.binding(),
                        vec!["Day".into(), "Week".into(), "Month".into()],
                    )
                    .label("CalendarScale")
                    .picker_style(PickerStyle::Segmented),
                    Picker::new(radio.binding(), vec!["Automatic".into(), "Manual".into()])
                        .label("UpdateMode")
                        .picker_style(PickerStyle::RadioGroup),
                    Picker::new(menu.binding(), vec!["Low".into(), "High".into()]).label("Quality"),
                ),
            )
            .padding(16)
        }
    }

    let handles = Handles::default();
    let mut sim = HeadlessSimulator::new(
        ControlAndPickerView(handles.clone()),
        Size::new(640.0, 480.0),
    );
    assert_layout_invariants(&sim, "control_size_and_picker");

    let h_mini = sim
        .node_bounds(find_live_control(&sim, "button", "MiniBtn").unwrap())
        .unwrap()
        .height;
    let h_small = sim
        .node_bounds(find_live_control(&sim, "button", "SmallBtn").unwrap())
        .unwrap()
        .height;
    let h_reg = sim
        .node_bounds(find_live_control(&sim, "button", "RegularBtn").unwrap())
        .unwrap()
        .height;
    let h_large = sim
        .node_bounds(find_live_control(&sim, "button", "LargeBtn").unwrap())
        .unwrap()
        .height;
    assert!(
        h_mini < h_small && h_small < h_reg && h_reg < h_large,
        "ControlSize heights must increase monotonically: {h_mini} < {h_small} < {h_reg} < {h_large}"
    );

    let month_tab = find_live_control(&sim, "picker", "Month").unwrap();
    sim.click_node(month_tab);
    assert_eq!(
        sim.runtime
            .enter(|| handles.seg.borrow().unwrap().get_untracked()),
        2
    );

    sim.press_key(KeyCode::ArrowLeft, Modifiers::NONE);
    assert_eq!(
        sim.runtime
            .enter(|| handles.seg.borrow().unwrap().get_untracked()),
        1
    );

    let manual_radio = find_live_control(&sim, "picker", "Manual").unwrap();
    sim.click_node(manual_radio);
    assert_eq!(
        sim.runtime
            .enter(|| handles.radio.borrow().unwrap().get_untracked()),
        1
    );

    let quality_menu = find_live_control(&sim, "picker", "Quality").unwrap();
    assert!(sim.dispatch_accessibility_action(ActionRequest {
        action: Action::Increment,
        target_tree: TreeId::ROOT,
        target_node: to_ak_id(quality_menu),
        data: None,
    }));
    assert_eq!(
        sim.runtime
            .enter(|| handles.menu.borrow().unwrap().get_untracked()),
        1
    );
}

#[test]
fn macos_form_section_groupbox_stepper_colorpicker_datepicker_and_link() {
    #[derive(Clone, Default)]
    struct Handles {
        step_val: Rc<RefCell<Option<State<f32>>>>,
        color_val: Rc<RefCell<Option<State<Color>>>>,
        date_val: Rc<RefCell<Option<State<String>>>>,
        opened_urls: Rc<RefCell<Option<State<Vec<String>>>>>,
    }
    struct MacSettingsView(Handles);
    impl View for MacSettingsView {
        fn body(self, cx: &mut Cx) -> impl View {
            let step_val = cx.state(2.0_f32);
            let color_val = cx.state(Color::from_hex(0x007AFF));
            let date_val = cx.state("2024-02-28".to_owned());
            let opened_urls = cx.state(Vec::<String>::new());

            *self.0.step_val.borrow_mut() = Some(step_val);
            *self.0.color_val.borrow_mut() = Some(color_val);
            *self.0.date_val.borrow_mut() = Some(date_val);
            *self.0.opened_urls.borrow_mut() = Some(opened_urls);

            Form::new((
                Section::new(
                    "Preferences",
                    (
                        LabeledContent::new(
                            "Docs",
                            Link::new(
                                "SwiftUI Guide",
                                "https://developer.apple.com/documentation/swiftui",
                            )
                            .on_open(move |url| {
                                opened_urls.update(|list| list.push(url.to_owned()))
                            }),
                        ),
                        Stepper::new("Timeout Seconds", step_val.binding(), 1.0..=5.0),
                        ColorPicker::new("Highlight", color_val.binding()),
                        DatePicker::new("Milestone", date_val.binding()),
                    ),
                )
                .footer("Applies immediately on macOS."),
                GroupBox::new("Summary", Text::new("All preferences synced.")),
            ))
        }
    }

    let handles = Handles::default();
    let mut sim = HeadlessSimulator::new(MacSettingsView(handles.clone()), Size::new(620.0, 560.0));
    assert_layout_invariants(&sim, "macos_settings_form");

    let plus = find_live_control(&sim, "button", "+").unwrap();
    sim.click_node(plus);
    assert!(
        (sim.runtime
            .enter(|| handles.step_val.borrow().unwrap().get_untracked())
            - 3.0)
            .abs()
            < 0.001
    );

    let stepper_ctrl = find_live_control(&sim, "stepper", "Timeout Seconds").unwrap();
    assert!(sim.dispatch_accessibility_action(ActionRequest {
        action: Action::Decrement,
        target_tree: TreeId::ROOT,
        target_node: to_ak_id(stepper_ctrl),
        data: None,
    }));
    assert!(
        (sim.runtime
            .enter(|| handles.step_val.borrow().unwrap().get_untracked())
            - 2.0)
            .abs()
            < 0.001
    );

    let next_date = find_live_control(&sim, "button", "›").unwrap();
    sim.click_node(next_date);
    assert_eq!(
        sim.runtime
            .enter(|| handles.date_val.borrow().unwrap().get_untracked()),
        "2024-02-29"
    );
    sim.click_node(next_date);
    assert_eq!(
        sim.runtime
            .enter(|| handles.date_val.borrow().unwrap().get_untracked()),
        "2024-03-01"
    );

    let cp = find_live_control(&sim, "colorpicker", "Highlight").unwrap();
    sim.click_node(cp);
    assert_eq!(
        sim.runtime
            .enter(|| handles.color_val.borrow().unwrap().get_untracked()),
        Color::from_hex(0xAF52DE)
    );

    let lnk = find_live_control(&sim, "link", "SwiftUI Guide").unwrap();
    let tree = sim.accessibility_tree();
    assert!(
        tree.nodes
            .iter()
            .any(|(id, ak)| *id == to_ak_id(lnk) && ak.role() == Role::Link)
    );
    sim.click_node(lnk);
    assert_eq!(
        sim.runtime
            .enter(|| handles.opened_urls.borrow().unwrap().get_untracked()),
        vec!["https://developer.apple.com/documentation/swiftui".to_owned()]
    );
}

#[test]
fn help_tooltip_badge_slider_keys_high_contrast_and_theme_matrix() {
    #[derive(Clone)]
    struct ShowcaseView {
        slider_ref: Rc<RefCell<Option<State<f32>>>>,
        theme: Theme,
    }
    impl View for ShowcaseView {
        fn body(self, cx: &mut Cx) -> impl View {
            let slider = cx.state(50.0_f32);
            *self.slider_ref.borrow_mut() = Some(slider);
            let palette = self.theme.to_palette();
            VStack::new(
                14,
                (
                    Button::new("SaveDoc", || {})
                        .badge("99+")
                        .help("Save current document"),
                    Slider::new(slider.binding(), 0.0, 100.0).step(10.0),
                ),
            )
            .padding(20)
            .environment(self.theme)
            .environment(palette)
        }
    }

    for (name, theme, scale, hc, reduced_motion) in [
        ("light-100", Theme::light(), 1.0, false, false),
        ("dark-100", Theme::dark(), 1.0, false, false),
        ("hc-200-reduced", Theme::dark(), 2.0, true, true),
    ] {
        let slider_ref = Rc::new(RefCell::new(None));
        let palette = theme.to_palette();
        let mut sim = HeadlessSimulator::new(
            ShowcaseView {
                slider_ref: slider_ref.clone(),
                theme,
            },
            Size::new(560.0, 320.0),
        );
        sim.runtime.set_high_contrast(hc);
        sim.runtime.set_text_scale_factor(scale);
        sim.runtime.set_reduced_motion(reduced_motion);
        sim.step_frame();
        assert_layout_invariants(&sim, name);

        let slider = slider_ref.borrow().unwrap();
        let sld = find_live_control(&sim, "slider", "Slider").unwrap();
        sim.click_node(sld);
        sim.press_key(KeyCode::Home, Modifiers::NONE);
        assert!((sim.runtime.enter(|| slider.get_untracked()) - 0.0).abs() < 0.001);
        sim.press_key(KeyCode::PageUp, Modifiers::NONE);
        assert!((sim.runtime.enter(|| slider.get_untracked()) - 20.0).abs() < 0.001);
        sim.press_key(KeyCode::End, Modifiers::NONE);
        assert!((sim.runtime.enter(|| slider.get_untracked()) - 100.0).abs() < 0.001);
        sim.press_key(KeyCode::PageDown, Modifiers::NONE);
        assert!((sim.runtime.enter(|| slider.get_untracked()) - 80.0).abs() < 0.001);

        let before_hover = sim.build_display_list();
        assert!(
            before_hover.commands.iter().any(|c| matches!(
                c,
                DisplayCommand::FillRoundedRect { color, .. }
                    if (color.r - palette.destructive.r).abs() < 0.05
            )),
            "[{name}] badge fill must be emitted"
        );

        if hc {
            assert!(
                before_hover.commands.iter().any(|c| matches!(
                    c,
                    DisplayCommand::StrokeRoundedRect { width, .. } if *width >= 1.5
                )),
                "[{name}] high-contrast must emit >= 1.5px stroke borders"
            );
        }

        let btn = find_live_control(&sim, "button", "SaveDoc").unwrap();
        let center = rect_center(sim.node_bounds(btn).unwrap());
        sim.gestures.handle_pointer(
            &sim.runtime,
            sim.root,
            &mut sim.focus,
            PointerEvent {
                position: center,
                button: PointerButton::Primary,
                phase: PointerPhase::Move,
                click_count: 0,
                modifiers: Modifiers::NONE,
            },
        );
        sim.step_frame();
        assert!(
            sim.runtime
                .with_inner(|inner| inner.nodes[btn].visual_props.is_hovered),
            "[{name}] SaveDoc must mark is_hovered on hover"
        );
        let while_hover = sim.build_display_list();
        assert!(
            while_hover.commands.len() > before_hover.commands.len(),
            "[{name}] hovering .help() target must emit floating tooltip commands"
        );
        assert!(HitTester::hit_test(&sim.runtime, sim.root, center).is_some());
        assert!(DirtyFlags::NONE.is_empty());
    }
}
