//! Todo list example showcasing `TextField`, `Checkbox`, keyed `ForEach`, `Picker`, and `Theme`.

use rustui::prelude::*;

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
                            .padding(Edges::symmetric(4.0, 8.0))
                            .background(Color::surface())
                            .corner_radius(6)
                        },
                    ),
                )),
            ),
        )
        .padding(16)
    }
}

fn main() -> Result<(), String> {
    tracing_subscriber::fmt::try_init().ok();
    App::new(TodoApp)
        .title("RustUI Todo")
        .size(560.0, 520.0)
        .run()
}
