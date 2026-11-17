//! Rich text editor example with `Toolbar`, `SplitView`, `Tree`, `TextEditor`, and live `Memo` stats.

use rustui::prelude::*;

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

fn main() -> Result<(), String> {
    tracing_subscriber::fmt::try_init().ok();
    App::new(TextEditorApp)
        .title("RustUI Text Editor")
        .size(860.0, 580.0)
        .run()
}
