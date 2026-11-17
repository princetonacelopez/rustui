//! End-to-end desktop example interaction, layout, state, overlay, and AccessKit tests.

use std::collections::{HashMap, HashSet};

use accesskit::{Action, ActionData, ActionRequest, NodeId as AkNodeId, Role, TreeId};
use rustui::core::{ContainerKind, NodeKind};
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

fn find_live_text(sim: &HeadlessSimulator, substring: &str) -> Option<NodeId> {
    find_live_node(
        sim,
        |node| matches!(&node.kind, NodeKind::Text(t) if t.text.contains(substring)),
    )
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
        (root_b.width - vp.width).abs() <= 0.5 && (root_b.height - vp.height).abs() <= 0.5,
        "[{label}] root bounds {root_b:?} must match viewport {vp:?}"
    );

    sim.runtime.with_inner(|inner| {
        // Walk from root, tracking whether we are inside a scrollable or virtualized ancestor.
        let mut stack = vec![(sim.root, false, false)];
        while let Some((id, in_scroll, in_lazy)) = stack.pop() {
            let Some(node) = inner.nodes.get(id) else {
                continue;
            };
            if node.visual_props.hidden {
                continue;
            }
            let b = node.bounds();
            let next_scroll = in_scroll
                || matches!(node.kind, NodeKind::Scroll(_) | NodeKind::VirtualList(_))
                || matches!(&node.kind, NodeKind::Control(c) if c.kind == "list" || c.kind == "table");
            let next_lazy = in_lazy || matches!(node.kind, NodeKind::VirtualList(_));

            // LazyVStack resets unused pooled slots to Rect::ZERO intentionally.
            let is_recycled_zero = next_lazy && b.width == 0.0 && b.height == 0.0;

            if !is_recycled_zero {
                if let NodeKind::Text(t) = &node.kind
                    && !t.text.is_empty()
                {
                    assert!(
                        b.width > 0.0 && b.height > 0.0,
                        "[{label}] text {:?} has degenerate bounds {b:?}",
                        t.text
                    );
                }
                if let NodeKind::Control(c) = &node.kind {
                    assert!(
                        b.width >= 6.0 && b.height >= 6.0,
                        "[{label}] control {:?} (label={:?}) has degenerate bounds {b:?}",
                        c.kind,
                        node.accessibility.label
                    );
                }

                if !in_scroll && (b.width > 0.0 || b.height > 0.0) {
                    assert!(
                        b.x >= -1.0
                            && b.y >= -1.0
                            && b.max_x() <= vp.width + 1.0
                            && b.max_y() <= vp.height + 1.0,
                        "[{label}] node {:?} ({:?}) exceeds viewport {vp:?}: {b:?}",
                        node.kind,
                        node.accessibility.label,
                        b = b
                    );
                }
            }

            // Adjacent child controls inside non-ZStack, non-Popover containers must not overlap.
            let is_zstack = matches!(node.kind, NodeKind::Container(ContainerKind::ZStack { .. }))
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

            for &cid in &node.children {
                stack.push((cid, next_scroll, next_lazy));
            }
        }
    });
}

fn assert_accesskit_valid(sim: &HeadlessSimulator, label: &str) {
    let update = sim.accessibility_tree();
    assert!(update.tree.is_some(), "[{label}] missing TreeInfo");
    assert_eq!(update.tree_id, TreeId::ROOT);

    let nodes_by_id: HashMap<AkNodeId, &accesskit::Node> =
        update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    assert!(
        nodes_by_id.values().any(|n| n.role() == Role::Window),
        "[{label}] missing Role::Window"
    );
    assert!(
        nodes_by_id.contains_key(&update.focus),
        "[{label}] focus node {:?} missing from AccessKit tree",
        update.focus
    );
    for (id, node) in &update.nodes {
        for &cid in node.children() {
            assert!(
                nodes_by_id.contains_key(&cid),
                "[{label}] parent {id:?} references missing child {cid:?}"
            );
        }
    }

    // Verify all visible live controls are represented in the AccessKit update with a label or value.
    let live: HashSet<NodeId> = live_nodes(sim).into_iter().collect();
    sim.runtime.with_inner(|inner| {
        for id in live {
            let Some(node) = inner.nodes.get(id) else {
                continue;
            };
            if node.accessibility.hidden {
                continue;
            }
            if let NodeKind::Control(c) = &node.kind {
                let ak = nodes_by_id.get(&to_ak_id(id)).unwrap_or_else(|| {
                    panic!("[{label}] control {} missing from AccessKit", c.kind)
                });
                assert!(
                    ak.label().is_some_and(|s| !s.is_empty())
                        || matches!(
                            ak.role(),
                            Role::Slider | Role::List | Role::Table | Role::Group
                        ),
                    "[{label}] interactive control {:?} missing non-empty AccessKit label",
                    c.kind
                );
            }
        }
    });
}

fn mean_luminance(rgba: &[u8]) -> f32 {
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for px in rgba.as_chunks::<4>().0 {
        let r = f64::from(px[0]) / 255.0;
        let g = f64::from(px[1]) / 255.0;
        let b = f64::from(px[2]) / 255.0;
        sum += 0.2126 * r + 0.7152 * g + 0.0722 * b;
        count += 1;
    }
    (sum / count.max(1) as f64) as f32
}

fn find_live_controls(sim: &HeadlessSimulator, kind: &str, label_needle: &str) -> Vec<NodeId> {
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
            if matches!(&node.kind, NodeKind::Control(c) if c.kind == kind)
                && (label_needle.is_empty()
                    || node
                        .accessibility
                        .label
                        .as_deref()
                        .is_some_and(|l| l.contains(label_needle)))
            {
                out.push(id);
            }
            for &c in node.children.iter().rev() {
                stack.push(c);
            }
        }
    });
    out
}

type EditableTextState = (Option<usize>, Option<(usize, usize)>, String);

fn editable_caret_and_sel(
    sim: &HeadlessSimulator,
    control_id: NodeId,
) -> Option<EditableTextState> {
    sim.runtime.with_inner(|inner| {
        let mut stack = vec![control_id];
        while let Some(id) = stack.pop() {
            let Some(node) = inner.nodes.get(id) else {
                continue;
            };
            if let NodeKind::Text(t) = &node.kind {
                return Some((t.caret, t.selection, t.text.clone()));
            }
            for &c in node.children.iter().rev() {
                stack.push(c);
            }
        }
        None
    })
}

fn has_focus_halo_near(sim: &mut HeadlessSimulator, bounds: Rect) -> bool {
    let dl = sim.build_display_list();
    dl.commands.iter().any(|cmd| {
        matches!(
            cmd,
            DisplayCommand::StrokeRoundedRect { rect, width, .. }
                if *width >= 2.0
                    && rect.x <= bounds.x + 1.0
                    && rect.max_x() >= bounds.max_x() - 1.0
                    && (rect.x - bounds.x).abs() <= 6.0
                    && (rect.y - bounds.y).abs() <= 6.0
        )
    })
}

#[test]
fn e2e_counter_example() {
    struct Counter;
    impl View for Counter {
        fn body(self, cx: &mut Cx) -> impl View {
            let count = cx.state(0);
            VStack::new(
                16,
                (
                    Text::new(move || format!("Count: {}", count.get())).font(Font::large_title()),
                    HStack::new(
                        10,
                        (
                            Button::new("-", move || count.update(|c| *c -= 1)),
                            Button::new("+", move || count.update(|c| *c += 1)),
                        ),
                    ),
                ),
            )
            .padding(24)
            .background(Color::surface_elevated())
            .border(1, Color::border())
            .corner_radius(12)
            .shadow(Color::rgba(0.0, 0.0, 0.0, 0.08), 10, Point::new(0.0, 3.0))
            .frame(
                Frame::new()
                    .max_width(f32::INFINITY)
                    .max_height(f32::INFINITY),
            )
            .background(Color::background())
        }
    }

    let mut sim = HeadlessSimulator::new(Counter, Size::new(400.0, 260.0));
    assert_layout_invariants(&sim, "counter initial");
    assert_accesskit_valid(&sim, "counter initial");
    assert!(find_live_text(&sim, "Count: 0").is_some());

    let rgba_initial = sim.render_rgba();
    assert_eq!(rgba_initial.len(), 400 * 260 * 4);

    let plus = find_live_control(&sim, "button", "+").expect("+ button");
    let minus = find_live_control(&sim, "button", "-").expect("- button");
    let plus_b = sim.node_bounds(plus).expect("+ bounds");
    assert_eq!(
        HitTester::cursor_kind_at(
            &sim.runtime,
            sim.root,
            Point::new(plus_b.mid_x(), plus_b.mid_y())
        ),
        "pointer"
    );
    sim.hover(plus_b.mid_x(), plus_b.mid_y());
    let plus_hovered = sim.runtime.with_inner(|inner| {
        matches!(
            inner.nodes.get(plus).map(|n| &n.kind),
            Some(NodeKind::Control(c)) if c.is_hovered
        )
    });
    assert!(plus_hovered, "hovering + button should set is_hovered");

    sim.click_node(plus);
    sim.click_node(plus);
    assert!(find_live_text(&sim, "Count: 2").is_some());

    sim.click_node(minus);
    sim.click_node(minus);
    sim.click_node(minus);
    assert!(find_live_text(&sim, "Count: -1").is_some());

    let rgba_updated = sim.render_rgba();
    assert_ne!(rgba_initial, rgba_updated);
    assert_layout_invariants(&sim, "counter updated");

    // Keyboard traversal & activation + focus ring + AccessKit Click action.
    let f_plus = sim.focus_next().expect("focus wraps to + button");
    assert_eq!(f_plus, plus);
    let plus_focus_b = sim.node_bounds(plus).unwrap();
    assert!(
        has_focus_halo_near(&mut sim, plus_focus_b),
        "focused + button should emit focus halo ring"
    );
    sim.press_key(KeyCode::Enter, Modifiers::NONE);
    assert!(find_live_text(&sim, "Count: 0").is_some());

    let f_minus = sim.focus_next().expect("focus - button");
    assert_eq!(f_minus, minus);
    sim.press_key(KeyCode::Space, Modifiers::NONE);
    assert!(find_live_text(&sim, "Count: -1").is_some());

    assert!(sim.dispatch_accessibility_action(ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: to_ak_id(plus),
        data: None,
    }));
    assert!(find_live_text(&sim, "Count: 0").is_some());
}

#[test]
fn e2e_todo_example() {
    #[derive(Clone, Debug)]
    struct TodoItem {
        id: u64,
        title: String,
        priority: &'static str,
        points: u32,
        done: bool,
    }
    struct TodoApp;
    impl View for TodoApp {
        fn body(self, cx: &mut Cx) -> impl View {
            let draft = cx.state(String::new());
            let priority = cx.state(0usize);
            let effort = cx.state(2.0f32);
            let validation_msg = cx.state(String::new());
            let next_id = cx.state(4u64);
            let filter = cx.state(0usize);
            let dark = cx.state(false);
            let items = cx.state(vec![
                TodoItem {
                    id: 1,
                    title: "Build reactive signal arena".into(),
                    priority: "P0",
                    points: 3,
                    done: true,
                },
                TodoItem {
                    id: 2,
                    title: "Implement proposal/response layout".into(),
                    priority: "P1",
                    points: 2,
                    done: true,
                },
                TodoItem {
                    id: 3,
                    title: "Ship 60fps virtualized LazyVStack".into(),
                    priority: "P1",
                    points: 5,
                    done: false,
                },
            ]);
            let remaining = cx.memo(move || items.get().iter().filter(|t| !t.done).count());
            let add_item = move || {
                let text = draft.get_untracked().trim().to_owned();
                if text.is_empty() {
                    validation_msg.set("Enter a task description before adding.".into());
                    return;
                }
                validation_msg.set(String::new());
                let prio_label = match priority.get_untracked() {
                    1 => "P0",
                    2 => "P2",
                    _ => "P1",
                };
                let pts = effort.get_untracked().round().clamp(1.0, 5.0) as u32;
                let id = next_id.get_untracked();
                next_id.set(id + 1);
                items.update(|list| {
                    list.push(TodoItem {
                        id,
                        title: text,
                        priority: prio_label,
                        points: pts,
                        done: false,
                    });
                });
                draft.set(String::new());
            };

            VStack::new(
                12,
                (
                    HStack::new(
                        12,
                        (
                            Text::new("RustUI Todos").font(Font::title()),
                            Spacer::new(),
                            Toggle::new("Dark Mode", dark.binding()),
                        ),
                    ),
                    VStack::new(
                        8,
                        (
                            HStack::new(
                                8,
                                (
                                    TextField::new("Add a todo...", draft.binding())
                                        .on_submit(move |_| add_item()),
                                    Button::new("Add", add_item),
                                ),
                            ),
                            HStack::new(
                                10,
                                (
                                    Picker::new(
                                        priority.binding(),
                                        vec![
                                            "P1 · Normal".into(),
                                            "P0 · Urgent".into(),
                                            "P2 · Low".into(),
                                        ],
                                    ),
                                    Text::new("Effort:")
                                        .font(Font::caption())
                                        .color(Color::secondary()),
                                    Slider::new(effort.binding(), 1.0, 5.0).step(1.0),
                                    Text::new(move || {
                                        format!("{} pts", effort.get().round() as u32)
                                    })
                                    .font(Font::caption())
                                    .color(Color::secondary()),
                                ),
                            ),
                            If::new(
                                move || !validation_msg.get().is_empty(),
                                move || {
                                    Text::new(move || validation_msg.get())
                                        .font(Font::caption())
                                        .color(Color::destructive())
                                        .transition(Transition::Opacity)
                                },
                            ),
                        ),
                    )
                    .padding(10)
                    .background(Color::surface())
                    .border(1, Color::border())
                    .corner_radius(10),
                    HStack::new(
                        12,
                        (
                            Picker::new(
                                filter.binding(),
                                vec!["All".into(), "Active".into(), "Completed".into()],
                            ),
                            Spacer::new(),
                            Text::new(move || format!("{} remaining", remaining.get()))
                                .font(Font::caption())
                                .color(Color::secondary()),
                        ),
                    ),
                    Divider::new(),
                    ScrollView::vertical(VStack::new(
                        6,
                        ForEach::new(
                            move || {
                                let f = filter.get();
                                items
                                    .get()
                                    .into_iter()
                                    .filter(|item| match f {
                                        1 => !item.done,
                                        2 => item.done,
                                        _ => true,
                                    })
                                    .collect()
                            },
                            |item| item.id,
                            move |item| {
                                let id = item.id;
                                let checked_b = Binding::new(
                                    move || {
                                        items
                                            .get()
                                            .iter()
                                            .find(|t| t.id == id)
                                            .is_some_and(|t| t.done)
                                    },
                                    move |done| {
                                        items.update(|list| {
                                            if let Some(t) = list.iter_mut().find(|t| t.id == id) {
                                                t.done = done;
                                            }
                                        });
                                    },
                                );
                                HStack::new(
                                    8,
                                    (
                                        Checkbox::new(item.title, checked_b),
                                        Text::new(format!("{} · {}pt", item.priority, item.points))
                                            .font(Font::caption())
                                            .color(Color::secondary()),
                                        Spacer::new(),
                                        Button::new("Delete", move || {
                                            items.update(|list| list.retain(|t| t.id != id));
                                        }),
                                    ),
                                )
                                .padding(Edges::symmetric(6.0, 10.0))
                                .background(Color::surface_elevated())
                                .border(1, Color::border())
                                .corner_radius(8)
                            },
                        ),
                    ))
                    .frame(
                        Frame::new()
                            .max_width(f32::INFINITY)
                            .max_height(f32::INFINITY),
                    ),
                ),
            )
            .alignment(HorizontalAlignment::Leading)
            .padding(16)
            .frame(
                Frame::new()
                    .max_width(f32::INFINITY)
                    .max_height(f32::INFINITY)
                    .alignment(Alignment::TOP_LEADING),
            )
            .background(Color::background())
            .environment_reactive(move || {
                if dark.get() {
                    Theme::dark().to_palette()
                } else {
                    Theme::light().to_palette()
                }
            })
        }
    }

    let mut sim = HeadlessSimulator::new(TodoApp, Size::new(640.0, 580.0));
    assert_layout_invariants(&sim, "todo initial");
    assert_accesskit_valid(&sim, "todo initial");
    assert!(find_live_text(&sim, "1 remaining").is_some());

    // Empty submit triggers validation banner, cleared once valid item is submitted.
    let add_btn = find_live_control(&sim, "button", "Add").expect("Add button");
    sim.click_node(add_btn);
    assert!(find_live_text(&sim, "Enter a task description before adding.").is_some());

    // Type new todo in TextField and submit with Enter.
    let field = find_live_control(&sim, "textfield", "Add a todo...").expect("todo textfield");
    sim.click_node(field);
    sim.type_text("Write E2E tests");
    sim.press_key(KeyCode::Enter, Modifiers::NONE);
    assert!(find_live_control(&sim, "checkbox", "Write E2E tests").is_some());
    assert!(find_live_text(&sim, "Enter a task description before adding.").is_none());
    assert!(find_live_text(&sim, "2 remaining").is_some());

    // Switch priority Picker to P0 · Urgent and bump effort Slider to 5 pts, then click Add.
    let pickers = find_live_controls(&sim, "picker", "");
    assert_eq!(pickers.len(), 2, "expected priority and filter pickers");
    sim.click_node(pickers[0]);
    assert!(find_live_text(&sim, "P0 · Urgent").is_some());
    let effort_slider = find_live_control(&sim, "slider", "").expect("effort slider");
    let es_b = sim
        .node_bounds(effort_slider)
        .expect("effort slider bounds");
    sim.drag(
        Point::new(es_b.mid_x(), es_b.mid_y()),
        Point::new(es_b.max_x() - 1.0, es_b.mid_y()),
    );
    assert!(find_live_text(&sim, "5 pts").is_some());

    sim.click_node(field);
    sim.type_text("Verify Dark Mode");
    sim.click_node(add_btn);
    assert!(find_live_control(&sim, "checkbox", "Verify Dark Mode").is_some());
    assert!(find_live_text(&sim, "P0 · 5pt").is_some());
    assert!(find_live_text(&sim, "3 remaining").is_some());

    // Toggle Checkbox for "Verify Dark Mode" and check AccessKit state + remaining count.
    let verify_cb = find_live_control(&sim, "checkbox", "Verify Dark Mode").expect("verify cb");
    sim.click_node(verify_cb);
    assert!(find_live_text(&sim, "2 remaining").is_some());
    let ak_tree = sim.accessibility_tree();
    let cb_ak = ak_tree
        .nodes
        .iter()
        .find(|(id, _)| *id == to_ak_id(verify_cb))
        .map(|(_, n)| n)
        .expect("checkbox in AK");
    assert_eq!(cb_ak.value(), Some("true"));

    // Filter with second Picker -> Completed (click cycles All -> Active -> Completed).
    let filter_picker = pickers[1];
    sim.click_node(filter_picker); // Active
    assert!(find_live_control(&sim, "checkbox", "Write E2E tests").is_some());
    assert!(find_live_control(&sim, "checkbox", "Verify Dark Mode").is_none());
    sim.click_node(filter_picker); // Completed
    assert!(find_live_control(&sim, "checkbox", "Write E2E tests").is_none());
    assert!(find_live_control(&sim, "checkbox", "Verify Dark Mode").is_some());
    sim.click_node(filter_picker); // All
    assert_layout_invariants(&sim, "todo filtered all");

    // Dark Mode Toggle -> verify Switch value and actual dark luminance shift.
    let light_lum = mean_luminance(&sim.render_rgba());
    assert!(
        light_lum > 0.7,
        "expected bright light mode luminance, got {light_lum}"
    );

    let dark_toggle = find_live_control(&sim, "toggle", "Dark Mode").expect("Dark Mode toggle");
    assert!(sim.dispatch_accessibility_action(ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: to_ak_id(dark_toggle),
        data: None,
    }));
    let dark_lum = mean_luminance(&sim.render_rgba());
    assert!(
        dark_lum < 0.35,
        "expected dark mode luminance < 0.35, got {dark_lum}"
    );
    assert_layout_invariants(&sim, "todo dark mode");
    assert_accesskit_valid(&sim, "todo dark mode");
}

#[test]
fn e2e_text_editor_example() {
    let _clip_guard = CLIPBOARD_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    struct TextEditorApp;
    impl View for TextEditorApp {
        fn body(self, cx: &mut Cx) -> impl View {
            let content = cx.state(
                "# RustUI Notes\n\nWrite Unicode, 日本語, 中文, and emoji 🚀 here.\n\
                 Supports rope-backed undo/redo, grapheme cursor navigation, and IME composition."
                    .to_owned(),
            );
            let split_ratio = cx.state(0.28f32);
            let selected_doc = cx.state("README.md".to_owned());
            let expanded = cx.state(true);

            let stats = cx.memo(move || {
                let txt = content.get();
                let lines = txt.lines().count().max(1);
                let words = txt.split_whitespace().count();
                let chars = txt.chars().count();
                format!("{lines} lines · {words} words · {chars} chars")
            });

            let make_tree = move |is_expanded: bool| {
                Tree::new(
                    vec![TreeItemData {
                        id: "root".into(),
                        label: "docs".into(),
                        expanded: is_expanded,
                        children: vec![
                            TreeItemData {
                                id: "readme".into(),
                                label: "README.md".into(),
                                expanded: false,
                                children: vec![],
                            },
                            TreeItemData {
                                id: "guide".into(),
                                label: "GUIDE.md".into(),
                                expanded: false,
                                children: vec![],
                            },
                        ],
                    }],
                    move |id| match id {
                        "root" => expanded.update(|e| *e = !*e),
                        "readme" => {
                            selected_doc.set("README.md".into());
                            content.set(
                                "# RustUI Notes\n\nWrite Unicode, 日本語, 中文, and emoji 🚀 here.\n\
                                 Supports rope-backed undo/redo, grapheme cursor navigation, and IME composition."
                                    .into(),
                            );
                        }
                        "guide" => {
                            selected_doc.set("GUIDE.md".into());
                            content.set(
                                "# RustUI Developer Guide\n\n- State<T> hooks with cx.state(init)\n\
                                 - Proposal/response Stack, Grid, and SplitView layouts\n\
                                 - AccessKit screen reader bridge"
                                    .into(),
                            );
                        }
                        _ => {}
                    },
                )
            };

            VStack::new(
                0,
                (
                    Toolbar::new((
                        Text::new(move || format!("Editing: {}", selected_doc.get()))
                            .font(Font::headline()),
                        Spacer::new(),
                        Button::new("Sample CJK/Emoji", move || {
                            content.set("Hello RustUI!\n日本語テスト\n中文输入测试 🎨".into());
                        }),
                        Button::new("Clear", move || content.set(String::new())),
                    )),
                    SplitView::horizontal(
                        split_ratio.binding(),
                        VStack::new(
                            8,
                            (
                                Text::new("Workspace").font(Font::headline()),
                                If::new(move || expanded.get(), move || make_tree(true))
                                    .else_view(move || make_tree(false)),
                            ),
                        )
                        .alignment(HorizontalAlignment::Leading)
                        .padding(12)
                        .frame(
                            Frame::new()
                                .max_width(f32::INFINITY)
                                .max_height(f32::INFINITY)
                                .alignment(Alignment::TOP_LEADING),
                        )
                        .background(Color::surface()),
                        VStack::new(
                            8,
                            (
                                TextEditor::new(content.binding()),
                                HStack::new(
                                    8,
                                    (
                                        Text::new(move || stats.get())
                                            .font(Font::caption())
                                            .color(Color::secondary()),
                                        Spacer::new(),
                                        Text::new("IME & Rope Ready")
                                            .font(Font::caption())
                                            .color(Color::accent()),
                                    ),
                                ),
                            ),
                        )
                        .alignment(HorizontalAlignment::Leading)
                        .padding(12)
                        .frame(
                            Frame::new()
                                .max_width(f32::INFINITY)
                                .max_height(f32::INFINITY)
                                .alignment(Alignment::TOP_LEADING),
                        ),
                    ),
                ),
            )
            .frame(
                Frame::new()
                    .max_width(f32::INFINITY)
                    .max_height(f32::INFINITY)
                    .alignment(Alignment::TOP_LEADING),
            )
        }
    }

    let mut sim = HeadlessSimulator::new(TextEditorApp, Size::new(860.0, 580.0));
    assert_layout_invariants(&sim, "text_editor initial");
    assert_accesskit_valid(&sim, "text_editor initial");
    assert!(find_live_text(&sim, "Editing: README.md").is_some());
    assert!(find_live_text(&sim, "138 chars").is_some());

    // Drag SplitView splitter rightward and verify cursor kind + sidebar grows.
    let split = find_live_node(&sim, |n| {
        matches!(n.kind, NodeKind::Container(ContainerKind::CustomLayout)) && n.children.len() == 3
    })
    .expect("SplitView");
    let splitter_id = sim
        .runtime
        .with_inner(|inner| inner.nodes.get(split).unwrap().children[1]);
    let sb = sim.node_bounds(splitter_id).expect("splitter bounds");
    assert!(
        sb.x > 200.0 && sb.x < 280.0,
        "unexpected initial split position {sb:?}"
    );
    assert_eq!(
        HitTester::cursor_kind_at(&sim.runtime, sim.root, Point::new(sb.mid_x(), sb.mid_y())),
        "ew-resize"
    );
    sim.drag(
        Point::new(sb.mid_x(), sb.mid_y()),
        Point::new(sb.mid_x() + 90.0, sb.mid_y()),
    );
    let sb_after = sim
        .node_bounds(splitter_id)
        .expect("splitter bounds after drag");
    assert!(
        sb_after.x > sb.x + 50.0,
        "splitter should move right: before={sb:?}, after={sb_after:?}"
    );

    // Tree collapse/expand + document switching.
    let root_tree_item = find_live_text(&sim, "docs").expect("docs tree item");
    sim.click_node(root_tree_item);
    assert!(
        find_live_text(&sim, "docs").is_some(),
        "root tree item must remain visible when collapsed"
    );
    assert!(find_live_text(&sim, "GUIDE.md").is_none());
    let root_tree_item = find_live_text(&sim, "docs").expect("docs collapsed");
    sim.click_node(root_tree_item);
    let guide_item = find_live_text(&sim, "GUIDE.md").expect("GUIDE.md expanded");
    sim.click_node(guide_item);
    assert!(find_live_text(&sim, "Editing: GUIDE.md").is_some());

    // TextEditor I-beam cursor positioning via click + arrow/Home/End navigation.
    let editor = find_live_control(&sim, "texteditor", "").expect("text_editor control");
    let ed_b = sim.node_bounds(editor).expect("editor bounds");
    assert_eq!(
        HitTester::cursor_kind_at(
            &sim.runtime,
            sim.root,
            Point::new(ed_b.x + 24.0, ed_b.y + 16.0)
        ),
        "text"
    );
    assert!(
        editable_caret_and_sel(&sim, editor).is_some(),
        "TextEditor should contain a text child"
    );

    sim.click(ed_b.x + 12.0, ed_b.y + 14.0);
    let (caret_start, _, _) = editable_caret_and_sel(&sim, editor).unwrap();
    let caret_start = caret_start.expect("caret should be present when focused");
    assert!(
        caret_start <= 2,
        "expected caret near line 1 start, got {caret_start}"
    );

    sim.click(ed_b.x + 115.0, ed_b.y + 14.0);
    let (caret_mid, _, text_before_mid_insert) = editable_caret_and_sel(&sim, editor).unwrap();
    let caret_mid = caret_mid.expect("caret at middle click");
    assert!(
        caret_mid > caret_start + 3 && caret_mid < 24,
        "expected middle-of-line caret, got {caret_mid} (start={caret_start})"
    );
    sim.type_text("*");
    let (caret_after_type, _, text_after_mid_insert) =
        editable_caret_and_sel(&sim, editor).unwrap();
    assert_eq!(
        text_after_mid_insert.len(),
        text_before_mid_insert.len() + 1
    );
    assert_ne!(
        text_after_mid_insert.chars().last(),
        Some('*'),
        "typing after middle click must insert inline rather than at EOF"
    );
    assert_eq!(caret_after_type, Some(caret_mid + 1));

    // Undo inline insert via Ctrl+Z, then click down on line 3.
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::NONE
    };
    sim.press_key(KeyCode::Character("z".into()), ctrl);
    sim.click(ed_b.x + 80.0, ed_b.y + 54.0);
    let (caret_l3, _, _) = editable_caret_and_sel(&sim, editor).unwrap();
    let caret_l3 = caret_l3.expect("caret on line 3");
    assert!(
        caret_l3 > 24,
        "expected line 3 byte offset > 24, got {caret_l3}"
    );

    sim.press_key(KeyCode::ArrowLeft, Modifiers::NONE);
    let (c_left, _, _) = editable_caret_and_sel(&sim, editor).unwrap();
    assert!(c_left.unwrap() < caret_l3, "ArrowLeft must decrease caret");
    sim.press_key(KeyCode::ArrowRight, Modifiers::NONE);
    let (c_right, _, _) = editable_caret_and_sel(&sim, editor).unwrap();
    assert_eq!(c_right, Some(caret_l3), "ArrowRight must restore caret");
    sim.press_key(KeyCode::Home, Modifiers::NONE);
    let (c_home, _, _) = editable_caret_and_sel(&sim, editor).unwrap();
    let c_home = c_home.unwrap();
    assert!(c_home < caret_l3, "Home must jump to start of line 3");
    sim.press_key(KeyCode::End, Modifiers::NONE);
    let (c_end, _, _) = editable_caret_and_sel(&sim, editor).unwrap();
    assert!(c_end.unwrap() > c_home, "End must jump to end of line 3");
    sim.press_key(KeyCode::ArrowUp, Modifiers::NONE);
    let (c_up, _, _) = editable_caret_and_sel(&sim, editor).unwrap();
    assert!(
        c_up.unwrap() < c_home,
        "ArrowUp must move to an earlier line"
    );
    sim.press_key(KeyCode::ArrowDown, Modifiers::NONE);
    let (c_down, _, _) = editable_caret_and_sel(&sim, editor).unwrap();
    assert!(c_down.unwrap() >= c_home, "ArrowDown must return to line 3");

    // Drag selection + Ctrl+C / Ctrl+V / Ctrl+Z.
    sim.drag(
        Point::new(ed_b.x + 10.0, ed_b.y + 14.0),
        Point::new(ed_b.x + 100.0, ed_b.y + 14.0),
    );
    let (_, drag_sel, before_paste) = editable_caret_and_sel(&sim, editor).unwrap();
    assert!(
        drag_sel.is_some_and(|(s, e)| e > s),
        "dragging in TextEditor must select text, got {drag_sel:?}"
    );
    sim.press_key(KeyCode::Character("c".into()), ctrl);
    sim.click(ed_b.x + 10.0, ed_b.y + 14.0);
    sim.press_key(KeyCode::Home, Modifiers::NONE);
    sim.press_key(KeyCode::Character("v".into()), ctrl);
    let (_, _, after_paste) = editable_caret_and_sel(&sim, editor).unwrap();
    assert!(
        after_paste.len() > before_paste.len(),
        "Ctrl+V must paste copied text"
    );
    sim.press_key(KeyCode::Character("z".into()), ctrl);
    let (_, _, after_undo) = editable_caret_and_sel(&sim, editor).unwrap();
    assert_eq!(after_undo, before_paste, "Ctrl+Z must undo pasted text");

    // Right-click Context Menu (Select All, Copy, Paste, Undo, Clear).
    sim.right_click(ed_b.x + 60.0, ed_b.y + 18.0);
    assert_layout_invariants(&sim, "text_editor context menu");
    let select_all_btn =
        find_live_control(&sim, "button", "Select All").expect("Context Select All");
    assert!(find_live_control(&sim, "button", "Copy").is_some());
    assert!(find_live_control(&sim, "button", "Paste").is_some());
    sim.click_node(select_all_btn);
    let (_, all_sel, full_text) = editable_caret_and_sel(&sim, editor).unwrap();
    assert_eq!(all_sel, Some((0, full_text.len())));

    sim.right_click(ed_b.x + 60.0, ed_b.y + 18.0);
    let copy_btn = find_live_control(&sim, "button", "Copy").expect("Context Copy");
    sim.click_node(copy_btn);
    sim.click(ed_b.x + 10.0, ed_b.y + 14.0);
    sim.right_click(ed_b.x + 60.0, ed_b.y + 18.0);
    let paste_btn = find_live_control(&sim, "button", "Paste").expect("Context Paste");
    sim.click_node(paste_btn);
    let (_, _, doubled) = editable_caret_and_sel(&sim, editor).unwrap();
    assert_eq!(doubled.len(), full_text.len() * 2);

    sim.right_click(ed_b.x + 60.0, ed_b.y + 18.0);
    let undo_btn = find_live_control(&sim, "button", "Undo").expect("Context Undo");
    sim.click_node(undo_btn);
    let (_, _, reverted) = editable_caret_and_sel(&sim, editor).unwrap();
    assert_eq!(reverted, full_text);

    // IME composition + Sample CJK/Emoji + Clear buttons.
    sim.click_node(editor);
    sim.ime_preedit("nihongo", Some((0, 7)));
    sim.ime_commit("日本語追記");
    let tree = sim.accessibility_tree();
    let ed_val = tree
        .nodes
        .iter()
        .find(|(id, _)| *id == to_ak_id(editor))
        .and_then(|(_, n)| n.value())
        .unwrap_or("");
    assert!(
        ed_val.contains("日本語追記"),
        "expected IME commit in editor, got {ed_val:?}"
    );

    let sample_btn = find_live_control(&sim, "button", "Sample CJK/Emoji").expect("sample button");
    sim.click_node(sample_btn);
    assert!(find_live_text(&sim, "3 lines").is_some());

    let clear_btn = find_live_control(&sim, "button", "Clear").expect("clear button");
    sim.click_node(clear_btn);
    assert!(find_live_text(&sim, "0 chars").is_some());
    assert_layout_invariants(&sim, "text_editor cleared");
}

#[test]
fn e2e_dashboard_example() {
    const SAMPLE_SVG_ICON: &str = r##"
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">
  <defs>
    <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#2563eb"/>
      <stop offset="100%" stop-color="#7c3aed"/>
    </linearGradient>
  </defs>
  <rect x="4" y="4" width="56" height="56" rx="14" fill="url(#bg)"/>
  <circle cx="24" cy="26" r="9" fill="#38bdf8"/>
  <path d="M16 44 L29 29 L38 38 L48 22" stroke="#ffffff" stroke-width="4" fill="none" stroke-linecap="round" stroke-linejoin="round"/>
</svg>
"##;

    fn make_procedural_rgba(size: u32) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let fx = x as f32 / size as f32;
                let fy = y as f32 / size as f32;
                let checker = ((x / 8) + (y / 8)).is_multiple_of(2);
                let r = if checker {
                    (fx * 200.0 + 40.0) as u8
                } else {
                    30
                };
                let g = (fy * 190.0 + 50.0) as u8;
                let b = ((1.0 - fx) * 180.0 + 75.0) as u8;
                pixels.extend_from_slice(&[r, g, b, 255]);
            }
        }
        pixels
    }

    struct DashboardApp;
    impl View for DashboardApp {
        fn body(self, cx: &mut Cx) -> impl View {
            let tab = cx.state(0usize);
            let nav = cx.state(0usize);
            let gain = cx.state(0.72f32);
            let selected_row = cx.state(None::<usize>);
            let show_sheet = cx.state(false);
            let show_popover = cx.state(false);

            let pulse_phase = cx.state(0usize);
            let card_accent = cx.state(false);
            let zoom_scale = cx.state(1.0f32);
            let reduced_motion_flag = cx.state(cx.runtime().is_reduced_motion());
            let rt_rm = cx.runtime().clone();
            let reduced_motion_binding = Binding::new(
                move || reduced_motion_flag.get(),
                move |enabled| {
                    rt_rm.set_reduced_motion(enabled);
                    reduced_motion_flag.set(enabled);
                },
            );

            let form_name = cx.state(String::new());
            let form_key = cx.state(String::new());
            let form_env = cx.state(0usize);
            let form_workers = cx.state(4.0f32);
            let form_autodeploy = cx.state(true);
            let form_terms = cx.state(false);
            let form_submitted = cx.state(String::new());
            let proc_rgba = make_procedural_rgba(48);

            Sheet::new(
                show_sheet.binding(),
                VStack::new(
                    0,
                    (
                        Toolbar::new((
                            Text::new("RustUI Telemetry Dashboard").font(Font::headline()),
                            Spacer::new(),
                            Popover::new(
                                show_popover.binding(),
                                Button::new("Info", move || show_popover.update(|p| *p = !*p)),
                                || Text::new("60 fps GPU-rendered RustUI showcase"),
                            ),
                            Menu::new(
                                "Window",
                                vec![MenuItem::new("Open Report Sheet", move || {
                                    show_sheet.set(true);
                                })],
                            ),
                        )),
                        TabView::new(
                            tab.binding(),
                            vec![
                                TabItem::new("Overview", move || {
                                    NavigationStack::new(
                                        nav.binding(),
                                        vec![
                                            NavigationPage::new("Metrics", move || {
                                                VStack::new(
                                                    12,
                                                    (
                                                        HStack::new(
                                                            12,
                                                            (
                                                                Text::new("Amplitude:"),
                                                                Slider::new(
                                                                    gain.binding(),
                                                                    0.1,
                                                                    1.0,
                                                                ),
                                                                Text::new(move || {
                                                                    format!(
                                                                        "{:.0}%",
                                                                        gain.get() * 100.0
                                                                    )
                                                                })
                                                                .font(Font::caption())
                                                                .color(Color::secondary()),
                                                                Spacer::new(),
                                                                Button::new(
                                                                    "Details ›",
                                                                    move || {
                                                                        nav.set(1);
                                                                    },
                                                                ),
                                                            ),
                                                        ),
                                                        Canvas::new(move |rect, dl| {
                                                            dl.fill_rounded_rect(
                                                                rect,
                                                                CornerRadii::all(8.0),
                                                                Color::from_hex(0x0f172a),
                                                            );
                                                            let g = gain.get();
                                                            for i in 0..12usize {
                                                                let h = (rect.height - 28.0)
                                                                    * (0.4 * g);
                                                                dl.fill_rounded_rect(
                                                                    Rect::new(
                                                                        rect.x
                                                                            + 16.0
                                                                            + i as f32 * 40.0,
                                                                        rect.max_y() - 12.0 - h,
                                                                        24.0,
                                                                        h,
                                                                    ),
                                                                    CornerRadii::all(3.0),
                                                                    Color::from_hex(0x3b82f6),
                                                                );
                                                            }
                                                        })
                                                        .frame(Frame::new().height(220)),
                                                    ),
                                                )
                                                .padding(16)
                                            }),
                                            NavigationPage::new("Node Table", move || {
                                                Table::new(
                                                    vec![
                                                        TableColumn::fixed("ID", 70.0),
                                                        TableColumn::flexible(
                                                            "Subsystem",
                                                            140.0,
                                                            260.0,
                                                        ),
                                                        TableColumn::fixed("Budget", 100.0),
                                                    ],
                                                    20,
                                                    selected_row.binding(),
                                                    |r, c| match c {
                                                        0 => format!("#{r}"),
                                                        1 => format!("Layer Node {r}"),
                                                        _ => "0.4 ms".into(),
                                                    },
                                                )
                                                .padding(12)
                                            }),
                                        ],
                                    )
                                }),
                                TabItem::new("1M Virtual List", || {
                                    LazyVStack::new(1_000_000, 26, |idx| {
                                        Text::new(format!("Virtual Row #{idx}"))
                                    })
                                    .with_pool_updater(|idx, node| {
                                        format_virtual_row_text(node, "Virtual Row #", idx);
                                    })
                                    .padding(12)
                                }),
                                TabItem::new("Graphics & Shapes", move || {
                                    let rgba_copy = proc_rgba.clone();
                                    VStack::new(
                                        14,
                                        (
                                            HStack::new(
                                                10,
                                                (
                                                    Button::new("Animate Shapes", move || {
                                                        with_animation(Animation::spring(), || {
                                                            card_accent.update(|a| *a = !*a);
                                                            pulse_phase.update(|p| *p = (*p + 1) % 6);
                                                        });
                                                    }),
                                                    Toggle::new(
                                                        "Reduced Motion",
                                                        reduced_motion_binding.clone(),
                                                    ),
                                                    Spacer::new(),
                                                    Text::new(move || {
                                                        format!("Zoom: {:.0}%", zoom_scale.get() * 100.0)
                                                    })
                                                    .font(Font::caption())
                                                    .color(Color::secondary()),
                                                ),
                                            ),
                                            HStack::new(
                                                16,
                                                (
                                                    Canvas::new(move |rect, dl| {
                                                        let step = pulse_phase.get() as f32;
                                                        let z = zoom_scale.get();
                                                        let accent = if card_accent.get() {
                                                            Color::from_hex(0x8b5cf6)
                                                        } else {
                                                            Color::from_hex(0x2563eb)
                                                        };
                                                        dl.draw_gradient(
                                                            rect,
                                                            CornerRadii::all(12.0),
                                                            GradientKind::Linear {
                                                                start: Point::new(0.0, 0.0),
                                                                end: Point::new(rect.width, rect.height),
                                                                stops: vec![
                                                                    GradientStop {
                                                                        offset: 0.0,
                                                                        color: Color::from_hex(0x0f172a),
                                                                    },
                                                                    GradientStop {
                                                                        offset: 1.0,
                                                                        color: Color::from_hex(0x1e1b4b),
                                                                    },
                                                                ],
                                                            },
                                                        );
                                                        dl.draw_path(
                                                            vec![
                                                                PathOp::MoveTo(Point::new(
                                                                    rect.x + 20.0,
                                                                    rect.max_y() - 36.0,
                                                                )),
                                                                PathOp::CubicTo {
                                                                    ctrl1: Point::new(
                                                                        rect.x + rect.width * 0.3,
                                                                        rect.max_y() - 60.0 - step * 4.0 * z,
                                                                    ),
                                                                    ctrl2: Point::new(
                                                                        rect.x + rect.width * 0.65,
                                                                        rect.max_y() - 16.0,
                                                                    ),
                                                                    to: Point::new(
                                                                        rect.max_x() - 20.0,
                                                                        rect.max_y() - 40.0,
                                                                    ),
                                                                },
                                                            ],
                                                            None,
                                                            Some((3.5, accent)),
                                                        );
                                                    })
                                                    .frame(
                                                        Frame::new()
                                                            .min_width(380.0)
                                                            .max_width(f32::INFINITY)
                                                            .height(220.0),
                                                    )
                                                    .on_pinch(move |ev| {
                                                        zoom_scale.update(|z| {
                                                            *z = (*z + ev.scale_delta).clamp(0.6, 1.8);
                                                        });
                                                    }),
                                                    VStack::new(
                                                        12,
                                                        (
                                                            Image::from_svg(SAMPLE_SVG_ICON)
                                                                .frame(Frame::fixed(56.0, 56.0))
                                                                .corner_radius(10),
                                                            Image::from_rgba(48, 48, rgba_copy)
                                                                .frame(Frame::fixed(56.0, 56.0))
                                                                .corner_radius(10),
                                                        ),
                                                    )
                                                    .padding(12)
                                                    .background(Color::surface())
                                                    .border(1, Color::border())
                                                    .corner_radius(10),
                                                ),
                                            ),
                                            Text::new(move || {
                                                if card_accent.get() {
                                                    "Animated Theme Card: Accent Violet (Spring Transition Active)".to_owned()
                                                } else {
                                                    "Animated Theme Card: Accent Cobalt".to_owned()
                                                }
                                            })
                                            .padding(12)
                                            .background_reactive(move || {
                                                if card_accent.get() {
                                                    Color::from_hex(0x7c3aed).with_alpha(0.22)
                                                } else {
                                                    Color::surface_elevated()
                                                }
                                            })
                                            .border(1, Color::border())
                                            .corner_radius(10)
                                            .animation(Animation::ease_in_out(0.25), card_accent),
                                        ),
                                    )
                                    .alignment(HorizontalAlignment::Leading)
                                    .padding(16)
                                }),
                                TabItem::new("Advanced Form", move || {
                                    VStack::new(
                                        12,
                                        (
                                            HStack::new(
                                                10,
                                                (
                                                    Text::new("Full Name:")
                                                        .frame(Frame::new().width(110.0)),
                                                    TextField::new(
                                                        "Enter full name...",
                                                        form_name.binding(),
                                                    ),
                                                ),
                                            ),
                                            HStack::new(
                                                10,
                                                (
                                                    Text::new("API Token:")
                                                        .frame(Frame::new().width(110.0)),
                                                    SecureField::new(
                                                        "At least 4 characters...",
                                                        form_key.binding(),
                                                    ),
                                                ),
                                            ),
                                            HStack::new(
                                                10,
                                                (
                                                    Text::new("Environment:")
                                                        .frame(Frame::new().width(110.0)),
                                                    Picker::new(
                                                        form_env.binding(),
                                                        vec![
                                                            "Production".into(),
                                                            "Staging".into(),
                                                            "Development".into(),
                                                        ],
                                                    ),
                                                    Spacer::new(),
                                                    Toggle::new(
                                                        "Auto-Deploy",
                                                        form_autodeploy.binding(),
                                                    ),
                                                ),
                                            ),
                                            HStack::new(
                                                10,
                                                (
                                                    Text::new("Concurrency:")
                                                        .frame(Frame::new().width(110.0)),
                                                    Slider::new(form_workers.binding(), 1.0, 16.0)
                                                        .step(1.0),
                                                    Text::new(move || {
                                                        format!(
                                                            "{:.0} workers",
                                                            form_workers.get().round()
                                                        )
                                                    })
                                                    .font(Font::caption())
                                                    .color(Color::secondary()),
                                                ),
                                            ),
                                            Checkbox::new(
                                                "Accept deployment security policy",
                                                form_terms.binding(),
                                            ),
                                            HStack::new(
                                                10,
                                                (
                                                    Button::new("Submit Config", move || {
                                                        let n = form_name.get_untracked();
                                                        let k = form_key.get_untracked();
                                                        let e = match form_env.get_untracked() {
                                                            1 => "Staging",
                                                            2 => "Development",
                                                            _ => "Production",
                                                        };
                                                        let w = form_workers.get_untracked().round()
                                                            as u32;
                                                        let auto = form_autodeploy.get_untracked();
                                                        if n.trim().is_empty()
                                                            || k.len() < 4
                                                            || !form_terms.get_untracked()
                                                        {
                                                            form_submitted.set(
                                                                "Validation error: enter name, 4+ char token, and accept policy."
                                                                    .into(),
                                                            );
                                                        } else {
                                                            form_submitted.set(format!(
                                                                "Saved: {n} · {e} · {w} workers · auto_deploy={auto}"
                                                            ));
                                                        }
                                                    }),
                                                    Button::new("Reset", move || {
                                                        form_name.set(String::new());
                                                        form_key.set(String::new());
                                                        form_env.set(0);
                                                        form_workers.set(4.0);
                                                        form_autodeploy.set(true);
                                                        form_terms.set(false);
                                                        form_submitted.set(String::new());
                                                    }),
                                                    Spacer::new(),
                                                    Text::new(move || {
                                                        let n = form_name.get();
                                                        let k = form_key.get();
                                                        let ok = !n.trim().is_empty()
                                                            && k.len() >= 4
                                                            && form_terms.get();
                                                        let env_name = match form_env.get() {
                                                            1 => "Staging",
                                                            2 => "Development",
                                                            _ => "Production",
                                                        };
                                                        if ok {
                                                            format!("Valid ({env_name})")
                                                        } else {
                                                            "Complete required fields".to_owned()
                                                        }
                                                    })
                                                    .font(Font::caption())
                                                    .color(Color::secondary()),
                                                ),
                                            ),
                                            If::new(
                                                move || !form_submitted.get().is_empty(),
                                                move || {
                                                    Text::new(move || form_submitted.get())
                                                        .padding(10)
                                                        .background(Color::surface())
                                                        .border(1, Color::border())
                                                        .corner_radius(8)
                                                        .transition(Transition::Opacity)
                                                },
                                            ),
                                        ),
                                    )
                                    .alignment(HorizontalAlignment::Leading)
                                    .padding(16)
                                }),
                            ],
                        ),
                    ),
                ),
                move || {
                    VStack::new(
                        12,
                        (
                            Text::new("Telemetry Summary").font(Font::title()),
                            Text::new("All GPU, text, layout, and AccessKit pipelines nominal."),
                            Button::new("Close", move || show_sheet.set(false)),
                        ),
                    )
                },
            )
        }
    }

    let mut sim = HeadlessSimulator::new(DashboardApp, Size::new(960.0, 640.0));
    assert_layout_invariants(&sim, "dashboard initial");
    assert_accesskit_valid(&sim, "dashboard initial");
    assert!(find_live_text(&sim, "72%").is_some());

    // Slider drag + AccessKit SetValue updates percentage badge.
    let slider = find_live_control(&sim, "slider", "").expect("amplitude slider");
    let sb = sim.node_bounds(slider).expect("slider bounds");
    sim.drag(
        Point::new(sb.mid_x(), sb.mid_y()),
        Point::new(sb.max_x() - 2.0, sb.mid_y()),
    );
    assert!(find_live_text(&sim, "72%").is_none());
    assert!(sim.dispatch_accessibility_action(ActionRequest {
        action: Action::SetValue,
        target_tree: TreeId::ROOT,
        target_node: to_ak_id(slider),
        data: Some(ActionData::NumericValue(0.5)),
    }));
    assert!(find_live_text(&sim, "50%").is_some());

    // Popover open -> fits within viewport -> closes on toggle click.
    let info_btn = find_live_control(&sim, "button", "Info").expect("Info button");
    sim.click_node(info_btn);
    let pop_text =
        find_live_text(&sim, "60 fps GPU-rendered RustUI showcase").expect("popover text");
    let pop_b = sim.node_bounds(pop_text).expect("popover bounds");
    assert!(
        pop_b.max_x() <= 960.0 && pop_b.x >= 0.0,
        "popover clipped outside viewport: {pop_b:?}"
    );
    sim.click_node(info_btn);
    assert!(find_live_text(&sim, "60 fps GPU-rendered RustUI showcase").is_none());

    // NavigationStack push to Node Table -> select row -> navigate back.
    let details_btn = find_live_control(&sim, "button", "Details ›").expect("Details button");
    sim.click_node(details_btn);
    assert!(find_live_text(&sim, "Node Table").is_some());
    assert!(find_live_text(&sim, "Layer Node 0").is_some());
    let row2 = find_live_text(&sim, "Layer Node 2").expect("Layer Node 2");
    sim.click_node(row2);
    assert_layout_invariants(&sim, "dashboard table");

    let back_btn = find_live_control(&sim, "button", "Back").expect("Back button");
    sim.click_node(back_btn);
    assert!(find_live_text(&sim, "Metrics").is_some());

    // TabView -> 1M Virtual List -> scroll.
    let vlist_tab = find_live_control(&sim, "button", "1M Virtual List").expect("1M list tab");
    sim.click_node(vlist_tab);
    assert!(find_live_text(&sim, "Virtual Row #0").is_some());
    assert_layout_invariants(&sim, "dashboard virtual list");
    sim.scroll(Point::new(400.0, 350.0), 0.0, -520.0);
    assert!(
        find_live_text(&sim, "Virtual Row #0").is_none(),
        "expected Row #0 recycled offscreen after scroll"
    );
    assert!(
        live_nodes(&sim).len() < 120,
        "1M LazyVStack must stay O(viewport) nodes"
    );

    // TabView -> Graphics & Shapes -> SVG/RGBA images, Canvas pinch zoom, Animate + Reduced Motion.
    let gfx_tab = find_live_control(&sim, "button", "Graphics & Shapes").expect("Graphics tab");
    sim.click_node(gfx_tab);
    assert_layout_invariants(&sim, "dashboard graphics");
    let img_count = sim.runtime.with_inner(|inner| {
        live_nodes(&sim)
            .into_iter()
            .filter(|id| {
                matches!(
                    inner.nodes.get(*id).map(|n| &n.kind),
                    Some(NodeKind::Image(_))
                )
            })
            .count()
    });
    assert!(
        img_count >= 2,
        "expected SVG and RGBA image nodes, found {img_count}"
    );

    let anim_btn = find_live_control(&sim, "button", "Animate Shapes").expect("Animate Shapes btn");
    sim.click_node(anim_btn);
    assert!(find_live_text(&sim, "Animated Theme Card: Accent Violet").is_some());

    let canvas =
        find_live_node(&sim, |n| matches!(n.kind, NodeKind::Canvas(_))).expect("Graphics canvas");
    let cb = sim.node_bounds(canvas).expect("canvas bounds");
    sim.pinch(Point::new(cb.mid_x(), cb.mid_y()), 0.25);
    assert!(find_live_text(&sim, "Zoom: 125%").is_some());

    let rm_toggle =
        find_live_control(&sim, "toggle", "Reduced Motion").expect("Reduced Motion toggle");
    sim.click_node(rm_toggle);
    assert!(sim.runtime.is_reduced_motion());
    sim.click_node(anim_btn);
    assert!(find_live_text(&sim, "Animated Theme Card: Accent Cobalt").is_some());
    assert!(
        sim.runtime
            .with_inner(|inner| inner.scheduler.animations.is_empty()),
        "reduced motion must complete animations immediately without queuing frames"
    );

    // TabView -> Advanced Form -> Validation, SecureField masking, Submit, Reset.
    let form_tab = find_live_control(&sim, "button", "Advanced Form").expect("Advanced Form tab");
    sim.click_node(form_tab);
    assert_layout_invariants(&sim, "dashboard advanced form");
    assert!(find_live_text(&sim, "Complete required fields").is_some());

    let submit_btn = find_live_control(&sim, "button", "Submit Config").expect("Submit Config");
    sim.click_node(submit_btn);
    assert!(find_live_text(&sim, "Validation error:").is_some());

    let name_field = find_live_control(&sim, "textfield", "Enter full name...").expect("Full Name");
    sim.click_node(name_field);
    sim.type_text("Alice");

    let token_field =
        find_live_control(&sim, "securefield", "At least 4 characters...").expect("API Token");
    sim.click_node(token_field);
    sim.type_text("s3cr3t");
    let (_, _, masked) = editable_caret_and_sel(&sim, token_field).unwrap();
    assert_eq!(masked, "••••••", "SecureField must mask typed characters");

    let terms_cb = find_live_control(&sim, "checkbox", "Accept deployment security policy")
        .expect("Policy checkbox");
    sim.click_node(terms_cb);
    assert!(find_live_text(&sim, "Valid (Production)").is_some());

    sim.click_node(submit_btn);
    assert!(
        find_live_text(
            &sim,
            "Saved: Alice · Production · 4 workers · auto_deploy=true"
        )
        .is_some()
    );

    let reset_btn = find_live_control(&sim, "button", "Reset").expect("Reset btn");
    sim.click_node(reset_btn);
    assert!(find_live_text(&sim, "Saved:").is_none());
    assert!(find_live_text(&sim, "Complete required fields").is_some());

    let overview_tab = find_live_control(&sim, "button", "Overview").expect("Overview tab");
    sim.click_node(overview_tab);
    assert!(find_live_text(&sim, "Amplitude:").is_some());

    // Menu -> Open Report Sheet -> modal Sheet blocks background click -> Close button dismisses.
    let menu_btn = find_live_control(&sim, "button", "Window").expect("Window menu");
    sim.click_node(menu_btn);
    let open_sheet =
        find_live_control(&sim, "button", "Open Report Sheet").expect("Open Report Sheet");
    sim.click_node(open_sheet);
    assert!(find_live_text(&sim, "Telemetry Summary").is_some());
    assert_layout_invariants(&sim, "dashboard sheet open");

    let summary_text = find_live_text(&sim, "Telemetry Summary").expect("Telemetry Summary");
    sim.click_node(summary_text);
    assert!(find_live_text(&sim, "Telemetry Summary").is_some());

    let close_btn = find_live_control(&sim, "button", "Close").expect("Close button");
    sim.click_node(close_btn);
    assert!(find_live_text(&sim, "Telemetry Summary").is_none());

    sim.click_node(menu_btn);
    let open_sheet =
        find_live_control(&sim, "button", "Open Report Sheet").expect("Open Report Sheet");
    sim.click_node(open_sheet);
    assert!(find_live_text(&sim, "Telemetry Summary").is_some());

    let info_b = sim.node_bounds(info_btn).expect("info bounds");
    sim.click(info_b.mid_x(), info_b.mid_y());
    assert!(find_live_text(&sim, "60 fps GPU-rendered RustUI showcase").is_none());
    assert!(find_live_text(&sim, "Telemetry Summary").is_none());
}

static CLIPBOARD_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn e2e_terminal_example() {
    let _clip_guard = CLIPBOARD_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum LineTone {
        Prompt,
        Output,
        Success,
        Accent,
        Error,
    }
    impl LineTone {
        fn color(self) -> Color {
            match self {
                Self::Prompt => Color::from_hex(0x38bdf8),
                Self::Output => Color::from_hex(0xe2e8f0),
                Self::Success => Color::from_hex(0x4ade80),
                Self::Accent => Color::from_hex(0xc084fc),
                Self::Error => Color::from_hex(0xf87171),
            }
        }
    }
    #[derive(Clone, Debug)]
    struct TermLine {
        id: u64,
        text: String,
        tone: LineTone,
    }

    struct TerminalApp;
    impl TerminalApp {
        fn execute_command(
            raw: &str,
            lines: State<Vec<TermLine>>,
            history: State<Vec<String>>,
            next_id: State<u64>,
            cmd_count: State<u32>,
        ) {
            let cmd = raw.trim();
            if cmd.is_empty() {
                return;
            }
            cmd_count.update(|n| *n += 1);
            history.update(|h| h.push(cmd.to_owned()));
            if cmd == "clear" {
                lines.set(Vec::new());
                return;
            }
            let push_line = |text: String, tone: LineTone| {
                let id = next_id.get_untracked();
                next_id.set(id + 1);
                lines.update(|buf| buf.push(TermLine { id, text, tone }));
            };
            push_line(format!("❯ {cmd}"), LineTone::Prompt);
            match cmd {
                "help" => push_line(
                    "Built-in commands: help, ls, ls -la, pwd, whoami, uname -a, sysinfo, cargo test, echo <msg>, cat README.md, history, clear"
                        .to_owned(),
                    LineTone::Accent,
                ),
                "ls" | "ls -la" | "ll" => {
                    push_line(
                        "drwxr-xr-x  crates/    drwxr-xr-x  docs/      -rw-r--r--  Cargo.toml"
                            .to_owned(),
                        LineTone::Output,
                    );
                }
                "sysinfo" => {
                    push_line("  Status   : GPU 60 fps ready · AccessKit active".to_owned(), LineTone::Success);
                }
                other if other.starts_with("echo ") => {
                    push_line(other[5..].to_owned(), LineTone::Success);
                }
                other => {
                    push_line(format!("rustui-sh: executed `{other}` (exit 0)"), LineTone::Error);
                }
            }
        }
    }

    impl View for TerminalApp {
        fn body(self, cx: &mut Cx) -> impl View {
            let prompt_line = cx.state(String::new());
            let cmd_count = cx.state(1u32);
            let history = cx.state(vec!["sysinfo".to_owned()]);
            let next_id = cx.state(7u64);
            let lines = cx.state(vec![
                TermLine {
                    id: 1,
                    text: "RustUI Terminal v0.1.0 · GPU Monospace Renderer (Parley + Wgpu)".into(),
                    tone: LineTone::Accent,
                },
                TermLine {
                    id: 3,
                    text: "❯ sysinfo".into(),
                    tone: LineTone::Prompt,
                },
                TermLine {
                    id: 6,
                    text:
                        "  Status   : Clipboard Copy/Paste, Context Menu, Touch & AccessKit Ready"
                            .into(),
                    tone: LineTone::Success,
                },
            ]);
            let submit_cmd = move || {
                let input = prompt_line.get_untracked();
                Self::execute_command(&input, lines, history, next_id, cmd_count);
                prompt_line.set(String::new());
            };

            VStack::new(
                10,
                (
                    Toolbar::new((
                        HStack::new(
                            6,
                            (
                                EmptyView
                                    .frame(Frame::fixed(10.0, 10.0))
                                    .corner_radius(5.0)
                                    .background(Color::from_hex(0xef4444)),
                                EmptyView
                                    .frame(Frame::fixed(10.0, 10.0))
                                    .corner_radius(5.0)
                                    .background(Color::from_hex(0xf59e0b)),
                                EmptyView
                                    .frame(Frame::fixed(10.0, 10.0))
                                    .corner_radius(5.0)
                                    .background(Color::from_hex(0x22c55e)),
                            ),
                        ),
                        Text::new(move || {
                            format!("zsh — ~/projects/rustui · {} cmds", cmd_count.get())
                        })
                        .font(Font::monospace()),
                        Spacer::new(),
                        Button::new("Help", move || {
                            Self::execute_command("help", lines, history, next_id, cmd_count);
                        }),
                        Button::new("ls -la", move || {
                            Self::execute_command("ls -la", lines, history, next_id, cmd_count);
                        }),
                        Button::new("sysinfo", move || {
                            Self::execute_command("sysinfo", lines, history, next_id, cmd_count);
                        }),
                        Button::new("Clear", move || {
                            Self::execute_command("clear", lines, history, next_id, cmd_count);
                        }),
                    )),
                    ScrollView::vertical(
                        VStack::new(
                            5,
                            ForEach::new(
                                move || lines.get(),
                                |line| line.id,
                                |line| {
                                    Text::new(line.text)
                                        .font(Font::monospace())
                                        .color(line.tone.color())
                                },
                            ),
                        )
                        .alignment(HorizontalAlignment::Leading)
                        .padding(14)
                        .frame(
                            Frame::new()
                                .max_width(f32::INFINITY)
                                .alignment(Alignment::TOP_LEADING),
                        ),
                    )
                    .frame(
                        Frame::new()
                            .max_width(f32::INFINITY)
                            .max_height(f32::INFINITY)
                            .alignment(Alignment::TOP_LEADING),
                    )
                    .background(Color::from_hex(0x0b1120))
                    .border(1, Color::from_hex(0x1e293b))
                    .corner_radius(10)
                    .padding(Edges::symmetric(0.0, 12.0)),
                    HStack::new(
                        8,
                        (
                            Text::new("rustui ❯")
                                .font(Font::monospace())
                                .color(Color::from_hex(0x4ade80)),
                            TextField::new(
                                "Enter command (help, ls, sysinfo, cargo test, echo, clear)...",
                                prompt_line.binding(),
                            )
                            .on_submit(move |_| submit_cmd()),
                            Button::new("Run", submit_cmd),
                        ),
                    )
                    .padding(Edges::symmetric(8.0, 12.0))
                    .background(Color::from_hex(0x111827))
                    .border(1, Color::from_hex(0x334155))
                    .corner_radius(10)
                    .padding(Edges::new(0.0, 12.0, 12.0, 12.0)),
                ),
            )
            .frame(
                Frame::new()
                    .max_width(f32::INFINITY)
                    .max_height(f32::INFINITY)
                    .alignment(Alignment::TOP_LEADING),
            )
            .background(Color::background())
            .environment(Theme::dark().to_palette())
        }
    }

    let mut sim = HeadlessSimulator::new(TerminalApp, Size::new(1060.0, 680.0))
        .with_clear_color(Theme::dark().colors.background);
    assert_layout_invariants(&sim, "terminal initial");
    assert_accesskit_valid(&sim, "terminal initial");
    assert!(find_live_text(&sim, "RustUI Terminal v0.1.0").is_some());
    assert!(find_live_text(&sim, "❯ sysinfo").is_some());

    // Enter command into prompt and submit via Enter key -> output rendered cleanly.
    let prompt = find_live_control(
        &sim,
        "textfield",
        "Enter command (help, ls, sysinfo, cargo test, echo, clear)...",
    )
    .expect("terminal prompt");
    sim.click_node(prompt);
    sim.type_text("echo hello rustui");
    sim.press_key(KeyCode::Enter, Modifiers::NONE);
    assert!(find_live_text(&sim, "❯ echo hello rustui").is_some());
    assert!(find_live_text(&sim, "hello rustui").is_some());
    assert!(find_live_text(&sim, "2 cmds").is_some());

    // Quick action button "ls -la" appends directory output.
    let ls_btn = find_live_control(&sim, "button", "ls -la").expect("ls -la btn");
    sim.click_node(ls_btn);
    assert!(find_live_text(&sim, "Cargo.toml").is_some());

    // Right-click Context Menu on prompt TextField (Select All -> Copy -> Paste) + Run button.
    sim.click_node(prompt);
    sim.type_text("echo menu_ok");
    let pb = sim.node_bounds(prompt).expect("prompt bounds");
    sim.right_click(pb.x + 24.0, pb.mid_y());
    let sel_all = find_live_control(&sim, "button", "Select All").expect("prompt Select All");
    sim.click_node(sel_all);
    sim.right_click(pb.x + 24.0, pb.mid_y());
    let copy_btn = find_live_control(&sim, "button", "Copy").expect("prompt Copy");
    sim.click_node(copy_btn);
    sim.press_key(KeyCode::Delete, Modifiers::NONE);
    sim.right_click(pb.x + 24.0, pb.mid_y());
    let paste_btn = find_live_control(&sim, "button", "Paste").expect("prompt Paste");
    sim.click_node(paste_btn);
    let run_btn = find_live_control(&sim, "button", "Run").expect("Run btn");
    sim.click_node(run_btn);
    assert!(find_live_text(&sim, "menu_ok").is_some());

    // Clear clears scrollback; help command repopulates built-in listing.
    let clear_btn = find_live_control(&sim, "button", "Clear").expect("Clear btn");
    sim.click_node(clear_btn);
    assert!(find_live_text(&sim, "menu_ok").is_none());

    sim.click_node(prompt);
    sim.type_text("help");
    sim.click_node(run_btn);
    assert!(find_live_text(&sim, "Built-in commands:").is_some());

    assert!(mean_luminance(&sim.render_rgba()) < 0.35);
    assert_layout_invariants(&sim, "terminal updated");
}

#[test]
fn e2e_touch_gestures_and_reduced_motion() {
    // OS theme & reduced-motion detection return valid typed results without panicking.
    let _sys_theme = detect_system_theme();
    let _sys_rm = detect_reduced_motion();

    struct TouchApp;
    impl View for TouchApp {
        fn body(self, cx: &mut Cx) -> impl View {
            let taps = cx.state(0u32);
            let scale = cx.state(1.0f32);
            let toggled = cx.state(false);
            VStack::new(
                12,
                (
                    HStack::new(
                        12,
                        (
                            Button::new("Tap Touch", move || taps.update(|t| *t += 1)),
                            Text::new(move || format!("Taps: {}", taps.get())),
                            Toggle::new("Switch", toggled.binding()),
                            Text::new(move || format!("Pinch: {:.2}", scale.get())),
                        ),
                    ),
                    Canvas::new(move |rect, dl| {
                        dl.fill_rounded_rect(rect, CornerRadii::all(8.0), Color::surface());
                    })
                    .frame(Frame::new().width(240.0).height(120.0))
                    .on_pinch(move |ev| {
                        scale.update(|s| *s = (*s + ev.scale_delta).max(0.25));
                    }),
                    ScrollView::vertical(VStack::new(
                        4,
                        ForEach::new(
                            || (0..30).collect::<Vec<usize>>(),
                            |i| *i,
                            |i| {
                                Text::new(format!("Touch Scroll Row #{i}"))
                                    .frame(Frame::new().height(26.0))
                            },
                        ),
                    ))
                    .frame(Frame::new().width(280.0).height(160.0)),
                ),
            )
            .alignment(HorizontalAlignment::Leading)
            .padding(16)
            .frame(
                Frame::new()
                    .max_width(f32::INFINITY)
                    .max_height(f32::INFINITY)
                    .alignment(Alignment::TOP_LEADING),
            )
        }
    }

    let mut sim = HeadlessSimulator::new(TouchApp, Size::new(560.0, 440.0));
    assert_layout_invariants(&sim, "touch initial");

    // 1. Single-finger touch tap activates Button.
    let tap_btn = find_live_control(&sim, "button", "Tap Touch").expect("Tap Touch button");
    let tb = sim.node_bounds(tap_btn).expect("Tap Touch bounds");
    sim.touch_down(1, tb.mid_x(), tb.mid_y());
    sim.touch_up(1, tb.mid_x(), tb.mid_y());
    assert!(find_live_text(&sim, "Taps: 1").is_some());

    // 2. Single-finger touch drag inside ScrollView scrolls content vertically.
    let row0_before = find_live_text(&sim, "Touch Scroll Row #0")
        .and_then(|id| sim.node_bounds(id))
        .expect("Row 0 before scroll");
    let sv = find_live_node(&sim, |n| matches!(n.kind, NodeKind::Scroll(_))).expect("ScrollView");
    let sv_b = sim.node_bounds(sv).expect("ScrollView bounds");
    sim.touch_down(2, sv_b.mid_x(), sv_b.max_y() - 20.0);
    sim.touch_move(2, sv_b.mid_x(), sv_b.max_y() - 90.0);
    sim.touch_move(2, sv_b.mid_x(), sv_b.max_y() - 160.0);
    sim.touch_up(2, sv_b.mid_x(), sv_b.max_y() - 160.0);
    let row0_after = find_live_text(&sim, "Touch Scroll Row #0")
        .and_then(|id| sim.node_bounds(id))
        .expect("Row 0 after touch scroll");
    assert!(
        row0_after.y < row0_before.y - 40.0,
        "expected touch drag to scroll rows upward: before={row0_before:?}, after={row0_after:?}"
    );

    // 3. Two-finger touch pinch dispatches PinchEvent to Canvas::on_pinch.
    let canvas = find_live_node(&sim, |n| matches!(n.kind, NodeKind::Canvas(_))).expect("Canvas");
    let cb = sim.node_bounds(canvas).expect("Canvas bounds");
    sim.touch_down(10, cb.mid_x() - 20.0, cb.mid_y());
    sim.touch_down(11, cb.mid_x() + 20.0, cb.mid_y());
    sim.touch_move(10, cb.mid_x() - 50.0, cb.mid_y());
    sim.touch_move(11, cb.mid_x() + 50.0, cb.mid_y());
    sim.touch_up(10, cb.mid_x() - 50.0, cb.mid_y());
    sim.touch_up(11, cb.mid_x() + 50.0, cb.mid_y());
    assert!(
        find_live_text(&sim, "Pinch: 1.00").is_none(),
        "two-finger touch pinch should change displayed Pinch scale"
    );

    // 4. Reduced motion stops queued animations cleanly.
    let sw = find_live_control(&sim, "toggle", "Switch").expect("Switch toggle");
    sim.click_node(sw);
    sim.set_reduced_motion(true);
    assert!(sim.runtime.is_reduced_motion());
    assert!(
        sim.runtime
            .with_inner(|inner| inner.scheduler.animations.is_empty()),
        "enabling reduced motion must flush active animations immediately"
    );
}

#[test]
fn e2e_v02_gallery_and_text_scaling() {
    #[derive(Observable, Clone, Default)]
    struct ShellModel {
        split_ratio: f32,
        nav_collapsed: bool,
        active_tab: usize,
        palette_open: bool,
        alert_open: bool,
        editor: EditorBufferState,
        tabs: Vec<TabBarItem>,
    }

    struct V02Shell;
    impl View for V02Shell {
        fn body(self, cx: &mut Cx) -> impl View {
            let model = cx.model(ShellModel {
                split_ratio: 0.25,
                nav_collapsed: false,
                active_tab: 0,
                palette_open: false,
                alert_open: false,
                editor: EditorBufferState::from_text("fn alpha() {}\nfn beta() {}\n"),
                tabs: vec![
                    TabBarItem::new("a", "alpha.rs").pinned(true),
                    TabBarItem::new("b", "beta.rs"),
                ],
            });
            let theme = Theme::dark();
            let cmds = vec![CommandItem::new(
                "go.b",
                "Switch to Beta",
                "Ctrl+2",
                move || {
                    model.set_active_tab(1);
                },
            )];
            let nav = VStack::new(
                8,
                (Text::new("Navigator").font(Font::headline()), Spacer::new()),
            )
            .padding(10)
            .frame(
                Frame::new()
                    .max_width(f32::INFINITY)
                    .max_height(f32::INFINITY),
            );

            let t_alert = theme.clone();
            let main_pane = VStack::new(
                0,
                (
                    TabBar::new(model.tabs_binding(), model.active_tab_binding()),
                    Switch::new(move || model.active_tab())
                        .case(0usize, move || {
                            EditorSurface::new(model.editor_binding())
                                .accessibility_label("Gallery Editor")
                        })
                        .default(move || {
                            VStack::new(
                                10,
                                (
                                    ProgressView::determinate(0.5).label("Indexing"),
                                    Gauge::new("Load", 40.0, 0.0, 100.0),
                                    Button::new("Trigger Alert", move || {
                                        model.set_alert_open(true)
                                    })
                                    .style(ButtonStyle::primary(&t_alert)),
                                    Spacer::new(),
                                ),
                            )
                            .padding(14)
                        }),
                ),
            )
            .frame(
                Frame::new()
                    .max_width(f32::INFINITY)
                    .max_height(f32::INFINITY),
            );

            let t_tb = theme.clone();
            Alert::new(
                model.alert_open_binding(),
                "Modal Check",
                "Alert dialog Role verification",
                vec![AlertButton::new("OK", || {})],
                CommandPalette::new(
                    model.palette_open_binding(),
                    cmds,
                    VStack::new(
                        0,
                        (
                            Toolbar::new((
                                Text::new("RustUI 0.2").font(Font::headline()),
                                Spacer::new(),
                                Button::new("Palette", move || model.set_palette_open(true))
                                    .style(ButtonStyle::primary(&t_tb)),
                            )),
                            SplitView::horizontal(model.split_ratio_binding(), nav, main_pane)
                                .collapsed(model.nav_collapsed_binding())
                                .frame(
                                    Frame::new()
                                        .max_width(f32::INFINITY)
                                        .max_height(f32::INFINITY),
                                ),
                            StatusBar::new((
                                Text::new(move || model.editor().status_summary()),
                                Spacer::new(),
                                Text::new("UTF-8"),
                            )),
                        ),
                    )
                    .frame(
                        Frame::new()
                            .max_width(f32::INFINITY)
                            .max_height(f32::INFINITY),
                    ),
                ),
            )
        }
    }

    let mut sim = HeadlessSimulator::new(V02Shell, Size::new(1100.0, 720.0));
    sim.step_frame();
    assert_layout_invariants(&sim, "v02 shell initial");

    let sb_b = sim
        .node_bounds(sim.find_node_by_label("Status Bar").expect("Status Bar"))
        .expect("sb bounds");
    let ed_id = sim
        .find_node_by_label("Gallery Editor")
        .expect("Gallery Editor");
    let ed_b = sim.node_bounds(ed_id).expect("ed bounds");
    assert!(
        sb_b.max_y() >= 718.0 && sb_b.max_y() <= 721.0,
        "StatusBar must anchor at bottom of viewport (got {sb_b:?})"
    );
    assert!(
        ed_b.height > 400.0,
        "EditorSurface must fill remaining height cleanly (got {ed_b:?})"
    );

    // Multi-cursor typing in EditorSurface
    sim.click_node(ed_id);
    sim.type_text("//");
    sim.press_key(
        KeyCode::ArrowDown,
        Modifiers {
            ctrl: true,
            alt: true,
            ..Modifiers::NONE
        },
    );
    sim.type_text("*");
    let ak_editor = sim.accessibility_tree();
    assert!(
        ak_editor
            .nodes
            .iter()
            .any(|(_, n)| n.role() == Role::Status),
        "StatusBar should expose AccessKit Status role"
    );

    // CommandPalette opens and executes tab switch -> reveals Gauge (Meter) & Alert (AlertDialog)
    let pal_btn = sim.find_node_by_label("Palette").expect("Palette button");
    sim.click_node(pal_btn);
    let cmd_item = sim
        .find_node_by_label("Switch to Beta   [Ctrl+2]")
        .expect("command item");
    sim.click_node(cmd_item);

    let alert_btn = sim
        .find_node_by_label("Trigger Alert")
        .expect("Trigger Alert button");
    sim.click_node(alert_btn);
    let ak_alert = sim.accessibility_tree();
    assert!(
        ak_alert.nodes.iter().any(|(_, n)| n.role() == Role::Meter),
        "Gauge should expose AccessKit Meter role"
    );
    assert!(
        ak_alert
            .nodes
            .iter()
            .any(|(_, n)| n.role() == Role::AlertDialog),
        "Alert should expose AccessKit AlertDialog role"
    );
    let ok_btn = sim.find_node_by_label("OK").expect("OK button");
    sim.click_node(ok_btn);

    // 200% text scaling check
    sim.runtime.set_text_scale_factor(2.0);
    sim.step_frame();
    assert_layout_invariants(&sim, "v02 shell 200% scale");
}
