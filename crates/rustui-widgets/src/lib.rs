//! Built-in widgets, styling, theming, and animations for RustUI.

pub mod collections;
pub mod containers;
pub mod core_widgets;
pub mod editor_surface;
pub mod input_widgets;
pub mod theme;

pub use collections::{
    LazyVStack, List, ScrollView, Table, TableColumn, Tree, TreeItemData, format_virtual_row_text,
};
pub use containers::{
    Alert, AlertButton, CommandItem, CommandPalette, ConfirmationDialog, DisclosureGroup,
    FileDialog, Form, GroupBox, LabeledContent, Menu, MenuItem, NavigationPage, NavigationStack,
    OutlineGroup, Popover, Section, Sheet, SplitView, StatusBar, TabBar, TabBarItem, TabItem,
    TabView, Toolbar,
};
pub use core_widgets::{
    Button, Canvas, CustomLayoutView, Divider, Gauge, Grid, HStack, Image, ProgressView, Shader,
    Spacer, TerminalLine, TerminalLineTone, TerminalView, Text, Toggle, VStack, ViewThatFits,
    WidgetViewExt, ZStack,
};
pub use editor_surface::{EditorBufferState, EditorSurface};
pub use input_widgets::{
    Checkbox, ColorPicker, DatePicker, Link, Picker, SecureField, Slider, Stepper, TextEditor,
    TextField,
};
pub use theme::{
    ButtonStyle, ControlSize, Font, ListStyle, PickerStyle, Theme, ThemeColors, ToggleStyle,
};

#[cfg(test)]
mod tests {
    use rustui_core::{
        Animation, Cx, KeyCode, Modifiers, Point, Size, View, ViewExt, with_animation,
    };
    use rustui_platform::HeadlessSimulator;

    use super::*;

    #[test]
    fn counter_example_updates_single_node_on_click() {
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

        let mut sim = HeadlessSimulator::new(Counter, Size::new(400.0, 300.0));
        assert!(sim.find_node_by_label("Count: 0").is_some());

        let plus_btn = sim.find_node_by_label("+").unwrap();
        let summary = sim.click_node(plus_btn);
        assert_eq!(summary.reactive_updates, 1);
        assert!(sim.find_node_by_label("Count: 1").is_some());

        let minus_btn = sim.find_node_by_label("-").unwrap();
        sim.click_node(minus_btn);
        assert!(sim.find_node_by_label("Count: 0").is_some());
    }

    #[test]
    fn textfield_and_texteditor_handle_japanese_chinese_emoji_ime() {
        struct EditorForm;
        impl View for EditorForm {
            fn body(self, cx: &mut Cx) -> impl View {
                let single = cx.state(String::new());
                let multi = cx.state(String::new());
                VStack::new(
                    10,
                    (
                        TextField::new("Title", single.binding()),
                        TextEditor::new(multi.binding()),
                    ),
                )
            }
        }

        let mut sim = HeadlessSimulator::new(EditorForm, Size::new(500.0, 400.0));
        let tf = sim.find_node_by_label("Title").unwrap();
        sim.click_node(tf);

        // Japanese IME pre-edit then commit
        sim.ime_preedit("にほんご", Some((0, 12)));
        sim.ime_commit("日本語");
        // Chinese + Emoji IME commit
        sim.ime_commit("中文🚀");

        assert!(sim.find_node_by_label("日本語中文🚀").is_some());

        // Tab to TextEditor and type multiline text
        sim.press_key(KeyCode::Tab, Modifiers::NONE);
        sim.type_text("Line1");
        sim.press_key(KeyCode::Enter, Modifiers::NONE);
        sim.type_text("Line2");
        assert!(sim.find_node_by_label("Line1\nLine2").is_some());
    }

    #[test]
    fn keyboard_only_form_controls_work() {
        struct Form;
        impl View for Form {
            fn body(self, cx: &mut Cx) -> impl View {
                let toggle = cx.state(false);
                let checked = cx.state(false);
                let slider = cx.state(0.0f32);
                let picker = cx.state(0usize);
                VStack::new(
                    8,
                    (
                        Toggle::new("Dark Mode", toggle.binding()),
                        Checkbox::new("Accept Terms", checked.binding()),
                        Slider::new(slider.binding(), 0.0, 10.0).step(1.0),
                        Picker::new(picker.binding(), vec!["Low".into(), "High".into()]),
                        Text::new(move || {
                            format!(
                                "state:{}:{}:{}:{}",
                                toggle.get(),
                                checked.get(),
                                slider.get() as i32,
                                picker.get()
                            )
                        }),
                    ),
                )
            }
        }

        let mut sim = HeadlessSimulator::new(Form, Size::new(400.0, 400.0));
        assert!(sim.find_node_by_label("state:false:false:0:0").is_some());

        // Tab -> Toggle -> Space
        sim.press_key(KeyCode::Tab, Modifiers::NONE);
        sim.press_key(KeyCode::Space, Modifiers::NONE);
        // Tab -> Checkbox -> Space
        sim.press_key(KeyCode::Tab, Modifiers::NONE);
        sim.press_key(KeyCode::Space, Modifiers::NONE);
        // Tab -> Slider -> Right arrow
        sim.press_key(KeyCode::Tab, Modifiers::NONE);
        sim.press_key(KeyCode::ArrowRight, Modifiers::NONE);
        // Tab -> Picker -> Down arrow
        sim.press_key(KeyCode::Tab, Modifiers::NONE);
        sim.press_key(KeyCode::ArrowDown, Modifiers::NONE);

        assert!(sim.find_node_by_label("state:true:true:1:1").is_some());
    }

    #[test]
    fn lazy_vstack_handles_one_million_rows_without_allocating_million_nodes() {
        let view = LazyVStack::new(1_000_000, 28, |idx| Text::new(format!("Row {idx}")))
            .with_pool_updater(|idx, node| format_virtual_row_text(node, "Row ", idx));

        let mut sim = HeadlessSimulator::new(view, Size::new(500.0, 600.0));
        assert!(
            sim.runtime.node_count() < 100,
            "1M virtual list must build only visible pool, got {} nodes",
            sim.runtime.node_count()
        );

        // Scroll down rapidly
        sim.scroll(Point::new(100.0, 100.0), 0.0, -3000.0);
        assert!(sim.find_node_by_label("Row 98").is_some());
    }

    #[test]
    fn containers_theming_and_canvas_render_cleanly() {
        struct Demo;
        impl View for Demo {
            fn body(self, cx: &mut Cx) -> impl View {
                let tab = cx.state(0usize);
                let nav = cx.state(0usize);
                let split = cx.state(0.4f32);
                let sheet = cx.state(false);
                let sel = cx.state(None::<usize>);

                Sheet::new(
                    sheet.binding(),
                    VStack::new(
                        8,
                        (
                            Toolbar::new((
                                Text::new("RustUI Demo").font(Font::headline()),
                                Spacer::new(),
                                Menu::new(
                                    "Actions",
                                    vec![MenuItem::new("Open Sheet", move || sheet.set(true))],
                                ),
                            )),
                            TabView::new(
                                tab.binding(),
                                vec![
                                    TabItem::new("Main", move || {
                                        SplitView::horizontal(
                                            split.binding(),
                                            NavigationStack::new(
                                                nav.binding(),
                                                vec![NavigationPage::new("Home", || {
                                                    Text::new("Welcome")
                                                })],
                                            ),
                                            Table::new(
                                                vec![
                                                    TableColumn::fixed("ID", 60.0),
                                                    TableColumn::flexible("Name", 100.0, 300.0),
                                                ],
                                                5,
                                                sel.binding(),
                                                |r, c| format!("{r}:{c}"),
                                            ),
                                        )
                                    }),
                                    TabItem::new("Canvas", || {
                                        Canvas::new(|rect, dl| {
                                            dl.fill_rounded_rect(
                                                rect,
                                                rustui_core::CornerRadii::all(8.0),
                                                rustui_core::Color::accent(),
                                            );
                                        })
                                    }),
                                ],
                            ),
                        ),
                    ),
                    || Text::new("Modal Sheet Content"),
                )
                .environment(Theme::dark())
            }
        }

        let mut sim = HeadlessSimulator::new(Demo, Size::new(800.0, 600.0));
        let png = with_animation(Animation::spring(), || {
            sim.render_png().expect("render demo")
        });
        assert_eq!(&png[0..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn v02_widgets_editorsurface_tabbar_palette_progress_theme_work() {
        use rustui_core::{Cx, KeyCode, Modifiers, State, View, ViewExt};
        use rustui_platform::HeadlessSimulator;
        use rustui_text::{
            DecorationKind, EditorDecoration, FoldRange, GutterDiffKind, LineDiffMarker,
            SyntaxTokenSpan,
        };

        let toml = Theme::high_contrast().to_toml_string();
        let parsed = Theme::from_toml_str(&toml).expect("theme toml roundtrip");
        assert!(parsed.is_dark);
        assert_eq!(parsed, Theme::high_contrast());
        assert!(Theme::light().to_toml_string().contains("is_dark = false"));

        struct Showcase;
        impl View for Showcase {
            fn body(self, cx: &mut Cx) -> impl View {
                let mut initial = EditorBufferState::from_text("fn main() {\n    let x = 1;\n}");
                initial.syntax_spans = vec![SyntaxTokenSpan {
                    start_byte: 0,
                    end_byte: 2,
                    color: rustui_core::Color::accent(),
                }];
                initial.decorations = vec![EditorDecoration {
                    start_byte: 20,
                    end_byte: 21,
                    kind: DecorationKind::InlayHint {
                        label: ": i32".into(),
                    },
                    message: None,
                }];
                initial.folds = vec![FoldRange {
                    start_line: 0,
                    end_line: 2,
                    collapsed: false,
                }];
                initial.line_diffs = vec![LineDiffMarker {
                    line: 1,
                    kind: GutterDiffKind::Modified,
                }];
                initial.show_minimap = true;
                initial.toggle_breakpoint(1);

                let ed: State<EditorBufferState> = cx.state(initial);
                let tabs = cx.state(vec![
                    TabBarItem::new("main", "main.rs").pinned(true),
                    TabBarItem::new("lib", "lib.rs").dirty(true),
                    TabBarItem::new("mod", "mod.rs"),
                ]);
                let active_tab = cx.state(0usize);
                let palette_open = cx.state(true);
                let ran_cmd = cx.state(false);
                let expanded = cx.state(false);
                let term_lines = cx.state(vec![TerminalLine {
                    id: 1,
                    text: "cargo check".into(),
                    tone: TerminalLineTone::Prompt,
                }]);

                let ed_for_status = ed;
                let term_append = term_lines;
                let ran_set = ran_cmd;

                CommandPalette::new(
                    palette_open.binding(),
                    vec![CommandItem::new(
                        "build",
                        "Run Cargo Build",
                        "Ctrl+B",
                        move || ran_set.set(true),
                    )],
                    VStack::new(
                        8,
                        (
                            TabBar::new(tabs.binding(), active_tab.binding()),
                            HStack::new(
                                12,
                                (
                                    ProgressView::determinate(0.65).label("Indexing"),
                                    Gauge::new("CPU", 42.0, 0.0, 100.0),
                                    ViewThatFits::new((
                                        Text::new("Wide Desktop Layout Title Text"),
                                        Text::new("Compact"),
                                    )),
                                ),
                            ),
                            DisclosureGroup::new("Workspace Details", expanded.binding(), || {
                                Text::new("Expanded info")
                            }),
                            EditorSurface::new(ed.binding()).accessibility_label("Code Editor"),
                            TerminalView::new(term_lines.binding(), move |cmd| {
                                term_append.update(|v| {
                                    let id = v.last().map(|l| l.id + 1).unwrap_or(1);
                                    v.push(TerminalLine {
                                        id,
                                        text: format!("ok: {cmd}"),
                                        tone: TerminalLineTone::Success,
                                    });
                                });
                            }),
                            StatusBar::new((
                                Text::new(move || ed_for_status.get().status_summary()),
                                Spacer::new(),
                                Text::new("UTF-8"),
                            )),
                        ),
                    ),
                )
            }
        }

        let mut sim = HeadlessSimulator::new(Showcase, Size::new(1024.0, 720.0));
        sim.step_frame();
        assert!(sim.find_node_by_label("Command Palette").is_some());
        let run_btn = sim
            .find_node_by_label("Run Cargo Build   [Ctrl+B]")
            .expect("command item");
        sim.click_node(run_btn);
        assert!(sim.find_node_by_label("Command Palette").is_none());

        let tab2 = sim.find_node_by_label("lib.rs •").expect("second tab");
        sim.click_node(tab2);
        let close3 = sim.find_node_by_label("Close tab 3").expect("close tab 3");
        sim.click_node(close3);
        let disc = sim
            .find_node_by_label("▸ Workspace Details")
            .expect("disclosure");
        sim.click_node(disc);
        assert!(sim.find_node_by_label("Expanded info").is_some());

        let editor_id = sim.find_node_by_label("Code Editor").expect("editor node");
        sim.click_node(editor_id);
        sim.type_text("// hi");
        sim.press_key(
            KeyCode::ArrowDown,
            Modifiers {
                ctrl: true,
                alt: true,
                ..Modifiers::NONE
            },
        );
        sim.type_text("!");
        let png = sim.render_png().expect("render v0.2 showcase");
        assert_eq!(&png[0..8], b"\x89PNG\r\n\x1a\n");
    }
}
