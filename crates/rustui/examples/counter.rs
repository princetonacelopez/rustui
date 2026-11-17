//! Counter example matching Section 4 of the RustUI Project Specification.

use rustui::prelude::*;

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

fn main() -> Result<(), String> {
    tracing_subscriber::fmt::try_init().ok();
    App::new(Counter)
        .title("RustUI Counter")
        .size(400.0, 260.0)
        .run()
}
