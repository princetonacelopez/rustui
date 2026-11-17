//! SwiftUI-style proposal/response `Layout` protocol, incremental caching engine, and pixel snapping.

use std::rc::Rc;
use std::time::Instant;

use rustui_core::{
    ContainerKind, DirtyFlags, NodeId, NodeKind, Point, Rect, Runtime, RuntimeInner, Size,
    TextDirection, TextNodeData,
};

use crate::containers::{
    GridLayout, HStackLayout, LazyVStackLayout, SpacerLayout, VStackLayout, ZStackLayout,
};

/// Interface implemented by text engines (such as `rustui-text`'s `ParleyTextMeasurer`)
/// to supply exact shaped text bounds during layout.
pub trait TextMeasurer {
    fn measure_text(&mut self, data: &TextNodeData, max_width: Option<f32>) -> Size<f32>;
}

/// Deterministic fallback text measurer used when running `rustui-layout` standalone in unit tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTextMeasurer;

impl TextMeasurer for DefaultTextMeasurer {
    fn measure_text(&mut self, data: &TextNodeData, max_width: Option<f32>) -> Size<f32> {
        let font_size = data.font.size.max(1.0);
        let char_w = font_size * if data.font.monospace { 0.6 } else { 0.54 };
        let line_h = (font_size * 1.35).ceil();

        if data.text.is_empty() {
            return Size::new(0.0, line_h);
        }

        let wrap_w = max_width.filter(|w| w.is_finite() && *w > 0.0);
        let mut max_line_w = 0.0f32;
        let mut total_lines = 0usize;

        for raw_line in data.text.split('\n') {
            if let Some(limit_w) = wrap_w {
                let max_chars = ((limit_w / char_w).floor() as usize).max(1);
                let chars: Vec<char> = raw_line.chars().collect();
                if chars.is_empty() {
                    total_lines += 1;
                } else {
                    for chunk in chars.chunks(max_chars) {
                        total_lines += 1;
                        max_line_w = max_line_w.max(chunk.len() as f32 * char_w);
                    }
                }
            } else {
                total_lines += 1;
                max_line_w = max_line_w.max(raw_line.chars().count() as f32 * char_w);
            }
        }

        if let Some(limit) = data.line_limit {
            total_lines = total_lines.min(limit.max(1));
        }
        let width = match wrap_w {
            Some(w) => max_line_w.min(w),
            None => max_line_w,
        };
        Size::new(width.ceil(), total_lines.max(1) as f32 * line_h)
    }
}

/// Snaps a logical rectangle to physical pixel boundaries at paint time using `scale_factor`.
#[inline]
pub fn snap_to_pixel(rect: Rect, scale_factor: f32) -> Rect {
    let scale = if scale_factor > 0.0 && scale_factor.is_finite() {
        scale_factor
    } else {
        1.0
    };
    let x0 = (rect.x * scale).round() / scale;
    let y0 = (rect.y * scale).round() / scale;
    let x1 = ((rect.x + rect.width) * scale).round() / scale;
    let y1 = ((rect.y + rect.height) * scale).round() / scale;
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// Core layout negotiation trait.
///
/// Parents propose a size (`None` = ideal size, `Some(0.0)` = minimum, `Some(INFINITY)` = maximum),
/// children choose their own size, and the parent places each child within `bounds`.
pub trait Layout: 'static {
    fn size(&self, proposed: Size<Option<f32>>, children: &mut Children<'_>) -> Size<f32>;
    fn place(&self, bounds: Rect, children: &mut Children<'_>);
}

/// Wrapper stored in `Node::custom_layout` via `Box<dyn Any>` for user [`Layout`] types.
#[derive(Clone)]
pub struct CustomLayoutBox(pub Rc<dyn Layout>);

impl std::fmt::Debug for CustomLayoutBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomLayoutBox").finish_non_exhaustive()
    }
}

pub(crate) trait LayoutOps {
    fn measure_node(&mut self, node_id: NodeId, proposed: Size<Option<f32>>) -> Size<f32>;
    fn place_node(&mut self, node_id: NodeId, origin: Point, proposed: Size<Option<f32>>);
    fn runtime(&self) -> &Runtime;
    fn direction(&self) -> TextDirection;
}

impl LayoutOps for LayoutContext<'_> {
    fn measure_node(&mut self, node_id: NodeId, proposed: Size<Option<f32>>) -> Size<f32> {
        LayoutContext::measure_node(self, node_id, proposed)
    }

    fn place_node(&mut self, node_id: NodeId, origin: Point, proposed: Size<Option<f32>>) {
        LayoutContext::place_node(self, node_id, origin, proposed)
    }

    fn runtime(&self) -> &Runtime {
        self.runtime
    }

    fn direction(&self) -> TextDirection {
        self.direction
    }
}

/// Child proxy exposed inside [`Layout::size`] and [`Layout::place`].
pub struct ChildProxy<'a> {
    node_id: NodeId,
    engine: &'a mut dyn LayoutOps,
}

impl std::fmt::Debug for ChildProxy<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildProxy")
            .field("node_id", &self.node_id)
            .finish()
    }
}

impl ChildProxy<'_> {
    /// Returns the retained [`NodeId`] of this child.
    pub fn id(&self) -> NodeId {
        self.node_id
    }

    /// Queries how large this child wants to be given `proposed`.
    pub fn size(&mut self, proposed: Size<Option<f32>>) -> Size<f32> {
        self.engine.measure_node(self.node_id, proposed)
    }

    /// Places this child at `origin` with `proposed` size.
    pub fn place(&mut self, origin: Point, proposed: Size<Option<f32>>) {
        self.engine.place_node(self.node_id, origin, proposed);
    }

    /// Returns this child's layout priority (`0.0` by default).
    pub fn priority(&self) -> f64 {
        self.engine
            .runtime()
            .with_inner(|inner| effective_priority(inner, self.node_id))
    }

    /// Returns a child's custom `LayoutValue` metadata set via `.layout_value(...)`.
    pub fn layout_value<T: Clone + 'static>(&self) -> Option<T> {
        self.engine.runtime().with_inner(|inner| {
            inner
                .nodes
                .get(self.node_id)
                .and_then(|n| n.layout_props.layout_values.get::<T>())
        })
    }

    /// Returns custom `(x, y)` alignment guide fractions `[0.0..1.0]` if set on the child.
    pub fn alignment_guides(&self) -> (Option<f32>, Option<f32>) {
        self.engine.runtime().with_inner(|inner| {
            inner
                .nodes
                .get(self.node_id)
                .map(|n| {
                    (
                        n.layout_props.custom_alignment_x,
                        n.layout_props.custom_alignment_y,
                    )
                })
                .unwrap_or((None, None))
        })
    }

    /// Returns true if this child represents a floating popover/menu overlay.
    pub fn is_popover_overlay(&self) -> bool {
        self.engine.runtime().with_inner(|inner| {
            inner
                .nodes
                .get(self.node_id)
                .is_some_and(|n| n.accessibility.role == rustui_core::AccessibilityRole::Popover)
        })
    }

    /// Returns this child's explicit layout offset (`n.layout_props.offset`).
    pub fn offset(&self) -> Point {
        self.engine.runtime().with_inner(|inner| {
            inner
                .nodes
                .get(self.node_id)
                .map_or(Point::ZERO, |n| n.layout_props.offset)
        })
    }

    /// Resets this child's cached bounds to empty (used for offscreen pooled virtual list items).
    pub fn reset_bounds(&self) {
        self.engine.runtime().with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(self.node_id) {
                n.layout_cache.bounds = Rect::ZERO;
                n.dirty
                    .remove(DirtyFlags::LAYOUT | DirtyFlags::DESCENDANT_LAYOUT);
            }
        });
    }

    /// Returns the child's effective [`TextDirection`].
    pub fn direction(&self) -> TextDirection {
        self.engine.direction()
    }
}

/// Collection of layout children passed to [`Layout::size`] and [`Layout::place`].
pub struct Children<'a> {
    pub(crate) ids: Vec<NodeId>,
    pub(crate) direction: TextDirection,
    pub(crate) viewport: Size<f32>,
    pub(crate) engine: &'a mut dyn LayoutOps,
}

impl std::fmt::Debug for Children<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Children")
            .field("count", &self.ids.len())
            .field("direction", &self.direction)
            .finish()
    }
}

impl<'a> Children<'a> {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn direction(&self) -> TextDirection {
        self.direction
    }

    /// Returns the root viewport logical size.
    pub fn viewport(&self) -> Size<f32> {
        self.viewport
    }

    pub fn get_mut(&mut self, index: usize) -> Option<ChildProxy<'_>> {
        let node_id = *self.ids.get(index)?;
        Some(ChildProxy {
            node_id,
            engine: self.engine,
        })
    }

    pub fn child(&mut self, index: usize) -> ChildProxy<'_> {
        self.get_mut(index).expect("child index out of bounds")
    }

    pub fn measure_all(&mut self, proposed: Size<Option<f32>>) -> Vec<Size<f32>> {
        let count = self.ids.len();
        let mut sizes = Vec::with_capacity(count);
        for i in 0..count {
            sizes.push(self.child(i).size(proposed));
        }
        sizes
    }
}

fn effective_priority(inner: &RuntimeInner, node_id: NodeId) -> f64 {
    let mut cursor = Some(node_id);
    while let Some(id) = cursor {
        let Some(node) = inner.nodes.get(id) else {
            break;
        };
        if node.layout_props.layout_priority != 0.0 {
            return node.layout_props.layout_priority;
        }
        match node.kind {
            NodeKind::Container(ContainerKind::Spacer { .. }) => return -1.0,
            NodeKind::Modifier | NodeKind::Composite => {
                cursor = node.children.first().copied();
            }
            _ => break,
        }
    }
    0.0
}

fn collect_flattened_children(inner: &RuntimeInner, parent_id: NodeId, out: &mut Vec<NodeId>) {
    let Some(parent) = inner.nodes.get(parent_id) else {
        return;
    };
    let children = parent.children.clone();
    for cid in children {
        let Some(c) = inner.nodes.get(cid) else {
            continue;
        };
        if c.visual_props.hidden {
            continue;
        }
        let is_transparent = matches!(
            &c.kind,
            NodeKind::Dynamic(_) | NodeKind::Container(ContainerKind::Fragment)
        );

        if is_transparent {
            collect_flattened_children(inner, cid, out);
        } else {
            out.push(cid);
        }
    }
}

fn enclosing_stack_axis(inner: &RuntimeInner, node_id: NodeId) -> Option<bool> {
    let mut cursor = inner.nodes.get(node_id).and_then(|n| n.parent);
    while let Some(id) = cursor {
        let Some(n) = inner.nodes.get(id) else {
            break;
        };
        match &n.kind {
            NodeKind::Container(ContainerKind::HStack { .. }) => return Some(true),
            NodeKind::Container(ContainerKind::VStack { .. }) => return Some(false),
            NodeKind::Modifier
            | NodeKind::Composite
            | NodeKind::Dynamic(_)
            | NodeKind::Container(ContainerKind::Fragment) => {
                cursor = n.parent;
            }
            _ => return None,
        }
    }
    None
}

pub(crate) struct LayoutContext<'a> {
    pub(crate) runtime: &'a Runtime,
    pub(crate) measurer: &'a mut dyn TextMeasurer,
    pub(crate) direction: TextDirection,
    pub(crate) viewport: Size<f32>,
    pub(crate) laid_out_count: usize,
}

impl<'a> LayoutContext<'a> {
    fn resolve_direction(&self, node_id: NodeId) -> TextDirection {
        self.runtime.with_inner(|inner| {
            let mut cursor = Some(node_id);
            while let Some(id) = cursor {
                let Some(n) = inner.nodes.get(id) else {
                    break;
                };
                if let Some(dir) = n.layout_props.direction_override {
                    return dir;
                }
                if let Some(env_dir) = n.env.get::<TextDirection>() {
                    return env_dir;
                }
                cursor = n.parent;
            }
            self.direction
        })
    }

    pub fn measure_node(&mut self, node_id: NodeId, proposed: Size<Option<f32>>) -> Size<f32> {
        let cached = self.runtime.with_inner(|inner| {
            let n = inner.nodes.get(node_id)?;
            if !n.dirty.intersects(DirtyFlags::LAYOUT)
                && n.layout_cache.last_proposal == Some(proposed)
            {
                Some(n.layout_cache.measured_size)
            } else {
                None
            }
        });
        if let Some(size) = cached {
            return size;
        }

        self.laid_out_count += 1;
        let dir = self.resolve_direction(node_id);

        enum NodeLayoutAction {
            Hidden,
            SingleChild {
                props: rustui_core::LayoutProps,
                child: Option<NodeId>,
            },
            Control {
                kind: &'static str,
                props: rustui_core::LayoutProps,
                child: Option<NodeId>,
                clip_to_bounds: bool,
            },
            Text {
                props: rustui_core::LayoutProps,
                data: TextNodeData,
            },
            Image {
                props: rustui_core::LayoutProps,
                intrinsic: Size<f32>,
            },
            Container {
                props: rustui_core::LayoutProps,
                kind: ContainerKind,
                children: Vec<NodeId>,
            },
            Custom {
                props: rustui_core::LayoutProps,
                layout: Rc<dyn Layout>,
                children: Vec<NodeId>,
            },
            Scroll {
                props: rustui_core::LayoutProps,
                axes: rustui_core::ScrollAxes,
                child: Option<NodeId>,
            },
            VirtualList {
                props: rustui_core::LayoutProps,
                data: rustui_core::VirtualListNodeData,
            },
            Spacer {
                props: rustui_core::LayoutProps,
                min_length: f32,
                stack_axis: Option<bool>,
            },
            Divider {
                props: rustui_core::LayoutProps,
                thickness: f32,
            },
            CanvasOrShader {
                props: rustui_core::LayoutProps,
            },
        }

        let action = self.runtime.with_inner(|inner| {
            let Some(n) = inner.nodes.get(node_id) else {
                return NodeLayoutAction::Hidden;
            };
            if n.visual_props.hidden {
                return NodeLayoutAction::Hidden;
            }
            let props = n.layout_props.clone();
            match &n.kind {
                NodeKind::Modifier | NodeKind::Composite => NodeLayoutAction::SingleChild {
                    props,
                    child: n.children.first().copied(),
                },
                NodeKind::Text(t) => NodeLayoutAction::Text {
                    props,
                    data: t.clone(),
                },
                NodeKind::Image(img) => {
                    let intrinsic = match &img.source {
                        rustui_core::ImageSourceData::Rgba { width, height, .. } => {
                            Size::new(*width as f32, *height as f32)
                        }
                        _ => Size::new(64.0, 64.0),
                    };
                    NodeLayoutAction::Image { props, intrinsic }
                }
                NodeKind::Container(ContainerKind::Spacer { min_length }) => {
                    NodeLayoutAction::Spacer {
                        props,
                        min_length: *min_length,
                        stack_axis: enclosing_stack_axis(inner, node_id),
                    }
                }
                NodeKind::Container(ContainerKind::Divider { thickness, .. }) => {
                    NodeLayoutAction::Divider {
                        props,
                        thickness: *thickness,
                    }
                }
                NodeKind::Container(ContainerKind::CustomLayout) => {
                    let custom = n
                        .custom_layout
                        .as_ref()
                        .and_then(|c| c.downcast_ref::<CustomLayoutBox>())
                        .map(|b| b.0.clone());
                    let mut kids = Vec::new();
                    collect_flattened_children(inner, node_id, &mut kids);
                    if let Some(l) = custom {
                        NodeLayoutAction::Custom {
                            props,
                            layout: l,
                            children: kids,
                        }
                    } else {
                        NodeLayoutAction::Container {
                            props,
                            kind: ContainerKind::Fragment,
                            children: kids,
                        }
                    }
                }
                NodeKind::Container(ck) => {
                    let mut kids = Vec::new();
                    collect_flattened_children(inner, node_id, &mut kids);
                    NodeLayoutAction::Container {
                        props,
                        kind: ck.clone(),
                        children: kids,
                    }
                }
                NodeKind::Dynamic(_) => {
                    let mut kids = Vec::new();
                    collect_flattened_children(inner, node_id, &mut kids);
                    NodeLayoutAction::Container {
                        props,
                        kind: ContainerKind::Fragment,
                        children: kids,
                    }
                }
                NodeKind::Scroll(s) => NodeLayoutAction::Scroll {
                    props,
                    axes: s.axes,
                    child: n.children.first().copied(),
                },
                NodeKind::VirtualList(vl) => NodeLayoutAction::VirtualList {
                    props,
                    data: vl.clone(),
                },
                NodeKind::Canvas(_) | NodeKind::Shader(_) => {
                    NodeLayoutAction::CanvasOrShader { props }
                }
                NodeKind::Control(ctrl) => NodeLayoutAction::Control {
                    kind: ctrl.kind,
                    props,
                    child: n.children.first().copied(),
                    clip_to_bounds: n.visual_props.clip_to_bounds,
                },
            }
        });

        let measured = match action {
            NodeLayoutAction::Hidden => Size::ZERO,
            NodeLayoutAction::SingleChild { props, child } => {
                let inner_prop = Self::apply_props_to_proposal(&props, proposed);
                let content_size = if let Some(cid) = child {
                    self.measure_node(cid, inner_prop)
                } else {
                    Size::ZERO
                };
                Self::finalize_measured_size(&props, content_size, inner_prop, proposed)
            }
            NodeLayoutAction::Control {
                kind,
                props,
                child,
                clip_to_bounds,
            } => {
                let inner_prop = Self::apply_props_to_proposal(&props, proposed);
                let mut content_size = if let Some(cid) = child {
                    self.measure_node(cid, inner_prop)
                } else {
                    let default_size = match kind {
                        "checkbox" => Size::new(20.0, 20.0),
                        "toggle" => Size::new(44.0, 24.0),
                        "slider" => Size::new(160.0, 24.0),
                        "textfield" => Size::new(200.0, 36.0),
                        "securefield" => Size::new(200.0, 36.0),
                        "texteditor" => Size::new(320.0, 140.0),
                        _ => Size::new(80.0, 32.0),
                    };
                    let w = inner_prop
                        .width
                        .filter(|v| v.is_finite())
                        .unwrap_or(default_size.width);
                    Size::new(w, default_size.height)
                };
                if clip_to_bounds
                    && proposed.width == Some(0.0)
                    && matches!(kind, "textfield" | "securefield" | "texteditor")
                {
                    content_size.width = 0.0;
                }
                Self::finalize_measured_size(&props, content_size, inner_prop, proposed)
            }
            NodeLayoutAction::Text { props, data } => {
                let inner_prop = Self::apply_props_to_proposal(&props, proposed);
                let content_size = self.measurer.measure_text(&data, inner_prop.width);
                Self::finalize_measured_size(&props, content_size, inner_prop, proposed)
            }
            NodeLayoutAction::Image { props, intrinsic } => {
                let inner_prop = Self::apply_props_to_proposal(&props, proposed);
                let content_size = Size::new(
                    inner_prop
                        .width
                        .filter(|w| w.is_finite())
                        .unwrap_or(intrinsic.width),
                    inner_prop
                        .height
                        .filter(|h| h.is_finite())
                        .unwrap_or(intrinsic.height),
                );
                Self::finalize_measured_size(&props, content_size, inner_prop, proposed)
            }
            NodeLayoutAction::Spacer {
                props,
                min_length,
                stack_axis,
            } => {
                let inner_prop = Self::apply_props_to_proposal(&props, proposed);
                let raw = SpacerLayout { min_length }.measure(inner_prop);
                let content_size = match stack_axis {
                    Some(true) => Size::new(raw.width, 0.0),
                    Some(false) => Size::new(0.0, raw.height),
                    None => raw,
                };
                Self::finalize_measured_size(&props, content_size, inner_prop, proposed)
            }
            NodeLayoutAction::Divider { props, thickness } => {
                let inner_prop = Self::apply_props_to_proposal(&props, proposed);
                let content_size = Size::new(
                    inner_prop.width.filter(|w| w.is_finite()).unwrap_or(120.0),
                    thickness,
                );
                Self::finalize_measured_size(&props, content_size, inner_prop, proposed)
            }
            NodeLayoutAction::CanvasOrShader { props } => {
                let inner_prop = Self::apply_props_to_proposal(&props, proposed);
                let content_size = Size::new(
                    inner_prop.width.filter(|v| v.is_finite()).unwrap_or(200.0),
                    inner_prop.height.filter(|v| v.is_finite()).unwrap_or(150.0),
                );
                Self::finalize_measured_size(&props, content_size, inner_prop, proposed)
            }
            NodeLayoutAction::Scroll { props, axes, child } => {
                let inner_prop = Self::apply_props_to_proposal(&props, proposed);
                let child_size = if let Some(cid) = child {
                    let child_prop =
                        Size::new(
                            match axes {
                                rustui_core::ScrollAxes::Horizontal
                                | rustui_core::ScrollAxes::Both => None,
                                rustui_core::ScrollAxes::Vertical => inner_prop.width,
                            },
                            match axes {
                                rustui_core::ScrollAxes::Vertical
                                | rustui_core::ScrollAxes::Both => None,
                                rustui_core::ScrollAxes::Horizontal => inner_prop.height,
                            },
                        );
                    self.measure_node(cid, child_prop)
                } else {
                    Size::ZERO
                };
                self.runtime.with_inner_mut(|inner| {
                    if let Some(n) = inner.nodes.get_mut(node_id)
                        && let NodeKind::Scroll(s) = &mut n.kind
                    {
                        s.content_size = child_size;
                    }
                });
                let resolve_axis = |prop: Option<f32>, content: f32, scrolls: bool| -> f32 {
                    match prop {
                        Some(v) if v.is_infinite() => {
                            if scrolls {
                                10_000.0_f32.max(content)
                            } else {
                                content
                            }
                        }
                        Some(v) => v,
                        None => content,
                    }
                };
                let scrolls_h = matches!(
                    axes,
                    rustui_core::ScrollAxes::Horizontal | rustui_core::ScrollAxes::Both
                );
                let scrolls_v = matches!(
                    axes,
                    rustui_core::ScrollAxes::Vertical | rustui_core::ScrollAxes::Both
                );
                let content_size = Size::new(
                    resolve_axis(inner_prop.width, child_size.width, scrolls_h),
                    resolve_axis(inner_prop.height, child_size.height, scrolls_v),
                );
                Self::finalize_measured_size(&props, content_size, inner_prop, proposed)
            }
            NodeLayoutAction::VirtualList { props, data } => {
                let inner_prop = Self::apply_props_to_proposal(&props, proposed);
                let mut kids = self.runtime.with_inner(|inner| {
                    inner
                        .nodes
                        .get(node_id)
                        .map(|n| n.children.clone())
                        .unwrap_or_default()
                });
                let layout = LazyVStackLayout::from_data(&data);
                let mut children = Children {
                    ids: std::mem::take(&mut kids),
                    direction: dir,
                    viewport: self.viewport,
                    engine: self,
                };
                let content_size = layout.size(inner_prop, &mut children);
                Self::finalize_measured_size(&props, content_size, inner_prop, proposed)
            }
            NodeLayoutAction::Custom {
                props,
                layout,
                children: kids,
            } => {
                let inner_prop = Self::apply_props_to_proposal(&props, proposed);
                let mut children = Children {
                    ids: kids,
                    direction: dir,
                    viewport: self.viewport,
                    engine: self,
                };
                let content_size = layout.size(inner_prop, &mut children);
                Self::finalize_measured_size(&props, content_size, inner_prop, proposed)
            }
            NodeLayoutAction::Container {
                props,
                kind: ck,
                children: kids,
            } => {
                let inner_prop = Self::apply_props_to_proposal(&props, proposed);
                let mut children = Children {
                    ids: kids,
                    direction: dir,
                    viewport: self.viewport,
                    engine: self,
                };
                let content_size = match ck {
                    ContainerKind::VStack { spacing, alignment } => {
                        VStackLayout { spacing, alignment }.size(inner_prop, &mut children)
                    }
                    ContainerKind::HStack { spacing, alignment } => {
                        HStackLayout { spacing, alignment }.size(inner_prop, &mut children)
                    }
                    ContainerKind::ZStack { alignment } => {
                        ZStackLayout { alignment }.size(inner_prop, &mut children)
                    }
                    ContainerKind::Grid {
                        columns,
                        h_spacing,
                        v_spacing,
                    } => GridLayout {
                        columns,
                        h_spacing,
                        v_spacing,
                    }
                    .size(inner_prop, &mut children),
                    ContainerKind::Fragment => {
                        if children.is_empty() {
                            Size::ZERO
                        } else {
                            let sizes = children.measure_all(inner_prop);
                            let mut w = 0.0f32;
                            let mut h = 0.0f32;
                            for s in sizes {
                                w = w.max(s.width);
                                h += s.height;
                            }
                            Size::new(w, h)
                        }
                    }
                    ContainerKind::Spacer { min_length } => {
                        SpacerLayout { min_length }.size(inner_prop, &mut children)
                    }
                    ContainerKind::Divider { thickness, .. } => Size::new(
                        inner_prop.width.filter(|w| w.is_finite()).unwrap_or(120.0),
                        thickness,
                    ),
                    ContainerKind::CustomLayout => Size::ZERO,
                };
                Self::finalize_measured_size(&props, content_size, inner_prop, proposed)
            }
        };

        self.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(node_id) {
                n.layout_cache.last_proposal = Some(proposed);
                n.layout_cache.measured_size = measured;
            }
        });
        measured
    }

    fn apply_props_to_proposal(
        props: &rustui_core::LayoutProps,
        proposed: Size<Option<f32>>,
    ) -> Size<Option<f32>> {
        let mut p = proposed;

        if props.fixed_horizontal {
            p.width = None;
        }
        if props.fixed_vertical {
            p.height = None;
        }

        if let Some(frame) = &props.frame {
            if let Some(w) = frame.ideal_width.filter(|_| p.width.is_none()) {
                p.width = Some(w);
            }
            if let Some(h) = frame.ideal_height.filter(|_| p.height.is_none()) {
                p.height = Some(h);
            }
            if let Some(max_w) = frame.max_width {
                p.width = match (p.width, max_w.is_finite()) {
                    (Some(w), true) => Some(w.min(max_w)),
                    (Some(w), false) => Some(w),
                    (None, true) => Some(max_w),
                    (None, false) => None,
                };
            }
            if let Some(min_w) = frame.min_width {
                p.width = p.width.map(|w| w.max(min_w));
            }
            if let Some(max_h) = frame.max_height {
                p.height = match (p.height, max_h.is_finite()) {
                    (Some(h), true) => Some(h.min(max_h)),
                    (Some(h), false) => Some(h),
                    (None, true) => Some(max_h),
                    (None, false) => None,
                };
            }
            if let Some(min_h) = frame.min_height {
                p.height = p.height.map(|h| h.max(min_h));
            }
        }

        let h_pad = props.padding.horizontal();
        let v_pad = props.padding.vertical();
        p.width = p.width.map(|w| {
            if w.is_finite() {
                (w - h_pad).max(0.0)
            } else {
                w
            }
        });
        p.height = p.height.map(|h| {
            if h.is_finite() {
                (h - v_pad).max(0.0)
            } else {
                h
            }
        });

        if let Some((ratio, mode)) = props.aspect_ratio
            && ratio > 0.0
        {
            match (p.width, p.height) {
                (Some(w), Some(h)) if w.is_finite() && h.is_finite() => {
                    let target_h = w / ratio;
                    let target_w = h * ratio;
                    match mode {
                        rustui_core::ContentMode::Fit => {
                            if target_h <= h {
                                p.height = Some(target_h);
                            } else {
                                p.width = Some(target_w);
                            }
                        }
                        rustui_core::ContentMode::Fill => {
                            if target_h >= h {
                                p.height = Some(target_h);
                            } else {
                                p.width = Some(target_w);
                            }
                        }
                    }
                }
                (Some(w), None) if w.is_finite() => {
                    p.height = Some(w / ratio);
                }
                (None, Some(h)) if h.is_finite() => {
                    p.width = Some(h * ratio);
                }
                _ => {}
            }
        }

        p
    }

    fn finalize_measured_size(
        props: &rustui_core::LayoutProps,
        mut content_size: Size<f32>,
        inner_prop: Size<Option<f32>>,
        proposed: Size<Option<f32>>,
    ) -> Size<f32> {
        if let Some((ratio, _mode)) = props.aspect_ratio
            && ratio > 0.0
        {
            if let Some(w) = inner_prop.width.filter(|v| v.is_finite()) {
                content_size = Size::new(w, w / ratio);
            } else if let Some(h) = inner_prop.height.filter(|v| v.is_finite()) {
                content_size = Size::new(h * ratio, h);
            }
        }

        let mut w = content_size.width + props.padding.horizontal();
        let mut h = content_size.height + props.padding.vertical();

        if let Some(frame) = &props.frame {
            if let Some(ideal_w) = frame.ideal_width
                && (proposed.width.is_none() || props.fixed_horizontal)
            {
                w = ideal_w;
            }
            if let Some(ideal_h) = frame.ideal_height
                && (proposed.height.is_none() || props.fixed_vertical)
            {
                h = ideal_h;
            }
            if let Some(max_w) = frame.max_width {
                if max_w.is_infinite() {
                    match proposed.width {
                        Some(pw) if pw.is_infinite() => w = w.max(10_000.0),
                        Some(pw) if pw.is_finite() => w = w.max(pw),
                        _ => {}
                    }
                } else {
                    w = w.min(max_w);
                }
            }
            if let Some(min_w) = frame.min_width {
                w = w.max(min_w);
            }
            if let Some(max_h) = frame.max_height {
                if max_h.is_infinite() {
                    match proposed.height {
                        Some(ph) if ph.is_infinite() => h = h.max(10_000.0),
                        Some(ph) if ph.is_finite() => h = h.max(ph),
                        _ => {}
                    }
                } else {
                    h = h.min(max_h);
                }
            }
            if let Some(min_h) = frame.min_height {
                h = h.max(min_h);
            }
        }

        Size::new(w, h)
    }

    pub fn place_node(&mut self, node_id: NodeId, origin: Point, proposed: Size<Option<f32>>) {
        let fast_path = self.runtime.with_inner(|inner| {
            let n = inner.nodes.get(node_id)?;
            if n.layout_cache.last_proposal != Some(proposed) {
                return None;
            }
            if (n.layout_cache.bounds.width - n.layout_cache.measured_size.width).abs() > 0.01
                || (n.layout_cache.bounds.height - n.layout_cache.measured_size.height).abs() > 0.01
            {
                return None;
            }
            let exp_x = origin.x + n.layout_props.offset.x;
            let exp_y = origin.y + n.layout_props.offset.y;
            if (n.layout_cache.bounds.x - exp_x).abs() > 0.01
                || (n.layout_cache.bounds.y - exp_y).abs() > 0.01
            {
                return None;
            }
            if !n
                .dirty
                .intersects(DirtyFlags::LAYOUT | DirtyFlags::DESCENDANT_LAYOUT)
            {
                return Some(Vec::new());
            }
            if !n.dirty.intersects(DirtyFlags::LAYOUT) {
                let dirty_kids: Vec<(NodeId, Point, Size<Option<f32>>)> = n
                    .children
                    .iter()
                    .copied()
                    .filter_map(|cid| {
                        let c = inner.nodes.get(cid)?;
                        if c.dirty
                            .intersects(DirtyFlags::LAYOUT | DirtyFlags::DESCENDANT_LAYOUT)
                        {
                            let o = Point::new(
                                c.layout_cache.bounds.x - c.layout_props.offset.x,
                                c.layout_cache.bounds.y - c.layout_props.offset.y,
                            );
                            let p = c.layout_cache.last_proposal.unwrap_or(Size::new(
                                Some(c.layout_cache.bounds.width),
                                Some(c.layout_cache.bounds.height),
                            ));
                            Some((cid, o, p))
                        } else {
                            None
                        }
                    })
                    .collect();
                return Some(dirty_kids);
            }
            None
        });

        if let Some(dirty_kids) = fast_path {
            self.runtime.with_inner_mut(|inner| {
                if let Some(n) = inner.nodes.get_mut(node_id) {
                    n.dirty.remove(DirtyFlags::DESCENDANT_LAYOUT);
                }
            });
            for (cid, child_origin, child_prop) in dirty_kids {
                self.place_node(cid, child_origin, child_prop);
            }
            return;
        }

        let size = self.measure_node(node_id, proposed);
        let dir = self.resolve_direction(node_id);

        enum PlaceAction {
            Hidden,
            Modifier {
                props: rustui_core::LayoutProps,
                child: Option<NodeId>,
            },
            Container(rustui_core::Edges, ContainerKind, Vec<NodeId>),
            Custom(rustui_core::Edges, Rc<dyn Layout>, Vec<NodeId>),
            Scroll(
                rustui_core::Edges,
                rustui_core::ScrollNodeData,
                Option<NodeId>,
            ),
            VirtualList(
                rustui_core::Edges,
                rustui_core::VirtualListNodeData,
                Vec<NodeId>,
            ),
            Leaf,
        }

        let (offset, action) = self.runtime.with_inner_mut(|inner| {
            let Some(n) = inner.nodes.get_mut(node_id) else {
                return (Point::ZERO, PlaceAction::Hidden);
            };
            let offset = n.layout_props.offset;
            let bounds = Rect::new(
                origin.x + offset.x,
                origin.y + offset.y,
                size.width,
                size.height,
            );
            n.layout_cache.bounds = bounds;
            n.dirty
                .remove(DirtyFlags::LAYOUT | DirtyFlags::DESCENDANT_LAYOUT);

            if n.visual_props.hidden {
                return (offset, PlaceAction::Hidden);
            }

            let padding = n.layout_props.padding;
            let act = match &n.kind {
                NodeKind::Modifier | NodeKind::Composite | NodeKind::Control(_) => {
                    PlaceAction::Modifier {
                        props: n.layout_props.clone(),
                        child: n.children.first().copied(),
                    }
                }
                NodeKind::Container(ContainerKind::CustomLayout) => {
                    let custom = n
                        .custom_layout
                        .as_ref()
                        .and_then(|c| c.downcast_ref::<CustomLayoutBox>())
                        .map(|b| b.0.clone());
                    let mut kids = Vec::new();
                    collect_flattened_children(inner, node_id, &mut kids);
                    if let Some(l) = custom {
                        PlaceAction::Custom(padding, l, kids)
                    } else {
                        PlaceAction::Container(padding, ContainerKind::Fragment, kids)
                    }
                }
                NodeKind::Container(ck) => {
                    let ck = ck.clone();
                    let mut kids = Vec::new();
                    collect_flattened_children(inner, node_id, &mut kids);
                    PlaceAction::Container(padding, ck, kids)
                }
                NodeKind::Dynamic(_) => {
                    let mut kids = Vec::new();
                    collect_flattened_children(inner, node_id, &mut kids);
                    PlaceAction::Container(padding, ContainerKind::Fragment, kids)
                }
                NodeKind::Scroll(s) => {
                    PlaceAction::Scroll(padding, *s, n.children.first().copied())
                }
                NodeKind::VirtualList(vl) => {
                    PlaceAction::VirtualList(padding, vl.clone(), n.children.clone())
                }
                _ => PlaceAction::Leaf,
            };
            (offset, act)
        });

        let bounds = Rect::new(
            origin.x + offset.x,
            origin.y + offset.y,
            size.width,
            size.height,
        );

        match action {
            PlaceAction::Hidden | PlaceAction::Leaf => {}
            PlaceAction::Modifier { props, child } => {
                if let Some(cid) = child {
                    let inner_bounds = bounds.inset(props.padding);
                    let child_prop = Size::new(Some(inner_bounds.width), Some(inner_bounds.height));
                    let child_size = self.measure_node(cid, child_prop);
                    let alignment = props
                        .frame
                        .as_ref()
                        .map(|f| f.alignment)
                        .unwrap_or(rustui_core::Alignment::CENTER);
                    let ax = alignment.horizontal.factor(dir);
                    let ay = alignment.vertical.factor();
                    let cx = inner_bounds.x + (inner_bounds.width - child_size.width) * ax;
                    let cy = inner_bounds.y + (inner_bounds.height - child_size.height) * ay;
                    self.place_node(cid, Point::new(cx, cy), child_prop);
                }
            }
            PlaceAction::Scroll(padding, scroll, child) => {
                if let Some(cid) = child {
                    let inner_bounds = bounds.inset(padding);
                    let child_prop =
                        Size::new(
                            match scroll.axes {
                                rustui_core::ScrollAxes::Horizontal
                                | rustui_core::ScrollAxes::Both => Some(scroll.content_size.width),
                                rustui_core::ScrollAxes::Vertical => Some(inner_bounds.width),
                            },
                            match scroll.axes {
                                rustui_core::ScrollAxes::Vertical
                                | rustui_core::ScrollAxes::Both => Some(scroll.content_size.height),
                                rustui_core::ScrollAxes::Horizontal => Some(inner_bounds.height),
                            },
                        );
                    let child_origin = Point::new(
                        inner_bounds.x - scroll.offset.x,
                        inner_bounds.y - scroll.offset.y,
                    );
                    self.place_node(cid, child_origin, child_prop);
                }
            }
            PlaceAction::VirtualList(padding, vl, kids) => {
                let inner_bounds = bounds.inset(padding);
                let layout = LazyVStackLayout::from_data(&vl);
                let mut children = Children {
                    ids: kids,
                    direction: dir,
                    viewport: self.viewport,
                    engine: self,
                };
                layout.place(inner_bounds, &mut children);
            }
            PlaceAction::Custom(padding, layout, kids) => {
                let inner_bounds = bounds.inset(padding);
                let mut children = Children {
                    ids: kids,
                    direction: dir,
                    viewport: self.viewport,
                    engine: self,
                };
                layout.place(inner_bounds, &mut children);
            }
            PlaceAction::Container(padding, ck, kids) => {
                let inner_bounds = bounds.inset(padding);
                let mut children = Children {
                    ids: kids,
                    direction: dir,
                    viewport: self.viewport,
                    engine: self,
                };
                match ck {
                    ContainerKind::VStack { spacing, alignment } => {
                        VStackLayout { spacing, alignment }.place(inner_bounds, &mut children);
                    }
                    ContainerKind::HStack { spacing, alignment } => {
                        HStackLayout { spacing, alignment }.place(inner_bounds, &mut children);
                    }
                    ContainerKind::ZStack { alignment } => {
                        ZStackLayout { alignment }.place(inner_bounds, &mut children);
                    }
                    ContainerKind::Grid {
                        columns,
                        h_spacing,
                        v_spacing,
                    } => {
                        GridLayout {
                            columns,
                            h_spacing,
                            v_spacing,
                        }
                        .place(inner_bounds, &mut children);
                    }
                    ContainerKind::Fragment => {
                        let mut y = inner_bounds.y;
                        for i in 0..children.len() {
                            let c_prop = Size::new(Some(inner_bounds.width), None);
                            let sz = children.child(i).size(c_prop);
                            children
                                .child(i)
                                .place(Point::new(inner_bounds.x, y), c_prop);
                            y += sz.height;
                        }
                    }
                    ContainerKind::Spacer { .. }
                    | ContainerKind::Divider { .. }
                    | ContainerKind::CustomLayout => {}
                }
            }
        }

        // Sync intermediate transparent wrapper node bounds (`Dynamic` / `Fragment`) with their children.
        self.sync_transparent_descendant_bounds(node_id);
    }

    fn sync_transparent_descendant_bounds(&self, parent_id: NodeId) {
        self.runtime.with_inner_mut(|inner| {
            sync_transparent_recursive(inner, parent_id);
        });
    }
}

fn sync_transparent_recursive(inner: &mut RuntimeInner, parent_id: NodeId) {
    let children = inner
        .nodes
        .get(parent_id)
        .map(|n| n.children.clone())
        .unwrap_or_default();
    for cid in children {
        let is_transparent = inner.nodes.get(cid).is_some_and(|c| {
            matches!(
                c.kind,
                NodeKind::Dynamic(_) | NodeKind::Container(ContainerKind::Fragment)
            )
        });
        if is_transparent {
            sync_transparent_recursive(inner, cid);
            let union_rect = compute_subtree_union(inner, cid);
            if let Some(c) = inner.nodes.get_mut(cid) {
                c.layout_cache.bounds = union_rect;
                c.dirty.remove(DirtyFlags::LAYOUT);
            }
        }
    }
}

fn compute_subtree_union(inner: &RuntimeInner, node_id: NodeId) -> Rect {
    let Some(node) = inner.nodes.get(node_id) else {
        return Rect::ZERO;
    };
    let mut acc = Rect::ZERO;
    for &cid in &node.children {
        if let Some(child) = inner.nodes.get(cid) {
            let r = if matches!(
                child.kind,
                NodeKind::Dynamic(_) | NodeKind::Container(ContainerKind::Fragment)
            ) {
                compute_subtree_union(inner, cid)
            } else {
                child.layout_cache.bounds
            };
            acc = acc.union(&r);
        }
    }
    acc
}

/// Top-level layout coordinator executing measure + place across the retained node tree.
#[derive(Debug, Default)]
pub struct LayoutEngine {
    pub direction: TextDirection,
}

impl LayoutEngine {
    pub fn new() -> Self {
        Self {
            direction: TextDirection::Ltr,
        }
    }

    pub fn with_direction(direction: TextDirection) -> Self {
        Self { direction }
    }

    /// Runs incremental layout on `root` within `viewport`, returning total laid-out node count.
    pub fn layout(
        &self,
        runtime: &Runtime,
        root: NodeId,
        viewport: Size<f32>,
        measurer: &mut dyn TextMeasurer,
    ) -> usize {
        let start = Instant::now();
        let dir = runtime.with_inner(|inner| {
            inner
                .nodes
                .get(root)
                .and_then(|n| n.env.get::<TextDirection>())
                .unwrap_or(self.direction)
        });

        let mut ctx = LayoutContext {
            runtime,
            measurer,
            direction: dir,
            viewport,
            laid_out_count: 0,
        };
        let proposed = Size::new(Some(viewport.width), Some(viewport.height));
        runtime.enter(|| {
            ctx.place_node(root, Point::ZERO, proposed);
        });
        let laid_out = ctx.laid_out_count;

        let elapsed = start.elapsed();
        runtime.with_inner_mut(|inner| {
            inner.scheduler.metrics.laid_out_nodes = laid_out;
            inner.scheduler.metrics.layout_time = elapsed;
        });

        laid_out
    }
}
