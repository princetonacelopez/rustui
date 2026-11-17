//! Dashboard showcase demonstrating `NavigationStack`, `TabView`, `Table`, `LazyVStack` (1M rows),
//! `Canvas` shapes/gradients/Bezier paths, SVG & RGBA images, spring animations, touch pinch,
//! system `reduced_motion` support, advanced forms, `Popover`, `Menu`, and `Sheet`.

use rustui::prelude::*;

const SAMPLE_SVG_ICON: &str = r##"
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">
  <defs>
    <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#2563eb"/>
      <stop offset="100%" stop-color="#7c3aed"/>
    </linearGradient>
  </defs>
  <rect x="4" y="4" width="56" height="56" rx="14" fill="url(#bg)"/>
  <circle cx="24" cy="26" r="9" fill="#38bdf8"/>
  <path d="M16 44 L29 29 L38 38 L48 22" stroke="#ffffff" stroke-width="4" fill="none" stroke-linecap="round" stroke-linejoin="round"/>
</svg>
"##;

fn make_procedural_rgba(size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let fx = x as f32 / size as f32;
            let fy = y as f32 / size as f32;
            let checker = ((x / 8) + (y / 8)).is_multiple_of(2);
            let r = if checker {
                (fx * 200.0 + 40.0) as u8
            } else {
                30
            };
            let g = (fy * 190.0 + 50.0) as u8;
            let b = ((1.0 - fx) * 180.0 + 75.0) as u8;
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    pixels
}

struct DashboardApp;

impl View for DashboardApp {
    fn body(self, cx: &mut Cx) -> impl View {
        let tab = cx.state(0usize);
        let nav = cx.state(0usize);
        let gain = cx.state(0.72f32);
        let selected_row = cx.state(None::<usize>);
        let show_sheet = cx.state(false);
        let show_popover = cx.state(false);

        // Graphics & Animation states
        let pulse_phase = cx.state(0usize);
        let card_accent = cx.state(false);
        let zoom_scale = cx.state(1.0f32);
        let reduced_motion_flag = cx.state(cx.runtime().is_reduced_motion());
        let rt_rm = cx.runtime().clone();
        let reduced_motion_binding = Binding::new(
            move || reduced_motion_flag.get(),
            move |enabled| {
                rt_rm.set_reduced_motion(enabled);
                reduced_motion_flag.set(enabled);
            },
        );

        // Advanced Form states
        let form_name = cx.state(String::new());
        let form_key = cx.state(String::new());
        let form_env = cx.state(0usize);
        let form_workers = cx.state(4.0f32);
        let form_autodeploy = cx.state(true);
        let form_terms = cx.state(false);
        let form_submitted = cx.state(String::new());

        let proc_rgba = make_procedural_rgba(48);

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
                                                            Color::from_hex(0x0b1220),
                                                        );
                                                        dl.stroke_rounded_rect(
                                                            rect,
                                                            CornerRadii::all(8.0),
                                                            1.0,
                                                            Color::from_hex(0x334155),
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
                            TabItem::new("Graphics & Shapes", move || {
                                let rgba_copy = proc_rgba.clone();
                                VStack::new(
                                    14,
                                    (
                                        HStack::new(
                                            10,
                                            (
                                                Button::new("Animate Shapes", move || {
                                                    with_animation(Animation::spring(), || {
                                                        card_accent.update(|a| *a = !*a);
                                                        pulse_phase.update(|p| *p = (*p + 1) % 6);
                                                    });
                                                }),
                                                Toggle::new(
                                                    "Reduced Motion",
                                                    reduced_motion_binding.clone(),
                                                ),
                                                Spacer::new(),
                                                Button::new("Zoom -", move || {
                                                    zoom_scale.update(|z| {
                                                        *z = (*z - 0.15).clamp(0.6, 1.8)
                                                    });
                                                }),
                                                Text::new(move || {
                                                    format!("Zoom: {:.0}%", zoom_scale.get() * 100.0)
                                                })
                                                .font(Font::caption())
                                                .color(Color::secondary()),
                                                Button::new("Zoom +", move || {
                                                    zoom_scale.update(|z| {
                                                        *z = (*z + 0.15).clamp(0.6, 1.8)
                                                    });
                                                }),
                                            ),
                                        ),
                                        HStack::new(
                                            16,
                                            (
                                                Canvas::new(move |rect, dl| {
                                                    let step = pulse_phase.get() as f32;
                                                    let z = zoom_scale.get();
                                                    let accent = if card_accent.get() {
                                                        Color::from_hex(0x8b5cf6)
                                                    } else {
                                                        Color::from_hex(0x2563eb)
                                                    };
                                                    dl.draw_gradient(
                                                        rect,
                                                        CornerRadii::all(12.0),
                                                        GradientKind::Linear {
                                                            start: Point::new(0.0, 0.0),
                                                            end: Point::new(rect.width, rect.height),
                                                            stops: vec![
                                                                GradientStop {
                                                                    offset: 0.0,
                                                                    color: Color::from_hex(0x0f172a),
                                                                },
                                                                GradientStop {
                                                                    offset: 1.0,
                                                                    color: Color::from_hex(0x1e1b4b),
                                                                },
                                                            ],
                                                        },
                                                    );
                                                    let orb_r = (38.0 * z).min(rect.height * 0.35);
                                                    dl.draw_gradient(
                                                        Rect::new(
                                                            rect.x + 28.0 + step * 18.0,
                                                            rect.y + 24.0,
                                                            orb_r * 2.0,
                                                            orb_r * 2.0,
                                                        ),
                                                        CornerRadii::all(orb_r),
                                                        GradientKind::Radial {
                                                            center: Point::new(orb_r, orb_r),
                                                            radius: orb_r,
                                                            stops: vec![
                                                                GradientStop {
                                                                    offset: 0.0,
                                                                    color: Color::from_hex(0x38bdf8),
                                                                },
                                                                GradientStop {
                                                                    offset: 1.0,
                                                                    color: accent.with_alpha(0.0),
                                                                },
                                                            ],
                                                        },
                                                    );
                                                    // Vector Bezier wave path rendered via Vello
                                                    let base_y = rect.max_y() - 36.0;
                                                    let amp = (28.0 + step * 6.0) * z;
                                                    dl.draw_path(
                                                        vec![
                                                            PathOp::MoveTo(Point::new(
                                                                rect.x + 20.0,
                                                                base_y,
                                                            )),
                                                            PathOp::CubicTo {
                                                                ctrl1: Point::new(
                                                                    rect.x + rect.width * 0.3,
                                                                    base_y - amp,
                                                                ),
                                                                ctrl2: Point::new(
                                                                    rect.x + rect.width * 0.65,
                                                                    base_y + amp * 0.65,
                                                                ),
                                                                to: Point::new(
                                                                    rect.max_x() - 20.0,
                                                                    base_y - amp * 0.25,
                                                                ),
                                                            },
                                                        ],
                                                        None,
                                                        Some((3.5, accent)),
                                                    );
                                                    // Outlined & filled badge shape
                                                    dl.stroke_rounded_rect(
                                                        Rect::new(
                                                            rect.max_x() - 104.0,
                                                            rect.y + 18.0,
                                                            82.0,
                                                            30.0,
                                                        ),
                                                        CornerRadii::all(8.0),
                                                        1.5,
                                                        Color::from_hex(0x38bdf8),
                                                    );
                                                })
                                                .frame(
                                                    Frame::new()
                                                        .min_width(380.0)
                                                        .max_width(f32::INFINITY)
                                                        .height(220.0),
                                                )
                                                .on_pinch(move |ev| {
                                                    zoom_scale.update(|z| {
                                                        *z = (*z + ev.scale_delta).clamp(0.6, 1.8);
                                                    });
                                                }),
                                                VStack::new(
                                                    12,
                                                    (
                                                        VStack::new(
                                                            6,
                                                            (
                                                                Image::from_svg(SAMPLE_SVG_ICON)
                                                                    .frame(Frame::fixed(56.0, 56.0))
                                                                    .corner_radius(10),
                                                                Text::new("Vector SVG")
                                                                    .font(Font::caption()),
                                                            ),
                                                        ),
                                                        VStack::new(
                                                            6,
                                                            (
                                                                Image::from_rgba(48, 48, rgba_copy)
                                                                    .frame(Frame::fixed(56.0, 56.0))
                                                                    .corner_radius(10),
                                                                Text::new("RGBA Texture")
                                                                    .font(Font::caption()),
                                                            ),
                                                        ),
                                                    ),
                                                )
                                                .padding(12)
                                                .background(Color::surface())
                                                .border(1, Color::border())
                                                .corner_radius(10),
                                            ),
                                        ),
                                        Text::new(move || {
                                            if card_accent.get() {
                                                "Animated Theme Card: Accent Violet (Spring Transition Active)".to_owned()
                                            } else {
                                                "Animated Theme Card: Accent Cobalt (Click 'Animate Shapes' or Pinch Canvas)".to_owned()
                                            }
                                        })
                                        .padding(12)
                                        .background_reactive(move || {
                                            if card_accent.get() {
                                                Color::from_hex(0x7c3aed).with_alpha(0.22)
                                            } else {
                                                Color::surface_elevated()
                                            }
                                        })
                                        .border(1, Color::border())
                                        .corner_radius(10)
                                        .animation(Animation::ease_in_out(0.25), card_accent),
                                    ),
                                )
                                .alignment(HorizontalAlignment::Leading)
                                .padding(16)
                            }),
                            TabItem::new("Advanced Form", move || {
                                VStack::new(
                                    12,
                                    (
                                        HStack::new(
                                            10,
                                            (
                                                Text::new("Full Name:")
                                                    .frame(Frame::new().width(110.0)),
                                                TextField::new(
                                                    "Enter full name...",
                                                    form_name.binding(),
                                                ),
                                            ),
                                        ),
                                        HStack::new(
                                            10,
                                            (
                                                Text::new("API Token:")
                                                    .frame(Frame::new().width(110.0)),
                                                SecureField::new(
                                                    "At least 4 characters...",
                                                    form_key.binding(),
                                                ),
                                            ),
                                        ),
                                        HStack::new(
                                            10,
                                            (
                                                Text::new("Environment:")
                                                    .frame(Frame::new().width(110.0)),
                                                Picker::new(
                                                    form_env.binding(),
                                                    vec![
                                                        "Production".into(),
                                                        "Staging".into(),
                                                        "Development".into(),
                                                    ],
                                                ),
                                                Spacer::new(),
                                                Toggle::new(
                                                    "Auto-Deploy",
                                                    form_autodeploy.binding(),
                                                ),
                                            ),
                                        ),
                                        HStack::new(
                                            10,
                                            (
                                                Text::new("Concurrency:")
                                                    .frame(Frame::new().width(110.0)),
                                                Slider::new(form_workers.binding(), 1.0, 16.0)
                                                    .step(1.0),
                                                Text::new(move || {
                                                    format!(
                                                        "{:.0} workers",
                                                        form_workers.get().round()
                                                    )
                                                })
                                                .font(Font::caption())
                                                .color(Color::secondary()),
                                            ),
                                        ),
                                        Checkbox::new(
                                            "Accept deployment security policy",
                                            form_terms.binding(),
                                        ),
                                        HStack::new(
                                            10,
                                            (
                                                Button::new("Submit Config", move || {
                                                    let n = form_name.get_untracked();
                                                    let k = form_key.get_untracked();
                                                    let e = match form_env.get_untracked() {
                                                        1 => "Staging",
                                                        2 => "Development",
                                                        _ => "Production",
                                                    };
                                                    let w = form_workers.get_untracked().round()
                                                        as u32;
                                                    let auto = form_autodeploy.get_untracked();
                                                    if n.trim().is_empty()
                                                        || k.len() < 4
                                                        || !form_terms.get_untracked()
                                                    {
                                                        form_submitted.set(
                                                            "Validation error: enter name, 4+ char token, and accept policy."
                                                                .into(),
                                                        );
                                                    } else {
                                                        form_submitted.set(format!(
                                                            "Saved: {n} · {e} · {w} workers · auto_deploy={auto}"
                                                        ));
                                                    }
                                                }),
                                                Button::new("Reset", move || {
                                                    form_name.set(String::new());
                                                    form_key.set(String::new());
                                                    form_env.set(0);
                                                    form_workers.set(4.0);
                                                    form_autodeploy.set(true);
                                                    form_terms.set(false);
                                                    form_submitted.set(String::new());
                                                }),
                                                Spacer::new(),
                                                Text::new(move || {
                                                    let n = form_name.get();
                                                    let k = form_key.get();
                                                    let ok = !n.trim().is_empty()
                                                        && k.len() >= 4
                                                        && form_terms.get();
                                                    let env_name = match form_env.get() {
                                                        1 => "Staging",
                                                        2 => "Development",
                                                        _ => "Production",
                                                    };
                                                    if ok {
                                                        format!("Valid ({env_name})")
                                                    } else {
                                                        "Complete required fields".to_owned()
                                                    }
                                                })
                                                .font(Font::caption())
                                                .color(Color::secondary()),
                                            ),
                                        ),
                                        If::new(
                                            move || !form_submitted.get().is_empty(),
                                            move || {
                                                Text::new(move || form_submitted.get())
                                                    .padding(10)
                                                    .background(Color::surface())
                                                    .border(1, Color::border())
                                                    .corner_radius(8)
                                                    .transition(Transition::Opacity)
                                            },
                                        ),
                                    ),
                                )
                                .alignment(HorizontalAlignment::Leading)
                                .padding(16)
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
