//! Performance target verification suite (Section 10):
//! - Idle app: 0 redraws
//! - Single state change in a 10,000-node tree: < 1 ms signal update, < 8 ms total frame
//! - Scrolling a virtualized list of 1,000,000 rows: < 16.6 ms per frame (60 fps)
//! - Terminal grid 200x60 cells: < 16.6 ms per frame (60 fps)

use std::time::Instant;

use rustui::prelude::*;

#[test]
fn idle_app_performs_zero_redraws() {
    struct IdleView;
    impl View for IdleView {
        fn body(self, _cx: &mut Cx) -> impl View {
            VStack::new(8, (Text::new("Idle"), Button::new("Ok", || {}))).padding(12)
        }
    }

    let mut sim = HeadlessSimulator::new(IdleView, Size::new(400.0, 300.0));
    for _ in 0..10 {
        let summary = sim.step_frame();
        assert!(!summary.redrawn, "idle frame must perform 0 redraws");
    }
}

#[test]
fn single_state_change_in_10k_node_tree_meets_latency_budget() {
    struct HugeTree {
        state_out: std::rc::Rc<std::cell::Cell<Option<State<i32>>>>,
    }
    impl View for HugeTree {
        fn body(self, cx: &mut Cx) -> impl View {
            let count = cx.state(0i32);
            self.state_out.set(Some(count));
            let mut rows = Vec::with_capacity(10_000);
            for i in 0..10_000 {
                if i == 4_999 {
                    rows.push(
                        Text::new(move || format!("Active: {}", count.get()))
                            .frame(Frame::fixed(120.0, 20.0))
                            .into_any(),
                    );
                } else {
                    rows.push(Text::new("Row").frame(Frame::fixed(120.0, 20.0)).into_any());
                }
            }
            VStack::new(2, rows)
        }
    }

    let slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let mut sim = HeadlessSimulator::new(
        HugeTree {
            state_out: slot.clone(),
        },
        Size::new(600.0, 800.0),
    );
    assert!(sim.runtime.node_count() >= 10_000);

    let state = slot.get().expect("state initialized");
    let t0 = Instant::now();
    sim.runtime.enter(|| state.set(99));
    let summary = sim.step_frame();
    let total_elapsed = t0.elapsed();
    let signal_time = sim
        .runtime
        .with_inner(|i| i.scheduler.metrics.signal_update_time);

    assert_eq!(summary.reactive_updates, 1);
    assert!(
        summary.laid_out_nodes <= 4,
        "expected fixed-size boundary to limit re-layout to <= 4 nodes, laid out {}",
        summary.laid_out_nodes
    );
    assert!(
        signal_time.as_secs_f64() < 0.002,
        "signal update took {:?} (expected < 1ms)",
        signal_time
    );
    assert!(
        total_elapsed.as_secs_f64() < 0.015,
        "10k tree frame update took {:?}",
        total_elapsed
    );
    assert!(sim.find_node_by_label("Active: 99").is_some());
}

#[test]
fn scrolling_1m_row_virtualized_list_exceeds_60_fps_budget() {
    let view = LazyVStack::new(1_000_000, 24, |idx| Text::new(format!("Item {idx}")))
        .with_pool_updater(|idx, node| format_virtual_row_text(node, "Item ", idx));
    let mut sim = HeadlessSimulator::new(view, Size::new(500.0, 600.0));

    let frames = 60usize;
    let start = Instant::now();
    for _ in 0..frames {
        sim.scroll(Point::new(100.0, 100.0), 0.0, -45.0);
    }
    let elapsed = start.elapsed();
    let per_frame_ms = elapsed.as_secs_f64() * 1000.0 / frames as f64;
    assert!(
        per_frame_ms < 16.6,
        "1M row scroll averaged {per_frame_ms:.3} ms/frame (budget 16.6 ms)"
    );
}

#[test]
fn terminal_grid_200x60_emits_within_60fps_budget() {
    const COLS: usize = 200;
    const ROWS: usize = 60;

    let view = Canvas::new(|rect, dl| {
        dl.fill_rect(rect, Color::BLACK);
        let cw = rect.width / COLS as f32;
        let ch = rect.height / ROWS as f32;
        for r in 0..ROWS {
            for c in 0..COLS {
                dl.fill_rect(
                    Rect::new(
                        rect.x + c as f32 * cw,
                        rect.y + r as f32 * ch,
                        cw - 1.0,
                        ch - 1.0,
                    ),
                    Color::GREEN,
                );
            }
        }
    })
    .frame(Frame::fixed(1200.0, 720.0));

    let mut sim = HeadlessSimulator::new(view, Size::new(1200.0, 720.0));
    let start = Instant::now();
    let list = sim.build_display_list();
    let elapsed = start.elapsed();
    assert_eq!(list.commands.len(), COLS * ROWS + 1);
    assert_eq!(
        list.batch_count(),
        1,
        "all terminal cells coalesce into 1 batch"
    );
    assert!(
        elapsed.as_secs_f64() * 1000.0 < 16.6,
        "200x60 terminal display list build took {:?}",
        elapsed
    );
}
