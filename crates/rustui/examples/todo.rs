//! Todo list example showcasing OS-reactive dark/light theme, `TextField`,
//! priority `Picker`, effort `Slider`, validation, `Checkbox`, and keyed `ForEach`.

use rustui::prelude::*;

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
        let os_is_dark = cx.env::<ThemePalette>().is_some_and(|p| p.is_dark);
        let dark = cx.state(os_is_dark);
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
                                Text::new(move || format!("{} pts", effort.get().round() as u32))
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

fn main() -> Result<(), String> {
    tracing_subscriber::fmt::try_init().ok();
    App::new(TodoApp)
        .title("RustUI Todo")
        .size(640.0, 580.0)
        .run()
}
