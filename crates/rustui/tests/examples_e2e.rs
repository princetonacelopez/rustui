//! End-to-end desktop example interaction, layout, state, overlay, and AccessKit tests.

use std::collections::{HashMap, HashSet};

use accesskit::{Action, ActionData, ActionRequest, NodeId as AkNodeId, Role, TreeId};
use rustui::core::{ContainerKind, NodeKind};
use rustui::platform::to_ak_id;
use rustui::prelude::*;

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

    // Keyboard traversal & activation + AccessKit Click action (clicking minus earlier focused minus, so Tab advances to plus then wraps to minus).
    let f_plus = sim.focus_next().expect("focus wraps to + button");
    assert_eq!(f_plus, plus);
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
        done: bool,
    }
    struct TodoApp;
    impl View for TodoApp {
        fn body(self, cx: &mut Cx) -> impl View {
            let draft = cx.state(String::new());
            let next_id = cx.state(4u64);
            let filter = cx.state(0usize);
            let dark = cx.state(false);
            let items = cx.state(vec![
                TodoItem {
                    id: 1,
                    title: "Build reactive signal arena".into(),
                    done: true,
                },
                TodoItem {
                    id: 2,
                    title: "Implement proposal/response layout".into(),
                    done: true,
                },
                TodoItem {
                    id: 3,
                    title: "Ship 60fps virtualized LazyVStack".into(),
                    done: false,
                },
            ]);
            let remaining = cx.memo(move || items.get().iter().filter(|t| !t.done).count());
            let add_item = move || {
                let text = draft.get_untracked().trim().to_owned();
                if text.is_empty() {
                    return;
                }
                let id = next_id.get_untracked();
                next_id.set(id + 1);
                items.update(|list| {
                    list.push(TodoItem {
                        id,
                        title: text,
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
                    HStack::new(
                        8,
                        (
                            TextField::new("Add a todo...", draft.binding())
                                .on_submit(move |_| add_item()),
                            Button::new("Add", add_item),
                        ),
                    ),
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
                    )),
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

    let mut sim = HeadlessSimulator::new(TodoApp, Size::new(560.0, 640.0));
    assert_layout_invariants(&sim, "todo initial");
    assert_accesskit_valid(&sim, "todo initial");
    assert!(find_live_text(&sim, "1 remaining").is_some());

    // Type new todo in TextField and submit with Enter.
    let field = find_live_control(&sim, "textfield", "Add a todo...").expect("todo textfield");
    sim.click_node(field);
    sim.type_text("Write E2E tests");
    sim.press_key(KeyCode::Enter, Modifiers::NONE);
    assert!(find_live_control(&sim, "checkbox", "Write E2E tests").is_some());
    assert!(find_live_text(&sim, "2 remaining").is_some());

    // Type another todo and click Add button.
    let add_btn = find_live_control(&sim, "button", "Add").expect("Add button");
    sim.click_node(field);
    sim.type_text("Verify Dark Mode");
    sim.click_node(add_btn);
    assert!(find_live_control(&sim, "checkbox", "Verify Dark Mode").is_some());
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

    // Filter with Picker -> Completed (click cycles All -> Active -> Completed).
    let picker = find_live_control(&sim, "picker", "").expect("filter picker");
    sim.click_node(picker); // Active
    assert!(find_live_control(&sim, "checkbox", "Write E2E tests").is_some());
    assert!(find_live_control(&sim, "checkbox", "Verify Dark Mode").is_none());
    sim.click_node(picker); // Completed
    assert!(find_live_control(&sim, "checkbox", "Write E2E tests").is_none());
    assert!(find_live_control(&sim, "checkbox", "Verify Dark Mode").is_some());
    sim.click_node(picker); // All
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

    // Drag SplitView splitter rightward and verify sidebar grows.
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

    // TextEditor typing + IME composition + Sample CJK/Emoji + Clear buttons.
    let editor = find_live_control(&sim, "texteditor", "").expect("text_editor control");
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
    struct DashboardApp;
    impl View for DashboardApp {
        fn body(self, cx: &mut Cx) -> impl View {
            let tab = cx.state(0usize);
            let nav = cx.state(0usize);
            let gain = cx.state(0.72f32);
            let selected_row = cx.state(None::<usize>);
            let show_sheet = cx.state(false);
            let show_popover = cx.state(false);

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

    // TabView -> 1M Virtual List -> scroll -> return to Overview.
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

    // Clicking inside the sheet card absorbs clicks without dismissing the sheet.
    let summary_text = find_live_text(&sim, "Telemetry Summary").expect("Telemetry Summary");
    sim.click_node(summary_text);
    assert!(find_live_text(&sim, "Telemetry Summary").is_some());

    // Clicking Close explicitly dismisses the sheet.
    let close_btn = find_live_control(&sim, "button", "Close").expect("Close button");
    sim.click_node(close_btn);
    assert!(find_live_text(&sim, "Telemetry Summary").is_none());

    // Re-open via Window menu, then click the dimmed backdrop at Info coordinates:
    // the backdrop absorbs the click (Info popover stays closed) and dismisses the Sheet.
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

#[test]
fn e2e_terminal_example() {
    const COLS: usize = 200;
    const ROWS: usize = 60;
    struct TerminalApp;
    impl View for TerminalApp {
        fn body(self, cx: &mut Cx) -> impl View {
            let prompt_line = cx.state(String::new());
            let ticks = cx.state(1u32);
            VStack::new(
                0,
                (
                    Toolbar::new((
                        Text::new(move || {
                            format!("rustui-term · {COLS}x{ROWS} · frame {}", ticks.get())
                        })
                        .font(Font::monospace()),
                        Spacer::new(),
                        Button::new("Stream Output", move || ticks.update(|t| *t += 1)),
                    )),
                    Canvas::new(move |rect, dl| {
                        dl.fill_rect(rect, Color::from_hex(0x0b101b));
                        let cell_w = (rect.width / COLS as f32).max(2.0);
                        let cell_h = (rect.height / ROWS as f32).max(4.0);
                        let frame_t = ticks.get();
                        for r in 0..ROWS {
                            for c in 0..COLS {
                                if (r + c + frame_t as usize).is_multiple_of(5) {
                                    dl.fill_rect(
                                        Rect::new(
                                            rect.x + c as f32 * cell_w,
                                            rect.y + r as f32 * cell_h,
                                            (cell_w - 1.0).max(1.0),
                                            (cell_h - 2.0).max(1.0),
                                        ),
                                        Color::from_hex(0x38bdf8).with_alpha(0.75),
                                    );
                                }
                            }
                        }
                    })
                    .frame(
                        Frame::new()
                            .max_width(f32::INFINITY)
                            .max_height(f32::INFINITY),
                    ),
                    HStack::new(
                        8,
                        (
                            Text::new("$").font(Font::monospace()).color(Color::GREEN),
                            TextField::new("Enter shell command...", prompt_line.binding())
                                .on_submit(move |_| {
                                    ticks.update(|t| *t += 1);
                                    prompt_line.set(String::new());
                                }),
                        ),
                    )
                    .padding(8)
                    .background(Color::from_hex(0x111827)),
                ),
            )
            .environment(Theme::dark().to_palette())
        }
    }

    let mut sim = HeadlessSimulator::new(TerminalApp, Size::new(1200.0, 760.0))
        .with_clear_color(Theme::dark().colors.background);
    assert_layout_invariants(&sim, "terminal initial");
    assert_accesskit_valid(&sim, "terminal initial");
    assert!(find_live_text(&sim, "frame 1").is_some());

    let prompt = find_live_control(&sim, "textfield", "Enter shell command...").expect("prompt");
    sim.click_node(prompt);
    sim.type_text("ls -la");
    sim.press_key(KeyCode::Enter, Modifiers::NONE);
    assert!(find_live_text(&sim, "frame 2").is_some());

    let stream_btn = find_live_control(&sim, "button", "Stream Output").expect("stream button");
    sim.click_node(stream_btn);
    assert!(find_live_text(&sim, "frame 3").is_some());
    assert!(mean_luminance(&sim.render_rgba()) < 0.35);
    assert_layout_invariants(&sim, "terminal updated");
}
