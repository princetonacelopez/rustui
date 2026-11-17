//! Dashboard showcase demonstrating `NavigationStack`, `TabView`, `Table`, `LazyVStack` (1M rows),
//! `Canvas` chart, `Slider`, `Sheet`, `Popover`, `Menu`, and light/dark theming.

use rustui::prelude::*;

struct DashboardApp;

impl View for DashboardApp {
    fn body(self, cx: &mut Cx) -> impl View {
        let tab = cx.state(0usize);
        let nav = cx.state(0usize);
        let gain = cx.state(0.72f32);
        let selected_row = cx.state(None::<usize>);
        let show_sheet = cx.state(false);
        let show_popover = cx.state(false);

        Sheet::new(
            show_sheet.binding(),
            VStack::new(
                0,
                (
                    Toolbar::new((
                        Text::new("RustUI Telemetry Dashboard").font(Font::headline()),
                        Spacer::new(),
                        Popover::new(
                            show_popover.binding(),
                            Button::new("Info", move || show_popover.update(|p| *p = !*p)),
                            || Text::new("60 fps GPU-rendered RustUI showcase"),
                        ),
                        Menu::new(
                            "Window",
                            vec![MenuItem::new("Open Report Sheet", move || {
                                show_sheet.set(true);
                            })],
                        ),
                    )),
                    TabView::new(
                        tab.binding(),
                        vec![
                            TabItem::new("Overview", move || {
                                NavigationStack::new(
                                    nav.binding(),
                                    vec![
                                        NavigationPage::new("Metrics", move || {
                                            VStack::new(
                                                12,
                                                (
                                                    HStack::new(
                                                        12,
                                                        (
                                                            Text::new("Amplitude:"),
                                                            Slider::new(gain.binding(), 0.1, 1.0),
                                                            Text::new(move || {
                                                                format!(
                                                                    "{:.0}%",
                                                                    gain.get() * 100.0
                                                                )
                                                            })
                                                            .font(Font::caption())
                                                            .color(Color::secondary()),
                                                            Spacer::new(),
                                                            Button::new("Details ›", move || {
                                                                nav.set(1);
                                                            }),
                                                        ),
                                                    ),
                                                    Canvas::new(move |rect, dl| {
                                                        dl.fill_rounded_rect(
                                                            rect,
                                                            CornerRadii::all(8.0),
                                                            Color::from_hex(0x0f172a),
                                                        );
                                                        let bars = 12usize;
                                                        let bar_w =
                                                            (rect.width - 32.0) / (bars as f32);
                                                        let g = gain.get();
                                                        for i in 0..bars {
                                                            let h = (rect.height - 28.0)
                                                                * (((i as f32 * 0.6).sin() * 0.4
                                                                    + 0.55)
                                                                    * g)
                                                                    .clamp(0.1, 0.95);
                                                            dl.fill_rounded_rect(
                                                                Rect::new(
                                                                    rect.x
                                                                        + 16.0
                                                                        + i as f32 * bar_w
                                                                        + 3.0,
                                                                    rect.max_y() - 12.0 - h,
                                                                    (bar_w - 6.0).max(4.0),
                                                                    h,
                                                                ),
                                                                CornerRadii::all(3.0),
                                                                Color::from_hex(0x3b82f6),
                                                            );
                                                        }
                                                    })
                                                    .frame(Frame::new().height(220)),
                                                ),
                                            )
                                            .padding(16)
                                        }),
                                        NavigationPage::new("Node Table", move || {
                                            Table::new(
                                                vec![
                                                    TableColumn::fixed("ID", 70.0),
                                                    TableColumn::flexible(
                                                        "Subsystem",
                                                        140.0,
                                                        260.0,
                                                    ),
                                                    TableColumn::fixed("Budget", 100.0),
                                                ],
                                                20,
                                                selected_row.binding(),
                                                |r, c| match c {
                                                    0 => format!("#{r}"),
                                                    1 => format!("Layer Node {r}"),
                                                    _ => "0.4 ms".into(),
                                                },
                                            )
                                            .padding(12)
                                        }),
                                    ],
                                )
                            }),
                            TabItem::new("1M Virtual List", || {
                                LazyVStack::new(1_000_000, 26, |idx| {
                                    Text::new(format!("Virtual Row #{idx}"))
                                })
                                .with_pool_updater(|idx, node| {
                                    format_virtual_row_text(node, "Virtual Row #", idx);
                                })
                                .padding(12)
                            }),
                        ],
                    ),
                ),
            ),
            move || {
                VStack::new(
                    12,
                    (
                        Text::new("Telemetry Summary").font(Font::title()),
                        Text::new("All GPU, text, layout, and AccessKit pipelines nominal."),
                        Button::new("Close", move || show_sheet.set(false)),
                    ),
                )
            },
        )
    }
}

fn main() -> Result<(), String> {
    tracing_subscriber::fmt::try_init().ok();
    App::new(DashboardApp)
        .title("RustUI Dashboard")
        .size(960.0, 640.0)
        .run()
}
