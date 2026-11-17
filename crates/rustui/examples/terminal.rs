//! High-performance 200x60 cell terminal grid example built on `Canvas` and `TextField` (Section 10).

use rustui::prelude::*;

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
                            // Draw active terminal cell background + glyph foreground blocks in batch
                            if (r + c + frame_t as usize).is_multiple_of(5) {
                                let x = rect.x + c as f32 * cell_w;
                                let y = rect.y + r as f32 * cell_h;
                                let color = if r == ROWS - 1 && c < 12 {
                                    Color::from_hex(0x22c55e)
                                } else if (c + r).is_multiple_of(3) {
                                    Color::from_hex(0x38bdf8)
                                } else {
                                    Color::from_hex(0xe2e8f0)
                                };
                                dl.fill_rect(
                                    Rect::new(
                                        x + 0.5,
                                        y + 1.0,
                                        (cell_w - 1.0).max(1.0),
                                        (cell_h - 2.0).max(1.0),
                                    ),
                                    color.with_alpha(0.75),
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
                        TextField::new("Enter shell command...", prompt_line.binding()).on_submit(
                            move |_| {
                                ticks.update(|t| *t += 1);
                                prompt_line.set(String::new());
                            },
                        ),
                    ),
                )
                .padding(8)
                .background(Color::from_hex(0x111827)),
            ),
        )
    }
}

fn main() -> Result<(), String> {
    tracing_subscriber::fmt::try_init().ok();
    App::new(TerminalApp)
        .title("RustUI Terminal (200x60)")
        .size(1200.0, 760.0)
        .theme(Theme::dark())
        .run()
}
