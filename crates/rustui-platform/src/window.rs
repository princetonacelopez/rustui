//! Multi-window `winit` event loop, `wgpu` surface management, IME integration, and AccessKit bridge.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use accesskit_winit::{Adapter as AkAdapter, Event as AkEvent, WindowEvent as AkWindowEvent};
use rustui_core::{
    BuildCx, Color, KeyCode, KeyEvent, Modifiers, NodeId, Point, PointerButton, PointerEvent,
    PointerPhase, Rect, Runtime, ScrollEvent, Size, View,
};
use rustui_layout::LayoutEngine;
use rustui_render::{
    emit_display_list, DamageTracker, GlyphAtlas, InspectorOverlay, ResourceLoader, WgpuRenderer,
};
use rustui_text::{ParleyTextMeasurer, ParleyTextPainter, TextSystem};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId, WindowLevel};

use crate::accessibility::AccessKitBridge;
use crate::input::{FocusManager, GestureRecognizer};

/// System theme variant reported by the OS window manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SystemTheme {
    #[default]
    Light,
    Dark,
}

/// Configuration options for a desktop window (Section 8).
#[derive(Debug, Clone)]
pub struct WindowOptions {
    pub title: String,
    pub size: Size<f32>,
    pub min_size: Option<Size<f32>>,
    pub max_size: Option<Size<f32>>,
    pub resizable: bool,
    pub transparent: bool,
    pub always_on_top: bool,
    pub fullscreen: bool,
    pub custom_title_bar: bool,
    pub inspector: bool,
    pub clear_color: Color,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: "RustUI App".to_owned(),
            size: Size::new(960.0, 640.0),
            min_size: Some(Size::new(320.0, 240.0)),
            max_size: None,
            resizable: true,
            transparent: false,
            always_on_top: false,
            fullscreen: false,
            custom_title_bar: false,
            inspector: false,
            clear_color: Color::from_hex(0xf8fafc),
        }
    }
}

/// User event waking the winit event loop from AccessKit or background `cx.spawn` tasks.
#[derive(Debug)]
pub enum AppEvent {
    WakeRuntime,
    AccessKit(AkEvent),
}

impl From<AkEvent> for AppEvent {
    fn from(ev: AkEvent) -> Self {
        Self::AccessKit(ev)
    }
}

/// Specification for opening a window sharing the application [`Runtime`].
pub struct WindowSpec {
    pub options: WindowOptions,
    pub root_builder: Box<dyn FnOnce(&mut BuildCx) -> NodeId>,
}

impl std::fmt::Debug for WindowSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowSpec")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl WindowSpec {
    pub fn new<V: View>(options: WindowOptions, view: V) -> Self {
        Self {
            options,
            root_builder: Box::new(move |cx| view.build_node(cx)),
        }
    }
}

struct ManagedWindow {
    window: Arc<Window>,
    surface: Option<wgpu::Surface<'static>>,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    options: WindowOptions,
    root: NodeId,
    logical_size: Size<f32>,
    scale_factor: f32,
    focus: FocusManager,
    gestures: GestureRecognizer,
    damage: DamageTracker,
    inspector: InspectorOverlay,
    ak_adapter: AkAdapter,
    cursor_pos: Point,
    modifiers: Modifiers,
    occluded: bool,
}

/// Multi-window manager coordinating shared [`Runtime`], [`TextSystem`], and [`WgpuRenderer`].
pub struct WindowManager {
    pub runtime: Runtime,
    pub renderer: WgpuRenderer,
    pub text_system: TextSystem,
    pub atlas: GlyphAtlas,
    pub resources: ResourceLoader,
    pub layout_engine: LayoutEngine,
    pending_specs: Vec<WindowSpec>,
    windows: HashMap<WindowId, ManagedWindow>,
    proxy: EventLoopProxy<AppEvent>,
    on_theme_change: Option<Box<dyn Fn(SystemTheme)>>,
}

impl std::fmt::Debug for WindowManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowManager")
            .field("window_count", &self.windows.len())
            .finish()
    }
}

impl WindowManager {
    fn new(
        runtime: Runtime,
        specs: Vec<WindowSpec>,
        proxy: EventLoopProxy<AppEvent>,
        on_theme_change: Option<Box<dyn Fn(SystemTheme)>>,
    ) -> Self {
        let proxy_wake = proxy.clone();
        runtime.set_wake_callback(Arc::new(move || {
            let _ = proxy_wake.send_event(AppEvent::WakeRuntime);
        }));
        Self {
            runtime,
            renderer: WgpuRenderer::new(),
            text_system: TextSystem::new(),
            atlas: GlyphAtlas::default(),
            resources: ResourceLoader::new(),
            layout_engine: LayoutEngine::new(),
            pending_specs: specs,
            windows: HashMap::new(),
            proxy,
            on_theme_change,
        }
    }

    fn spawn_window(&mut self, event_loop: &ActiveEventLoop, spec: WindowSpec) {
        let opts = spec.options;
        let mut attrs = WindowAttributes::default()
            .with_title(opts.title.clone())
            .with_inner_size(LogicalSize::new(
                f64::from(opts.size.width),
                f64::from(opts.size.height),
            ))
            .with_resizable(opts.resizable)
            .with_transparent(opts.transparent)
            .with_decorations(!opts.custom_title_bar)
            .with_visible(false);

        if let Some(min) = opts.min_size {
            attrs = attrs.with_min_inner_size(LogicalSize::new(
                f64::from(min.width),
                f64::from(min.height),
            ));
        }
        if let Some(max) = opts.max_size {
            attrs = attrs.with_max_inner_size(LogicalSize::new(
                f64::from(max.width),
                f64::from(max.height),
            ));
        }
        if opts.always_on_top {
            attrs = attrs.with_window_level(WindowLevel::AlwaysOnTop);
        }
        if opts.fullscreen {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }

        let Ok(window) = event_loop.create_window(attrs) else {
            tracing::error!("Failed to create winit window");
            return;
        };
        let window = Arc::new(window);
        let ak_adapter = AkAdapter::with_event_loop_proxy(event_loop, &window, self.proxy.clone());

        let mut bcx = BuildCx::new(self.runtime.clone(), None);
        let root = (spec.root_builder)(&mut bcx);
        self.runtime.with_inner_mut(|inner| {
            if inner.root.is_none() {
                inner.root = Some(root);
            }
        });

        let scale_factor = window.scale_factor() as f32;
        let phys = window.inner_size();
        let logical_size = Size::new(
            (phys.width as f32 / scale_factor).max(1.0),
            (phys.height as f32 / scale_factor).max(1.0),
        );

        let surface = self
            .renderer
            .wgpu_instance()
            .create_surface(window.clone())
            .ok();
        let surface_config = if let Some(surf) = &surface {
            if self.renderer.ensure_gpu(Some(surf)) {
                Some(wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
                    color_space: wgpu::SurfaceColorSpace::Auto,
                    width: phys.width.max(1),
                    height: phys.height.max(1),
                    present_mode: wgpu::PresentMode::Fifo,
                    alpha_mode: if opts.transparent {
                        wgpu::CompositeAlphaMode::PreMultiplied
                    } else {
                        wgpu::CompositeAlphaMode::Auto
                    },
                    view_formats: vec![],
                    desired_maximum_frame_latency: 2,
                })
            } else {
                None
            }
        } else {
            None
        };

        if let (Some(surf), Some(cfg), Some((device, _))) = (
            &surface,
            &surface_config,
            self.renderer.device_and_queue(),
        ) {
            surf.configure(device, cfg);
        }

        window.set_visible(true);
        window.request_redraw();

        let id = window.id();
        let inspector = InspectorOverlay::new(opts.inspector);
        self.windows.insert(
            id,
            ManagedWindow {
                window,
                surface,
                surface_config,
                options: opts,
                root,
                logical_size,
                scale_factor,
                focus: FocusManager::new(),
                gestures: GestureRecognizer::new(),
                damage: DamageTracker::new(),
                inspector,
                ak_adapter,
                cursor_pos: Point::ZERO,
                modifiers: Modifiers::NONE,
                occluded: false,
            },
        );
    }

    fn redraw_window(&mut self, window_id: WindowId) {
        self.runtime.poll_tasks();
        self.runtime.flush_reactive();

        let anim_updates = self
            .runtime
            .with_inner_mut(|inner| inner.scheduler.tick_animations(Instant::now()));
        if !anim_updates.is_empty() {
            self.runtime.with_inner_mut(|inner| {
                for (nid, prop, val) in anim_updates {
                    if let Some(n) = inner.nodes.get_mut(nid) {
                        match (prop, val) {
                            (
                                rustui_core::AnimProperty::Opacity,
                                rustui_core::AnimValue::Scalar(op),
                            ) => n.visual_props.opacity = op,
                            (
                                rustui_core::AnimProperty::Color,
                                rustui_core::AnimValue::Color(c),
                            ) => n.visual_props.background = Some(c),
                            _ => {}
                        }
                        n.dirty.insert(rustui_core::DirtyFlags::PAINT);
                    }
                }
            });
        }

        let Some(win) = self.windows.get_mut(&window_id) else {
            return;
        };
        if win.occluded {
            return;
        }

        let mut measurer = ParleyTextMeasurer {
            system: &mut self.text_system,
            scale_factor: win.scale_factor,
        };
        self.layout_engine
            .layout(&self.runtime, win.root, win.logical_size, &mut measurer);
        self.runtime.flush_effects();

        // Update IME status & cursor rect if focused node accepts text input.
        if let Some(focused_id) = win.focus.focused_node() {
            let ime_info = self.runtime.with_inner(|inner| {
                inner.nodes.get(focused_id).and_then(|n| {
                    if n.focus.accepts_text_input {
                        Some(n.bounds())
                    } else {
                        None
                    }
                })
            });
            if let Some(rect) = ime_info {
                win.window.set_ime_allowed(true);
                win.window.set_ime_cursor_area(
                    LogicalPosition::new(f64::from(rect.x), f64::from(rect.y)),
                    LogicalSize::new(f64::from(rect.width.max(2.0)), f64::from(rect.height.max(18.0))),
                );
            } else {
                win.window.set_ime_allowed(false);
            }
        } else {
            win.window.set_ime_allowed(false);
        }

        let viewport_rect = Rect::new(0.0, 0.0, win.logical_size.width, win.logical_size.height);
        win.damage.collect(&self.runtime, viewport_rect);

        let mut painter = ParleyTextPainter {
            system: &mut self.text_system,
            scale_factor: win.scale_factor,
        };
        let mut list = emit_display_list(
            &self.runtime,
            win.root,
            win.scale_factor,
            &mut self.resources,
            Some(&mut painter),
        );
        win.inspector
            .emit(&self.runtime, &win.damage, &mut list, viewport_rect);

        if let (Some(surface), Some(cfg)) = (&win.surface, &win.surface_config) {
            match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    let view = frame
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    if self
                        .renderer
                        .render_to_texture_view(
                            &view,
                            cfg.format,
                            cfg.width,
                            cfg.height,
                            &list,
                            &mut self.atlas,
                            &self.resources,
                            win.options.clear_color,
                        )
                        .is_ok()
                        && let Some((_, queue)) = self.renderer.device_and_queue()
                    {
                        queue.present(frame);
                    }
                }
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    if let Some((dev, _)) = self.renderer.device_and_queue() {
                        surface.configure(dev, cfg);
                    }
                }
                _ => {}
            }
        }

        let root = win.root;
        let title = win.options.title.clone();
        let focused = win.focus.focused_node();
        let rt = &self.runtime;
        win.ak_adapter
            .update_if_active(|| AccessKitBridge::build_tree_update(rt, root, &title, focused));

        self.runtime.with_inner_mut(|inner| {
            inner.scheduler.clear_redraw();
        });
        if self.runtime.needs_redraw() {
            win.window.request_redraw();
        }
    }
}

impl ApplicationHandler<AppEvent> for WindowManager {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let specs = std::mem::take(&mut self.pending_specs);
        for spec in specs {
            self.spawn_window(event_loop, spec);
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::WakeRuntime => {
                self.runtime.poll_tasks();
                self.runtime.flush_reactive();
                for win in self.windows.values() {
                    win.window.request_redraw();
                }
            }
            AppEvent::AccessKit(ak) => {
                if let Some(win) = self.windows.get_mut(&ak.window_id) {
                    match ak.window_event {
                        AkWindowEvent::InitialTreeRequested => {
                            let root = win.root;
                            let title = win.options.title.clone();
                            let focused = win.focus.focused_node();
                            let rt = &self.runtime;
                            win.ak_adapter.update_if_active(|| {
                                AccessKitBridge::build_tree_update(rt, root, &title, focused)
                            });
                        }
                        AkWindowEvent::ActionRequested(req) => {
                            AccessKitBridge::handle_action_request(
                                &self.runtime,
                                &mut win.focus,
                                &req,
                            );
                            win.window.request_redraw();
                        }
                        AkWindowEvent::AccessibilityDeactivated => {}
                    }
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(win) = self.windows.get_mut(&window_id) {
            win.ak_adapter.process_event(&win.window, &event);
        }

        match event {
            WindowEvent::CloseRequested => {
                self.windows.remove(&window_id);
                if self.windows.is_empty() {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(phys) => {
                if let Some(win) = self.windows.get_mut(&window_id) {
                    let w = phys.width.max(1);
                    let h = phys.height.max(1);
                    win.logical_size = Size::new(
                        w as f32 / win.scale_factor,
                        h as f32 / win.scale_factor,
                    );
                    if let (Some(surf), Some(cfg), Some((dev, _))) = (
                        &win.surface,
                        &mut win.surface_config,
                        self.renderer.device_and_queue(),
                    ) {
                        cfg.width = w;
                        cfg.height = h;
                        surf.configure(dev, cfg);
                    }
                    win.damage.invalidate_all();
                    win.window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(win) = self.windows.get_mut(&window_id) {
                    win.scale_factor = scale_factor as f32;
                    let phys = win.window.inner_size();
                    win.logical_size = Size::new(
                        (phys.width as f32 / win.scale_factor).max(1.0),
                        (phys.height as f32 / win.scale_factor).max(1.0),
                    );
                    win.damage.invalidate_all();
                    win.window.request_redraw();
                }
            }
            WindowEvent::Occluded(occluded) => {
                if let Some(win) = self.windows.get_mut(&window_id) {
                    win.occluded = occluded;
                }
            }
            WindowEvent::ThemeChanged(theme) => {
                if let Some(cb) = &self.on_theme_change {
                    let t = match theme {
                        winit::window::Theme::Dark => SystemTheme::Dark,
                        winit::window::Theme::Light => SystemTheme::Light,
                    };
                    cb(t);
                }
                for win in self.windows.values() {
                    win.window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(mods) => {
                if let Some(win) = self.windows.get_mut(&window_id) {
                    let state = mods.state();
                    win.modifiers = Modifiers {
                        shift: state.shift_key(),
                        ctrl: state.control_key(),
                        alt: state.alt_key(),
                        meta: state.super_key(),
                    };
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(win) = self.windows.get_mut(&window_id) {
                    let logical = Point::new(
                        position.x as f32 / win.scale_factor,
                        position.y as f32 / win.scale_factor,
                    );
                    win.cursor_pos = logical;
                    win.gestures.handle_pointer(
                        &self.runtime,
                        win.root,
                        &mut win.focus,
                        PointerEvent {
                            position: logical,
                            button: PointerButton::Primary,
                            phase: PointerPhase::Move,
                            click_count: 0,
                            modifiers: win.modifiers,
                        },
                    );
                    if self.runtime.needs_redraw() {
                        win.window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(win) = self.windows.get_mut(&window_id) {
                    let btn = match button {
                        MouseButton::Right => PointerButton::Secondary,
                        MouseButton::Middle => PointerButton::Middle,
                        _ => PointerButton::Primary,
                    };
                    let phase = match state {
                        ElementState::Pressed => PointerPhase::Down,
                        ElementState::Released => PointerPhase::Up,
                    };
                    win.gestures.handle_pointer(
                        &self.runtime,
                        win.root,
                        &mut win.focus,
                        PointerEvent {
                            position: win.cursor_pos,
                            button: btn,
                            phase,
                            click_count: 1,
                            modifiers: win.modifiers,
                        },
                    );
                    win.window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(win) = self.windows.get_mut(&window_id) {
                    let (dx, dy) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => (x * 24.0, y * 24.0),
                        MouseScrollDelta::PixelDelta(p) => (
                            p.x as f32 / win.scale_factor,
                            p.y as f32 / win.scale_factor,
                        ),
                    };
                    win.gestures.handle_scroll(
                        &self.runtime,
                        win.root,
                        ScrollEvent {
                            position: win.cursor_pos,
                            delta_x: dx,
                            delta_y: dy,
                            is_momentum: false,
                        },
                    );
                    win.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(win) = self.windows.get_mut(&window_id) {
                    let key = map_winit_key(&event.logical_key);
                    let kev = KeyEvent {
                        key,
                        physical_code: format!("{:?}", event.physical_key),
                        text: event.text.as_ref().map(|t| t.to_string()),
                        modifiers: win.modifiers,
                        is_repeat: event.repeat,
                        is_down: event.state == ElementState::Pressed,
                    };
                    win.gestures
                        .handle_key(&self.runtime, win.root, &mut win.focus, kev);
                    win.window.request_redraw();
                }
            }
            WindowEvent::Ime(ime) => {
                if let Some(win) = self.windows.get_mut(&window_id) {
                    match ime {
                        Ime::Preedit(text, cursor) => {
                            win.gestures
                                .handle_ime_preedit(&self.runtime, &win.focus, text, cursor);
                        }
                        Ime::Commit(text) => {
                            win.gestures
                                .handle_ime_commit(&self.runtime, &win.focus, text);
                        }
                        Ime::Enabled | Ime::Disabled => {}
                    }
                    win.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                self.redraw_window(window_id);
            }
            _ => {}
        }
    }
}

fn map_winit_key(key: &Key) -> KeyCode {
    match key {
        Key::Named(NamedKey::Enter) => KeyCode::Enter,
        Key::Named(NamedKey::Tab) => KeyCode::Tab,
        Key::Named(NamedKey::Space) => KeyCode::Space,
        Key::Named(NamedKey::Backspace) => KeyCode::Backspace,
        Key::Named(NamedKey::Delete) => KeyCode::Delete,
        Key::Named(NamedKey::Escape) => KeyCode::Escape,
        Key::Named(NamedKey::ArrowLeft) => KeyCode::ArrowLeft,
        Key::Named(NamedKey::ArrowRight) => KeyCode::ArrowRight,
        Key::Named(NamedKey::ArrowUp) => KeyCode::ArrowUp,
        Key::Named(NamedKey::ArrowDown) => KeyCode::ArrowDown,
        Key::Named(NamedKey::Home) => KeyCode::Home,
        Key::Named(NamedKey::End) => KeyCode::End,
        Key::Named(NamedKey::PageUp) => KeyCode::PageUp,
        Key::Named(NamedKey::PageDown) => KeyCode::PageDown,
        Key::Character(s) => KeyCode::Character(s.to_string()),
        other => KeyCode::Named(format!("{other:?}")),
    }
}

/// Runs the desktop event loop for one or more windows sharing one [`Runtime`].
pub fn run_desktop_windows(
    runtime: Runtime,
    specs: Vec<WindowSpec>,
    on_theme_change: Option<Box<dyn Fn(SystemTheme)>>,
) -> Result<(), String> {
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let proxy = event_loop.create_proxy();
    let mut manager = WindowManager::new(runtime, specs, proxy, on_theme_change);
    event_loop.run_app(&mut manager).map_err(|e| e.to_string())
}
