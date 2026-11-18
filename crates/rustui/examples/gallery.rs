//! RustUI 0.2 Widget Gallery showcasing all 0.2 widgets, macros, theming, and scaling.

use rustui::prelude::*;

#[derive(Observable, Clone, Default)]
struct GalleryModel {
    theme_mode: usize,
    text_scale: f32,
    split_ratio: f32,
    nav_collapsed: bool,
    active_tab: usize,
    palette_open: bool,
    alert_open: bool,
    details_expanded: bool,
    progress: f32,
    selected_row: Option<usize>,
    last_action: String,
    tabs: Vec<TabBarItem>,
    editor: EditorBufferState,
    terminal_lines: Vec<TerminalLine>,
    picker_style_idx: usize,
    stepper_count: f32,
    highlight_color: Color,
    release_date: String,
    compact_controls: bool,
}

fn initial_editor_state() -> EditorBufferState {
    let source = "\
// Welcome to RustUI 0.2 EditorSurface
#[derive(Observable, Clone, Default)]
pub struct WorkspaceState {
    pub active_file: String,
    pub dirty_count: usize,
}

pub fn greeting(user: &str) -> String {
    format!(\"Hello, {user}!\")
}
";
    let mut state = EditorBufferState::from_text(source);
    let span_for = |needle: &str, color: Color| {
        let start = source.find(needle).unwrap_or(0);
        SyntaxTokenSpan {
            start_byte: start,
            end_byte: start + needle.len(),
            color,
        }
    };
    state.syntax_spans = vec![
        span_for(
            "// Welcome to RustUI 0.2 EditorSurface",
            Color::from_hex(0x64748b),
        ),
        span_for("pub struct", Color::from_hex(0x818cf8)),
        span_for("pub fn", Color::from_hex(0x818cf8)),
        span_for("\"Hello, {user}!\"", Color::from_hex(0x4ade80)),
    ];
    let lint_start = source.find("active_file").unwrap_or(0);
    let hint_start = source.find("-> String").unwrap_or(0);
    state.decorations = vec![
        EditorDecoration {
            start_byte: hint_start,
            end_byte: hint_start + 2,
            kind: DecorationKind::InlayHint {
                label: " /* pure */".into(),
            },
            message: Some("Inferred return annotation".into()),
        },
        EditorDecoration {
            start_byte: lint_start,
            end_byte: lint_start + "active_file".len(),
            kind: DecorationKind::SquiggleWarning,
            message: Some("Example lint: consider semantic identifier".into()),
        },
    ];
    state.folds = vec![
        FoldRange {
            start_line: 2,
            end_line: 5,
            collapsed: false,
        },
        FoldRange {
            start_line: 7,
            end_line: 9,
            collapsed: false,
        },
    ];
    state.line_diffs = vec![
        LineDiffMarker {
            line: 3,
            kind: GutterDiffKind::Added,
        },
        LineDiffMarker {
            line: 8,
            kind: GutterDiffKind::Modified,
        },
    ];
    state.toggle_breakpoint(8);
    state.show_minimap = true;
    state
}

#[derive(Debug, Clone, Copy, Default)]
struct GalleryApp;

impl View for GalleryApp {
    fn body(self, cx: &mut Cx) -> impl View {
        let model = cx.model(GalleryModel {
            theme_mode: 1,
            text_scale: 1.0,
            split_ratio: 0.25,
            nav_collapsed: false,
            active_tab: 0,
            palette_open: false,
            alert_open: false,
            details_expanded: true,
            progress: 0.72,
            selected_row: Some(1),
            last_action: "Ready".into(),
            tabs: vec![
                TabBarItem::new("editor", "EditorSurface").pinned(true),
                TabBarItem::new("metrics", "Data & Gauges").dirty(true),
                TabBarItem::new("macos", "macOS Controls"),
                TabBarItem::new("terminal", "Terminal & Theme"),
            ],
            editor: initial_editor_state(),
            picker_style_idx: 0,
            stepper_count: 3.0,
            highlight_color: Color::from_hex(0x007AFF),
            release_date: "2026-11-18".into(),
            compact_controls: false,
            terminal_lines: vec![
                TerminalLine {
                    id: 1,
                    text: "rustui doctor --verify".into(),
                    tone: TerminalLineTone::Prompt,
                },
                TerminalLine {
                    id: 2,
                    text: "✓ Renderer: WGPU hardware/software pipeline ready".into(),
                    tone: TerminalLineTone::Success,
                },
                TerminalLine {
                    id: 3,
                    text: "✓ AccessKit bridge active (AlertDialog, Meter, Status)".into(),
                    tone: TerminalLineTone::Accent,
                },
            ],
        });

        let runtime = cx.runtime().clone();
        cx.effect(move || {
            let mode = model.theme_mode();
            let scale = model.text_scale();
            runtime.set_high_contrast(mode == 2);
            runtime.set_text_scale_factor(scale);
        });

        let preview_entry = gallery_preview_spec();
        model.set_last_action(format!("Preview ready: {}", preview_entry.0));

        Switch::new(move || model.theme_mode())
            .case(0usize, move || {
                let theme = Theme::light();
                let palette = theme.to_palette();
                GalleryShell { model, theme }
                    .environment(Theme::light())
                    .environment(palette)
            })
            .case(1usize, move || {
                let theme = Theme::dark();
                let palette = theme.to_palette();
                GalleryShell { model, theme }
                    .environment(Theme::dark())
                    .environment(palette)
            })
            .case(2usize, move || {
                let theme = Theme::high_contrast();
                let palette = theme.to_palette();
                GalleryShell { model, theme }
                    .environment(Theme::high_contrast())
                    .environment(palette)
            })
            .default(move || {
                let theme = Theme::dark();
                let palette = theme.to_palette();
                GalleryShell { model, theme }
                    .environment(Theme::dark())
                    .environment(palette)
            })
    }
}

#[derive(Debug)]
struct GalleryShell {
    model: GalleryModelObservable,
    theme: Theme,
}

impl View for GalleryShell {
    fn body(self, _cx: &mut Cx) -> impl View {
        let model = self.model;
        let theme = self.theme;
        let theme_name = match model.theme_mode_untracked() {
            0 => "Light",
            1 => "Dark",
            _ => "High Contrast",
        };
        let toml_preview: String = theme
            .to_toml_string()
            .lines()
            .take(12)
            .collect::<Vec<_>>()
            .join("\n");

        let tree_roots = vec![
            TreeItemData {
                id: "src".into(),
                label: "Workspace Crate".into(),
                expanded: true,
                children: vec![
                    TreeItemData {
                        id: "lib".into(),
                        label: "src/lib.rs".into(),
                        expanded: false,
                        children: vec![],
                    },
                    TreeItemData {
                        id: "main".into(),
                        label: "src/main.rs".into(),
                        expanded: false,
                        children: vec![],
                    },
                    TreeItemData {
                        id: "theme".into(),
                        label: "theme.toml".into(),
                        expanded: false,
                        children: vec![],
                    },
                ],
            },
            TreeItemData {
                id: "tests".into(),
                label: "Verification".into(),
                expanded: true,
                children: vec![TreeItemData {
                    id: "e2e".into(),
                    label: "examples_e2e.rs".into(),
                    expanded: false,
                    children: vec![],
                }],
            },
        ];

        let commands = vec![
            CommandItem::new(
                "theme.light",
                "Theme: Switch to Light",
                "Ctrl+Alt+L",
                move || {
                    model.set_theme_mode(0);
                    model.set_last_action("Switched to Light theme".into());
                },
            ),
            CommandItem::new(
                "theme.dark",
                "Theme: Switch to Dark",
                "Ctrl+Alt+D",
                move || {
                    model.set_theme_mode(1);
                    model.set_last_action("Switched to Dark theme".into());
                },
            ),
            CommandItem::new(
                "theme.hc",
                "Theme: Switch to High Contrast",
                "Ctrl+Alt+H",
                move || {
                    model.set_theme_mode(2);
                    model.set_last_action("Switched to High Contrast theme".into());
                },
            ),
            CommandItem::new(
                "scale.200",
                "Scale: Set 200% Text Scaling",
                "Ctrl+2",
                move || {
                    model.set_text_scale(2.0);
                    model.set_last_action("Text scale set to 200%".into());
                },
            ),
            CommandItem::new("scale.100", "Scale: Reset 100%", "Ctrl+0", move || {
                model.set_text_scale(1.0);
                model.set_last_action("Text scale reset to 100%".into());
            }),
            CommandItem::new(
                "tab.metrics",
                "View: Open Data & Gauges",
                "Ctrl+2",
                move || {
                    model.set_active_tab(1);
                },
            ),
        ];

        let nav_panel = VStack::new(
            10,
            (
                HStack::new(
                    6,
                    (
                        Text::new("Navigator").font(Font::headline()),
                        Spacer::new(),
                        Button::new("Collapse", move || model.update_nav_collapsed(|c| *c = !*c))
                            .style(ButtonStyle::plain(&theme)),
                    ),
                ),
                OutlineGroup::new(tree_roots, move |id| {
                    model.set_last_action(format!("Toggled tree node: {id}"));
                }),
                DisclosureGroup::new("0.2 Diagnostics", model.details_expanded_binding(), || {
                    VStack::new(
                        4,
                        (
                            Text::new("Observable: per-field signals").font(Font::caption()),
                            Text::new("Glyph LRU: budgeted atlas").font(Font::caption()),
                            Text::new("AccessKit: Meter/Status/Alert").font(Font::caption()),
                        ),
                    )
                    .alignment(HorizontalAlignment::Leading)
                }),
                Spacer::new(),
            ),
        )
        .alignment(HorizontalAlignment::Leading)
        .padding(12)
        .background(Color::surface())
        .frame(
            Frame::new()
                .max_width(f32::INFINITY)
                .max_height(f32::INFINITY),
        );

        let theme_tab = theme.clone();
        let toml_for_tab = toml_preview;
        let main_pane = VStack::new(
            0,
            (
                TabBar::new(model.tabs_binding(), model.active_tab_binding()),
                Switch::new(move || model.active_tab())
                    .case(0usize, move || {
                        EditorSurface::new(model.editor_binding())
                            .accessibility_label("Gallery Code Editor")
                    })
                    .case(1usize, {
                        let t_alert = theme_tab.clone();
                        move || {
                            VStack::new(
                                12,
                                (
                                    Text::new("Progress, Gauges & Adaptive Layout")
                                        .font(Font::headline()),
                                    HStack::new(
                                        16,
                                        (
                                            ProgressView::determinate(model.progress())
                                                .label("Indexing Crate"),
                                            ProgressView::indeterminate().label("Analyzer"),
                                            Gauge::new("Memory Budget", 38.0, 0.0, 100.0),
                                        ),
                                    ),
                                    ViewThatFits::new((
                                        Text::new("ViewThatFits selected: Full Wide Status Banner")
                                            .color(Color::accent()),
                                        Text::new("Compact Banner"),
                                    )),
                                    Table::new(
                                        vec![
                                            TableColumn::fixed("Crate", 150.0),
                                            TableColumn::flexible("Capability", 160.0, 320.0),
                                            TableColumn::fixed("Status", 90.0),
                                        ],
                                        4,
                                        model.selected_row_binding(),
                                        |r, c| match (r, c) {
                                            (0, 0) => "rustui-macros".into(),
                                            (0, 1) => "#[derive(Observable)], #[preview]".into(),
                                            (0, 2) => "Active".into(),
                                            (1, 0) => "rustui-text".into(),
                                            (1, 1) => "Multi-cursor & UTF-16 LSP".into(),
                                            (1, 2) => "Ready".into(),
                                            (2, 0) => "rustui-render".into(),
                                            (2, 1) => "Atlas LRU + InspectorServer".into(),
                                            (2, 2) => "Active".into(),
                                            (3, 0) => "rustui-widgets".into(),
                                            (3, 1) => "EditorSurface, SplitView, TabBar".into(),
                                            _ => "Ready".into(),
                                        },
                                    )
                                    .frame(Frame::new().height(200.0)),
                                    HStack::new(
                                        8,
                                        (
                                            Button::new("Show Modal Alert", move || {
                                                model.set_alert_open(true);
                                            })
                                            .style(ButtonStyle::primary(&t_alert)),
                                            Button::new("+10% Progress", move || {
                                                model.update_progress(|p| {
                                                    *p = (*p + 0.1).min(1.0);
                                                });
                                            })
                                            .style(ButtonStyle::secondary(&t_alert)),
                                        ),
                                    ),
                                    Spacer::new(),
                                ),
                            )
                            .alignment(HorizontalAlignment::Leading)
                            .padding(16)
                        }
                    })
                    .case(2usize, {
                        let t_form = theme_tab.clone();
                        move || {
                            Form::new((
                                Section::new(
                                    "macOS SwiftUI Controls",
                                    (
                                        LabeledContent::new(
                                            "Layout Mode",
                                            Picker::new(
                                                model.picker_style_idx_binding(),
                                                vec![
                                                    "List".into(),
                                                    "Grid".into(),
                                                    "Inspector".into(),
                                                ],
                                            )
                                            .picker_style(PickerStyle::Segmented),
                                        ),
                                        Stepper::new(
                                            "Retry Limit",
                                            model.stepper_count_binding(),
                                            1.0..=10.0,
                                        ),
                                        ColorPicker::new(
                                            "Accent Well",
                                            model.highlight_color_binding(),
                                        ),
                                        DatePicker::new(
                                            "Release Target",
                                            model.release_date_binding(),
                                        ),
                                    ),
                                )
                                .footer("Settings follow macOS control sizing & keyboard rules."),
                                GroupBox::new(
                                    "Actions & Documentation",
                                    HStack::new(
                                        10,
                                        (
                                            Button::new("Save Changes", move || {
                                                model.set_last_action("Saved macOS form".into());
                                            })
                                            .style(ButtonStyle::primary(&t_form))
                                            .badge("NEW")
                                            .help("Saves all preferences"),
                                            Toggle::new(
                                                "Compact Controls",
                                                model.compact_controls_binding(),
                                            ),
                                            Link::new(
                                                "SwiftUI Reference",
                                                "https://developer.apple.com/documentation/swiftui",
                                            )
                                            .on_open(
                                                move |url| {
                                                    model.set_last_action(format!("Link: {url}"));
                                                },
                                            ),
                                        ),
                                    ),
                                ),
                            ))
                        }
                    })
                    .default(move || {
                        let toml_txt = toml_for_tab.clone();
                        VStack::new(
                            10,
                            (
                                TerminalView::new(model.terminal_lines_binding(), move |cmd| {
                                    model.update_terminal_lines(|v| {
                                        let id = v.last().map(|l| l.id + 1).unwrap_or(1);
                                        v.push(TerminalLine {
                                            id,
                                            text: format!("executed: {cmd}"),
                                            tone: TerminalLineTone::Output,
                                        });
                                    });
                                    model.set_last_action(format!("Terminal ran: {cmd}"));
                                })
                                .frame(Frame::new().height(260.0)),
                                Text::new("Active Theme TOML Snapshot").font(Font::headline()),
                                Text::new(toml_txt)
                                    .font(Font::monospace())
                                    .color(Color::secondary()),
                                Spacer::new(),
                            ),
                        )
                        .alignment(HorizontalAlignment::Leading)
                        .padding(16)
                    }),
            ),
        )
        .frame(
            Frame::new()
                .max_width(f32::INFINITY)
                .max_height(f32::INFINITY),
        );

        let theme_tb = theme;
        Alert::new(
            model.alert_open_binding(),
            "RustUI 0.2 Verification Alert",
            "Accessible AlertDialog role, custom actions, and TOML theming are active.",
            vec![
                AlertButton::new("Acknowledge", move || {
                    model.set_last_action("Alert acknowledged".into());
                }),
                AlertButton::destructive("Dismiss", move || {}),
            ],
            CommandPalette::new(
                model.palette_open_binding(),
                commands,
                VStack::new(
                    0,
                    (
                        Toolbar::new((
                            Text::new("RustUI 0.2 Gallery").font(Font::headline()),
                            EmptyView
                                .frame(Frame::fixed(1.0, 18.0))
                                .background(Color::border()),
                            Button::new("Light", move || model.set_theme_mode(0))
                                .style(ButtonStyle::plain(&theme_tb)),
                            Button::new("Dark", move || model.set_theme_mode(1))
                                .style(ButtonStyle::plain(&theme_tb)),
                            Button::new("High Contrast", move || model.set_theme_mode(2))
                                .style(ButtonStyle::plain(&theme_tb)),
                            Spacer::new(),
                            Button::new("100%", move || model.set_text_scale(1.0))
                                .style(ButtonStyle::secondary(&theme_tb)),
                            Button::new("200%", move || model.set_text_scale(2.0))
                                .style(ButtonStyle::secondary(&theme_tb)),
                            Button::new("Commands (Ctrl+P)", move || model.set_palette_open(true))
                                .style(ButtonStyle::primary(&theme_tb)),
                        )),
                        SplitView::horizontal(model.split_ratio_binding(), nav_panel, main_pane)
                            .collapsed(model.nav_collapsed_binding())
                            .frame(
                                Frame::new()
                                    .max_width(f32::INFINITY)
                                    .max_height(f32::INFINITY),
                            ),
                        StatusBar::new((
                            Text::new(move || model.editor().status_summary()),
                            EmptyView
                                .frame(Frame::fixed(1.0, 14.0))
                                .background(Color::border()),
                            Text::new(format!("Theme: {theme_name}")),
                            EmptyView
                                .frame(Frame::fixed(1.0, 14.0))
                                .background(Color::border()),
                            Text::new(move || format!("Scale: {:.0}%", model.text_scale() * 100.0)),
                            Spacer::new(),
                            Text::new(move || model.last_action()),
                        )),
                    ),
                )
                .frame(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .max_height(f32::INFINITY),
                ),
            ),
        )
    }
}

#[preview("Gallery Default")]
fn gallery() -> AnyView {
    AnyView::new(GalleryApp)
}

fn main() {
    if let Err(err) = App::new(GalleryApp)
        .title("RustUI 0.2 Widget Gallery")
        .size(1200.0, 760.0)
        .run()
    {
        eprintln!("gallery error: {err}");
    }
}
