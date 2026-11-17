//! Counter example matching Section 4 of the RustUI Project Specification.

use rustui::prelude::*;

struct Counter;

impl View for Counter {
    fn body(self, cx: &mut Cx) -> impl View {
        let count = cx.state(0);
        VStack::new(
            12,
            (
                Text::new(move || format!("Count: {}", count.get())).font(Font::title()),
                HStack::new(
                    8,
                    (
                        Button::new("-", move || count.update(|c| *c -= 1)),
                        Button::new("+", move || count.update(|c| *c += 1)),
                    ),
                ),
            ),
        )
        .padding(16)
    }
}

fn main() -> Result<(), String> {
    tracing_subscriber::fmt::try_init().ok();
    App::new(Counter)
        .title("RustUI Counter")
        .size(400.0, 260.0)
        .run()
}
