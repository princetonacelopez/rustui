//! Core Tier 1 widgets (`Text`, `Image`, `Button`, `Toggle`, `Divider`, `Spacer`, stacks, `Grid`, `Canvas`, `Shader`).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rustui_core::{
    AccessibilityRole, Alignment, Binding, BuildCx, Color, ContainerKind, ContentMode,
    ControlNodeData, CornerRadii, Cx, DirtyFlags, EmptyView, FontSpec, Frame, GridTrack,
    HorizontalAlignment, ImageNodeData, ImageSourceData, IntoPx, IntoTextSource, KeyCode, NodeId,
    NodeKind, Point, Primitive, Rect, ShaderNodeData, TextNodeData, TextSource, TruncationMode,
    VerticalAlignment, View, ViewSequence,
};
use rustui_layout::{CustomLayoutBox, Layout};
use rustui_render::DisplayList;

use crate::theme::{ButtonStyle, Font, Theme, ToggleStyle};

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

fn resolve_theme(cx: &BuildCx) -> Theme {
    cx.env::<Theme>().unwrap_or_default()
}

/// Declarative text view supporting static strings and reactive closures (`Fn() -> String`).
#[derive(Debug)]
pub struct Text {
    source: TextSource,
    font: FontSpec,
    color: Option<Color>,
    line_limit: Option<usize>,
    truncation: TruncationMode,
    alignment: HorizontalAlignment,
    underline: bool,
    strikethrough: bool,
}

impl Text {
    pub fn new(source: impl IntoTextSource) -> Self {
        Self {
            source: source.into_text_source(),
            font: Font::body(),
            color: None,
            line_limit: None,
            truncation: TruncationMode::Tail,
            alignment: HorizontalAlignment::Leading,
            underline: false,
            strikethrough: false,
        }
    }

    pub fn font(mut self, font: FontSpec) -> Self {
        self.font = font;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn line_limit(mut self, limit: usize) -> Self {
        self.line_limit = Some(limit);
        self
    }

    pub fn truncation_mode(mut self, mode: TruncationMode) -> Self {
        self.truncation = mode;
        self
    }

    pub fn alignment(mut self, alignment: HorizontalAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }
}

impl Primitive for Text {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let color = self.color.unwrap_or_else(|| {
            if theme.uses_default_palette() {
                Color::primary()
            } else {
                theme.colors.primary
            }
        });
        let data = TextNodeData {
            text: match &self.source {
                TextSource::Static(s) => s.clone(),
                TextSource::Reactive(_) => String::new(),
            },
            spans: Vec::new(),
            font: self.font,
            color: Some(color),
            line_limit: self.line_limit,
            truncation: self.truncation,
            alignment: self.alignment,
            underline: self.underline,
            strikethrough: self.strikethrough,
            preedit: None,
            selection: None,
            caret: None,
        };
        let node = cx.alloc_node(NodeKind::Text(data));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(node) {
                n.accessibility.role = AccessibilityRole::Label;
                if let TextSource::Static(s) = &self.source {
                    n.accessibility.label = Some(s.clone());
                }
            }
        });
        if let TextSource::Reactive(f) = self.source {
            cx.runtime.register_node_updater(
                node,
                move || f(),
                |n, val| {
                    n.accessibility.label = Some(val.clone());
                    if let NodeKind::Text(t) = &mut n.kind {
                        t.text = val;
                    }
                    DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY
                },
            );
        }
        node
    }
}

impl View for Text {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Raster or SVG image view.
#[derive(Debug, Clone)]
pub struct Image {
    source: ImageSourceData,
    content_mode: ContentMode,
    tint: Option<Color>,
}

impl Image {
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        let vec = bytes.into();
        let key = hash_bytes(&vec);
        Self {
            source: ImageSourceData::EncodedBytes {
                key,
                bytes: Rc::new(vec),
                is_svg: false,
            },
            content_mode: ContentMode::Fit,
            tint: None,
        }
    }

    pub fn from_svg(svg_bytes: impl Into<Vec<u8>>) -> Self {
        let vec = svg_bytes.into();
        let key = hash_bytes(&vec);
        Self {
            source: ImageSourceData::EncodedBytes {
                key,
                bytes: Rc::new(vec),
                is_svg: true,
            },
            content_mode: ContentMode::Fit,
            tint: None,
        }
    }

    pub fn from_rgba(width: u32, height: u32, pixels: impl Into<Vec<u8>>) -> Self {
        let vec = pixels.into();
        let key = hash_bytes(&vec) ^ (u64::from(width) << 32) ^ u64::from(height);
        Self {
            source: ImageSourceData::Rgba {
                key,
                width,
                height,
                pixels: Rc::new(vec),
            },
            content_mode: ContentMode::Fit,
            tint: None,
        }
    }

    pub fn placeholder() -> Self {
        Self {
            source: ImageSourceData::PendingPlaceholder,
            content_mode: ContentMode::Fit,
            tint: None,
        }
    }

    pub fn content_mode(mut self, mode: ContentMode) -> Self {
        self.content_mode = mode;
        self
    }

    pub fn tint(mut self, tint: Color) -> Self {
        self.tint = Some(tint);
        self
    }
}

impl Primitive for Image {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let node = cx.alloc_node(NodeKind::Image(ImageNodeData {
            source: self.source,
            content_mode: self.content_mode,
            tint: self.tint,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(node) {
                n.accessibility.role = AccessibilityRole::Image;
            }
        });
        node
    }
}

impl View for Image {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Interactive button widget (`Button::new(label, action)`).
pub struct Button {
    label: TextSource,
    on_tap: Rc<dyn Fn()>,
    disabled: bool,
    style: Option<ButtonStyle>,
}

impl std::fmt::Debug for Button {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Button")
            .field("label", &self.label)
            .field("disabled", &self.disabled)
            .finish()
    }
}

impl Button {
    pub fn new(label: impl IntoTextSource, on_tap: impl Fn() + 'static) -> Self {
        Self {
            label: label.into_text_source(),
            on_tap: Rc::new(on_tap),
            disabled: false,
            style: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = Some(style);
        self
    }
}

impl Primitive for Button {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let style = self
            .style
            .or_else(|| cx.env::<ButtonStyle>())
            .unwrap_or_else(|| ButtonStyle::primary(&theme));

        let node = cx.alloc_node(NodeKind::Control(ControlNodeData {
            kind: "button",
            is_disabled: self.disabled,
            ..Default::default()
        }));

        let tap_action = self.on_tap.clone();
        let key_action = self.on_tap;
        let disabled = self.disabled;
        let static_label = match &self.label {
            TextSource::Static(s) => Some(s.clone()),
            TextSource::Reactive(_) => None,
        };

        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(node) {
                n.layout_props.padding = style.padding;
                n.layout_props.frame = Some(if style.background.a <= 0.01 {
                    Frame::new()
                        .min_width(24.0)
                        .min_height(26.0)
                        .alignment(Alignment::CENTER)
                } else {
                    Frame::new()
                        .min_width(36.0)
                        .min_height(34.0)
                        .alignment(Alignment::CENTER)
                });
                n.visual_props.background = Some(if disabled {
                    style.background.with_alpha(0.45)
                } else {
                    style.background
                });
                n.visual_props.border = style.border;
                n.visual_props.corner_radii = CornerRadii::all(style.corner_radius);
                n.focus.focusable = !disabled;
                n.accessibility.role = AccessibilityRole::Button;
                n.accessibility.label = static_label.clone();
                n.accessibility.supports_click = !disabled;
                n.accessibility.supports_focus = !disabled;
                if !disabled {
                    n.events.on_tap = Some(tap_action);
                    n.events.on_key = Some(Rc::new(move |ev| {
                        if ev.is_down && matches!(ev.key, KeyCode::Enter | KeyCode::Space) {
                            key_action();
                            return true;
                        }
                        false
                    }));
                }
            }
        });

        let text_child = Text::new(self.label)
            .font(Font::headline())
            .color(style.foreground)
            .build_node(cx);
        cx.runtime.attach_child(node, text_child);
        node
    }
}

impl View for Button {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Interactive switch toggle (`Toggle::new(label, binding)`).
#[derive(Debug)]
pub struct Toggle {
    label: String,
    is_on: Binding<bool>,
}

impl Toggle {
    pub fn new(label: impl Into<String>, is_on: Binding<bool>) -> Self {
        Self {
            label: label.into(),
            is_on,
        }
    }
}

impl Primitive for Toggle {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let style = cx
            .env::<ToggleStyle>()
            .unwrap_or_else(|| ToggleStyle::from_theme(&theme));

        let row = cx.alloc_node(NodeKind::Container(ContainerKind::HStack {
            spacing: 10.0,
            alignment: VerticalAlignment::Center,
        }));

        let switch_node = cx.alloc_node(NodeKind::Control(ControlNodeData {
            kind: "toggle",
            value_bool: Some(cx.runtime.enter(|| self.is_on.get())),
            ..Default::default()
        }));

        let toggle_b = self.is_on.clone();
        let toggle_fn = Rc::new(move || {
            toggle_b.update(|v| *v = !*v);
        });
        let lbl_toggle = toggle_fn.clone();
        let key_fn = toggle_fn.clone();
        let set_b = self.is_on.clone();
        let label_str = self.label.clone();

        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(switch_node) {
                n.layout_props.frame = Some(Frame::fixed(44.0, 24.0));
                n.visual_props.corner_radii = CornerRadii::all(12.0);
                n.animation = Some(rustui_core::NodeAnimation {
                    animation: rustui_core::Animation::ease_out(0.14),
                    transition: None,
                });
                n.focus.focusable = true;
                n.accessibility.role = AccessibilityRole::Switch;
                n.accessibility.label = Some(label_str);
                n.accessibility.supports_click = true;
                n.accessibility.supports_focus = true;
                n.accessibility.supports_set_value = true;
                n.events.on_tap = Some(toggle_fn);
                n.events.on_key = Some(Rc::new(move |ev| {
                    if ev.is_down && matches!(ev.key, KeyCode::Space | KeyCode::Enter) {
                        key_fn();
                        return true;
                    }
                    false
                }));
                n.events.on_accessibility_set_value = Some(Rc::new(move |val| {
                    let parsed = matches!(val.as_str(), "true" | "1" | "on");
                    set_b.set(parsed);
                }));
            }
        });

        let watch_b = self.is_on.clone();
        cx.runtime.register_node_updater(
            switch_node,
            move || watch_b.get(),
            move |n, on| {
                n.visual_props.background = Some(if on { style.on_track } else { style.off_track });
                n.accessibility.value = Some(if on { "on".into() } else { "off".into() });
                if let NodeKind::Control(c) = &mut n.kind {
                    c.value_bool = Some(on);
                }
                DirtyFlags::PAINT | DirtyFlags::ACCESSIBILITY
            },
        );

        let knob = cx.alloc_node(NodeKind::Modifier);
        cx.runtime.with_inner_mut(|inner| {
            if let Some(k) = inner.nodes.get_mut(knob) {
                k.layout_props.frame = Some(Frame::fixed(18.0, 18.0));
                k.visual_props.background = Some(style.knob_color);
                k.visual_props.corner_radii = CornerRadii::all(9.0);
                k.animation = Some(rustui_core::NodeAnimation {
                    animation: rustui_core::Animation::ease_out(0.14),
                    transition: None,
                });
                k.visual_props.shadow = Some(rustui_core::BoxShadow {
                    color: Color::rgba(0.0, 0.0, 0.0, 0.18),
                    offset: Point::new(0.0, 1.0),
                    blur_radius: 2.0,
                });
            }
        });
        cx.runtime.register_node_updater(
            knob,
            move || self.is_on.get(),
            |k, on| {
                k.layout_props.offset = Point::new(if on { 10.0 } else { -10.0 }, 0.0);
                DirtyFlags::LAYOUT | DirtyFlags::PAINT
            },
        );
        cx.runtime.attach_child(switch_node, knob);

        let lbl = Text::new(self.label)
            .color(style.label_color)
            .build_node(cx);
        cx.runtime.with_inner_mut(|inner| {
            if let Some(l) = inner.nodes.get_mut(lbl) {
                l.events.on_tap = Some(lbl_toggle);
            }
        });
        cx.runtime.attach_child(row, switch_node);
        cx.runtime.attach_child(row, lbl);
        row
    }
}

impl View for Toggle {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Horizontal or vertical hairline separator.
#[derive(Debug, Clone, Copy)]
pub struct Divider {
    thickness: f32,
    color: Option<Color>,
}

impl Default for Divider {
    fn default() -> Self {
        Self::new()
    }
}

impl Divider {
    pub fn new() -> Self {
        Self {
            thickness: 1.0,
            color: None,
        }
    }

    pub fn thickness(mut self, t: impl IntoPx) -> Self {
        self.thickness = t.into_px().max(0.5);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

impl Primitive for Divider {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let c = self.color.unwrap_or_else(|| {
            if theme.uses_default_palette() {
                Color::border()
            } else {
                theme.colors.border
            }
        });
        cx.alloc_node(NodeKind::Container(ContainerKind::Divider {
            thickness: self.thickness,
            color: c,
        }))
    }
}

impl View for Divider {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Flexible empty space expanding along the enclosing stack axis.
#[derive(Debug, Clone, Copy, Default)]
pub struct Spacer {
    min_length: f32,
}

impl Spacer {
    pub fn new() -> Self {
        Self { min_length: 0.0 }
    }

    pub fn min_length(min_length: impl IntoPx) -> Self {
        Self {
            min_length: min_length.into_px().max(0.0),
        }
    }
}

impl Primitive for Spacer {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        cx.alloc_node(NodeKind::Container(ContainerKind::Spacer {
            min_length: self.min_length,
        }))
    }
}

impl View for Spacer {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Vertical stack container (`VStack::new(12, (...))`).
#[derive(Debug)]
pub struct VStack<C: ViewSequence> {
    spacing: f32,
    alignment: HorizontalAlignment,
    children: C,
}

impl<C: ViewSequence> VStack<C> {
    pub fn new(spacing: impl IntoPx, children: C) -> Self {
        Self {
            spacing: spacing.into_px(),
            alignment: HorizontalAlignment::Center,
            children,
        }
    }

    pub fn alignment(mut self, alignment: HorizontalAlignment) -> Self {
        self.alignment = alignment;
        self
    }
}

impl<C: ViewSequence> Primitive for VStack<C> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let node = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
            spacing: self.spacing,
            alignment: self.alignment,
        }));
        self.children.build_all(cx, node);
        node
    }
}

impl<C: ViewSequence> View for VStack<C> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Horizontal stack container (`HStack::new(8, (...))`).
#[derive(Debug)]
pub struct HStack<C: ViewSequence> {
    spacing: f32,
    alignment: VerticalAlignment,
    children: C,
}

impl<C: ViewSequence> HStack<C> {
    pub fn new(spacing: impl IntoPx, children: C) -> Self {
        Self {
            spacing: spacing.into_px(),
            alignment: VerticalAlignment::Center,
            children,
        }
    }

    pub fn alignment(mut self, alignment: VerticalAlignment) -> Self {
        self.alignment = alignment;
        self
    }
}

impl<C: ViewSequence> Primitive for HStack<C> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let node = cx.alloc_node(NodeKind::Container(ContainerKind::HStack {
            spacing: self.spacing,
            alignment: self.alignment,
        }));
        self.children.build_all(cx, node);
        node
    }
}

impl<C: ViewSequence> View for HStack<C> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// 2D overlay stack container (`ZStack::new(...)`).
#[derive(Debug)]
pub struct ZStack<C: ViewSequence> {
    alignment: Alignment,
    children: C,
}

impl<C: ViewSequence> ZStack<C> {
    pub fn new(children: C) -> Self {
        Self {
            alignment: Alignment::CENTER,
            children,
        }
    }

    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }
}

impl<C: ViewSequence> Primitive for ZStack<C> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let node = cx.alloc_node(NodeKind::Container(ContainerKind::ZStack {
            alignment: self.alignment,
        }));
        self.children.build_all(cx, node);
        node
    }
}

impl<C: ViewSequence> View for ZStack<C> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Multi-column grid container (`Grid`).
#[derive(Debug)]
pub struct Grid<C: ViewSequence> {
    columns: Vec<GridTrack>,
    h_spacing: f32,
    v_spacing: f32,
    children: C,
}

impl<C: ViewSequence> Grid<C> {
    pub fn new(columns: Vec<GridTrack>, children: C) -> Self {
        Self {
            columns,
            h_spacing: 8.0,
            v_spacing: 8.0,
            children,
        }
    }

    pub fn spacing(mut self, horizontal: impl IntoPx, vertical: impl IntoPx) -> Self {
        self.h_spacing = horizontal.into_px();
        self.v_spacing = vertical.into_px();
        self
    }
}

impl<C: ViewSequence> Primitive for Grid<C> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let node = cx.alloc_node(NodeKind::Container(ContainerKind::Grid {
            columns: self.columns,
            h_spacing: self.h_spacing,
            v_spacing: self.v_spacing,
        }));
        self.children.build_all(cx, node);
        node
    }
}

impl<C: ViewSequence> View for Grid<C> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Wrapper view using a user-supplied custom [`Layout`] implementation.
pub struct CustomLayoutView<L: Layout, C: ViewSequence> {
    layout: L,
    children: C,
}

impl<L: Layout, C: ViewSequence> std::fmt::Debug for CustomLayoutView<L, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomLayoutView").finish_non_exhaustive()
    }
}

impl<L: Layout, C: ViewSequence> CustomLayoutView<L, C> {
    pub fn new(layout: L, children: C) -> Self {
        Self { layout, children }
    }
}

impl<L: Layout, C: ViewSequence> Primitive for CustomLayoutView<L, C> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let node = cx.alloc_node(NodeKind::Container(ContainerKind::CustomLayout));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(node) {
                n.custom_layout = Some(Rc::new(CustomLayoutBox(Rc::new(self.layout))));
            }
        });
        self.children.build_all(cx, node);
        node
    }
}

impl<L: Layout, C: ViewSequence> View for CustomLayoutView<L, C> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

type CanvasDisplayPainter = Rc<dyn Fn(Rect, &mut DisplayList)>;

/// Custom display-list drawing canvas view (`Canvas::new(move |rect, dl| ...)`).
pub struct Canvas {
    painter: CanvasDisplayPainter,
}

impl std::fmt::Debug for Canvas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Canvas").finish_non_exhaustive()
    }
}

impl Canvas {
    pub fn new(painter: impl Fn(Rect, &mut DisplayList) + 'static) -> Self {
        Self {
            painter: Rc::new(painter),
        }
    }
}

impl Primitive for Canvas {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let p = self.painter;
        cx.alloc_node(NodeKind::Canvas(rustui_core::CanvasNodeData {
            painter: Rc::new(move |rect, any_dl| {
                if let Some(dl) = any_dl.downcast_mut::<DisplayList>() {
                    p(rect, dl);
                }
            }),
        }))
    }
}

impl View for Canvas {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Custom WGSL shader surface view (`Shader`).
#[derive(Debug, Clone)]
pub struct Shader {
    shader_id: u64,
    wgsl: String,
    uniforms: Vec<u8>,
}

impl Shader {
    pub fn new(wgsl: impl Into<String>) -> Self {
        let wgsl = wgsl.into();
        let shader_id = hash_bytes(wgsl.as_bytes());
        Self {
            shader_id,
            wgsl,
            uniforms: Vec::new(),
        }
    }

    pub fn uniforms(mut self, bytes: Vec<u8>) -> Self {
        self.uniforms = bytes;
        self
    }
}

impl Primitive for Shader {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        cx.alloc_node(NodeKind::Shader(ShaderNodeData {
            shader_id: self.shader_id,
            wgsl: self.wgsl,
            uniform_data: self.uniforms,
        }))
    }
}

impl View for Shader {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Convenience macro: `vstack![child1, child2, ...]` (Section 4).
#[macro_export]
macro_rules! vstack {
    ($($child:expr),* $(,)?) => {
        $crate::VStack::new(8, ($($child,)*))
    };
}

/// Convenience macro: `hstack![child1, child2, ...]`.
#[macro_export]
macro_rules! hstack {
    ($($child:expr),* $(,)?) => {
        $crate::HStack::new(8, ($($child,)*))
    };
}

/// Convenience macro: `zstack![child1, child2, ...]`.
#[macro_export]
macro_rules! zstack {
    ($($child:expr),* $(,)?) => {
        $crate::ZStack::new(($($child,)*))
    };
}
