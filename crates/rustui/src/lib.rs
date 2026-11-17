//! RustUI: declarative, GPU-rendered desktop UI framework with SwiftUI-style ergonomics.

pub use rustui_core as core;
pub use rustui_layout as layout;
pub use rustui_platform as platform;
pub use rustui_render as render;
pub use rustui_text as text;
pub use rustui_widgets as widgets;

pub mod prelude {
    pub use crate::App;
    pub use rustui_core::{
        AccessibilityLive, AccessibilityRole, Alignment, Animation, AnyView, AsyncExecutor,
        Binding, BuildCx, Color, ContentMode, CornerRadii, Cx, DragEvent, DragPayload, DropEvent,
        Edge, Edges, EffectHandle, EmptyView, ForEach, Frame, GridTrack, HorizontalAlignment, If,
        KeyCode, KeyEvent, Memo, Modifiers, NodeId, Observable, ObservableModel, PinchEvent, Point,
        PointerButton, PointerEvent, PointerPhase, Primitive, Rect, Resource, ResourceState,
        Runtime, SafeAreaInsets, ScrollEvent, SemanticColor, Size, State, Switch, TextDirection,
        ThemePalette, TimerId, Transition, VerticalAlignment, View, ViewExt, ViewSequence,
        impl_observable, preview, rustui_check_hooks, with_animation,
    };
    pub use rustui_layout::{ChildProxy, Children, Layout, LayoutEngine};
    pub use rustui_platform::{
        ChordMatch, DesktopPlatform, FileDialogOptions, FrameSummary, HeadlessSimulator,
        HotReloadHost, KeyChord, KeyContext, KeyStroke, KeybindingEntry, KeybindingRegistry,
        Platform, SystemTheme, WindowOptions, detect_platform_capabilities, detect_reduced_motion,
        detect_system_theme,
    };
    pub use rustui_render::{
        DisplayList, FontBlob, GradientKind, GradientStop, InspectorFrameSnapshot,
        InspectorNodeSnapshot, InspectorOverlay, InspectorServer, PathOp, ResourceLoader,
        export_perfetto_trace, include_asset,
    };
    pub use rustui_text::{
        AnchorBias, AnchorId, DecorationKind, EditableText, EditorDecoration, FoldRange,
        GutterDiffKind, ImeState, LineDiffMarker, RichText, Selection, SyntaxTokenSpan, TextBuffer,
        TextSpan, TextSystem,
    };
    pub use rustui_widgets::{
        Alert, AlertButton, Button, ButtonStyle, Canvas, Checkbox, CommandItem, CommandPalette,
        ConfirmationDialog, CustomLayoutView, DisclosureGroup, Divider, EditorBufferState,
        EditorSurface, FileDialog, Font, Gauge, Grid, HStack, Image, LazyVStack, List, ListStyle,
        Menu, MenuItem, NavigationPage, NavigationStack, OutlineGroup, Picker, Popover,
        ProgressView, ScrollView, SecureField, Shader, Sheet, Slider, Spacer, SplitView, StatusBar,
        TabBar, TabBarItem, TabItem, TabView, Table, TableColumn, TerminalLine, TerminalLineTone,
        TerminalView, Text, TextEditor, TextField, Theme, Toggle, ToggleStyle, Toolbar, Tree,
        TreeItemData, VStack, ViewThatFits, WidgetViewExt, ZStack, format_virtual_row_text, hstack,
        vstack, zstack,
    };
}

use rustui_core::{BuildCx, NodeId, Runtime, Size, View, ViewExt};
use rustui_platform::{
    HeadlessSimulator, SystemTheme, WindowOptions, WindowSpec, run_desktop_windows,
};
use rustui_widgets::Theme;

/// Top-level RustUI application builder owning a single [`Runtime`] and one or more windows.
pub struct App {
    runtime: Runtime,
    windows: Vec<PendingWindow>,
    theme: Theme,
    headless: bool,
    inspector: bool,
    snapshot_path: Option<std::path::PathBuf>,
    on_theme_change: Option<Box<dyn Fn(SystemTheme)>>,
}

type WindowRootBuilder = Box<dyn FnOnce(&mut BuildCx, Theme) -> NodeId>;

struct PendingWindow {
    options: WindowOptions,
    builder: WindowRootBuilder,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("window_count", &self.windows.len())
            .field("headless", &self.headless)
            .field("inspector", &self.inspector)
            .finish()
    }
}

impl App {
    /// Creates a new RustUI application with a primary root [`View`].
    pub fn new<V: View>(root: V) -> Self {
        let mut headless = std::env::var("RUSTUI_HEADLESS").is_ok_and(|v| v == "1" || v == "true");
        let mut inspector = std::env::var("RUSTUI_INSPECTOR").is_ok_and(|v| v == "1");
        let mut snapshot_path = None;

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--headless" => headless = true,
                "--inspector" => inspector = true,
                "--snapshot" => {
                    if let Some(p) = args.next() {
                        snapshot_path = Some(std::path::PathBuf::from(p));
                        headless = true;
                    }
                }
                _ => {}
            }
        }

        let theme = match rustui_platform::detect_system_theme() {
            SystemTheme::Dark => Theme::dark(),
            SystemTheme::Light => Theme::light(),
        };
        let runtime = Runtime::new();
        runtime.set_reduced_motion(rustui_platform::detect_reduced_motion());
        let opts = WindowOptions {
            inspector,
            clear_color: theme.colors.background,
            follow_os_theme: true,
            ..Default::default()
        };

        Self {
            runtime,
            windows: vec![PendingWindow {
                options: opts,
                builder: Box::new(move |cx, theme| {
                    let palette = theme.to_palette();
                    root.environment(theme).environment(palette).build_node(cx)
                }),
            }],
            theme,
            headless,
            inspector,
            snapshot_path,
            on_theme_change: None,
        }
    }

    /// Sets the primary window title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        if let Some(w) = self.windows.first_mut() {
            w.options.title = title.into();
        }
        self
    }

    /// Sets the primary window initial logical size.
    pub fn size(mut self, width: f32, height: f32) -> Self {
        if let Some(w) = self.windows.first_mut() {
            w.options.size = Size::new(width, height);
        }
        self
    }

    /// Replaces the primary window's [`WindowOptions`].
    pub fn window_options(mut self, mut options: WindowOptions) -> Self {
        options.inspector |= self.inspector;
        if let Some(w) = self.windows.first_mut() {
            w.options = options;
        }
        self
    }

    /// Adds a secondary window sharing the same [`Runtime`].
    pub fn with_window<V: View>(mut self, mut options: WindowOptions, view: V) -> Self {
        options.inspector |= self.inspector;
        self.windows.push(PendingWindow {
            options,
            builder: Box::new(move |cx, theme| {
                let palette = theme.to_palette();
                view.environment(theme).environment(palette).build_node(cx)
            }),
        });
        self
    }

    /// Sets the application [`Theme`] (`Theme::light()` or `Theme::dark()`).
    pub fn theme(mut self, theme: Theme) -> Self {
        if let Some(w) = self.windows.first_mut() {
            w.options.clear_color = theme.colors.background;
            w.options.follow_os_theme = false;
        }
        self.theme = theme;
        self
    }

    /// Enables or disables the debug inspector overlay.
    pub fn inspector(mut self, enabled: bool) -> Self {
        self.inspector = enabled;
        for w in &mut self.windows {
            w.options.inspector = enabled;
        }
        self
    }

    /// Forces headless execution mode (renders a frame and exits cleanly without opening a window).
    pub fn headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    /// Registers a callback notified when the OS switches between light and dark themes.
    pub fn on_theme_change(mut self, f: impl Fn(SystemTheme) + 'static) -> Self {
        self.on_theme_change = Some(Box::new(f));
        self
    }

    /// Returns a reference to the application's [`Runtime`].
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Runs the application (desktop event loop, or headless verification if `--headless` / `RUSTUI_HEADLESS=1`).
    pub fn run(self) -> Result<(), String> {
        if self.headless {
            return self.run_headless();
        }

        let theme = self.theme;
        let specs = self
            .windows
            .into_iter()
            .map(|pw| {
                let t = theme.clone();
                WindowSpec {
                    options: pw.options,
                    root_builder: Box::new(move |cx| (pw.builder)(cx, t)),
                }
            })
            .collect();

        run_desktop_windows(self.runtime, specs, self.on_theme_change)
    }

    /// Mounts all windows headlessly, executes layout/render/accessibility checks, and saves any snapshot.
    pub fn run_headless(self) -> Result<(), String> {
        let theme = self.theme;
        let snapshot_path = self.snapshot_path;
        for (idx, pw) in self.windows.into_iter().enumerate() {
            let opts = pw.options;
            let mut bcx = BuildCx::new(self.runtime.clone(), None);
            let root = self.runtime.enter(|| (pw.builder)(&mut bcx, theme.clone()));
            self.runtime.with_inner_mut(|inner| {
                if inner.root.is_none() {
                    inner.root = Some(root);
                }
            });

            let mut sim = HeadlessSimulator {
                runtime: self.runtime.clone(),
                root,
                viewport: opts.size,
                scale_factor: 1.0,
                title: opts.title.clone(),
                clear_color: opts.clear_color,
                focus: rustui_platform::FocusManager::new(),
                gestures: rustui_platform::GestureRecognizer::new(),
                layout_engine: rustui_layout::LayoutEngine::new(),
                text_system: rustui_text::TextSystem::new(),
                atlas: rustui_render::GlyphAtlas::default(),
                resources: rustui_render::ResourceLoader::new(),
                damage: rustui_render::DamageTracker::new(),
                inspector: rustui_render::InspectorOverlay::new(opts.inspector),
                renderer: rustui_render::WgpuRenderer::new(),
            };
            let summary = sim.step_frame();
            let png = sim.render_png()?;
            let ak = sim.accessibility_tree();

            if idx == 0
                && let Some(path) = &snapshot_path
            {
                std::fs::write(path, &png).map_err(|e| e.to_string())?;
            }

            tracing::info!(
                title = %opts.title,
                nodes = self.runtime.node_count(),
                laid_out = summary.laid_out_nodes,
                ak_nodes = ak.nodes.len(),
                png_bytes = png.len(),
                "RustUI headless run complete"
            );
        }
        Ok(())
    }
}
