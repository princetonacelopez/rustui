//! Property/fuzz tests for layout negotiation inputs and rope text editing operations (Section 10).

use rustui::prelude::*;

struct Rng(u64);
impl Rng {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.0 >> 32) as u32
    }
    fn next_usize(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u32() as usize) % bound
        }
    }
}

#[test]
fn fuzz_layout_proposals_and_directions_never_panic_or_produce_nan() {
    let mut rng = Rng(0xdead_beef_cafe_babe);
    let proposals = [
        Size::new(0.0, 0.0),
        Size::new(1.0, 1.0),
        Size::new(120.5, 42.3),
        Size::new(800.0, 600.0),
        Size::new(4096.0, 2160.0),
    ];

    for iter in 0..64 {
        let dir = if iter % 2 == 0 {
            TextDirection::Ltr
        } else {
            TextDirection::Rtl
        };
        let spacing = (rng.next_u32() % 24) as f32;
        let view = VStack::new(
            spacing,
            (
                HStack::new(
                    spacing,
                    (
                        Text::new("Alpha"),
                        Spacer::new(),
                        Button::new("Beta", || {}),
                    ),
                ),
                Grid::new(
                    vec![
                        GridTrack::Fixed(60.0),
                        GridTrack::Flexible {
                            min: 20.0,
                            max: 300.0,
                        },
                    ],
                    (
                        Text::new("Cell 1"),
                        Text::new("Cell 2"),
                        Text::new("Cell 3"),
                    ),
                ),
            ),
        )
        .text_direction(dir)
        .padding((rng.next_u32() % 16) as i32);

        let vp = proposals[rng.next_usize(proposals.len())];
        let mut sim = HeadlessSimulator::new(view, vp);
        sim.step_frame();
        let bounds = sim.node_bounds(sim.root).unwrap();
        assert!(bounds.width.is_finite() && bounds.width >= 0.0);
        assert!(bounds.height.is_finite() && bounds.height >= 0.0);
    }
}

#[test]
fn fuzz_rope_text_buffer_operations_preserve_valid_utf8_and_undo_invariants() {
    let mut rng = Rng(0x1234_5678_9abc_def0);
    let samples = ["a", " ", "\n", "é", "日", "中", "🚀", "👨‍👩‍👧‍👦"];
    let mut buf = TextBuffer::new("");

    for _ in 0..500 {
        match rng.next_usize(10) {
            0..=3 => {
                let s = samples[rng.next_usize(samples.len())];
                buf.insert(s);
            }
            4 => buf.delete_backward(),
            5 => buf.delete_forward(),
            6 => buf.move_left(rng.next_usize(2) == 1),
            7 => buf.move_right(rng.next_usize(2) == 1),
            8 => {
                buf.undo();
            }
            _ => {
                buf.redo();
            }
        }
        let text = buf.text();
        let sel = buf.selection();
        assert!(sel.anchor <= text.len() && text.is_char_boundary(sel.anchor));
        assert!(sel.active <= text.len() && text.is_char_boundary(sel.active));
    }
}
