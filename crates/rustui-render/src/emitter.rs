//! Emits a [`DisplayList`] from the retained UI [`Node`] tree.

use std::time::Instant;

use rustui_core::{
    ContainerKind, DirtyFlags, NodeId, NodeKind, Point, Rect, Runtime, RuntimeInner, TextNodeData,
};

use crate::display_list::{BlendMode, DisplayList};
use crate::resources::ResourceLoader;

/// Callback trait implemented by `rustui-text` (or fallback painters) to emit glyphs for [`TextNodeData`].
pub trait TextPainter {
    fn paint_text(&mut self, data: &TextNodeData, bounds: Rect, list: &mut DisplayList);
}

/// Emits a [`DisplayList`] for the subtree rooted at `root`.
pub fn emit_display_list(
    runtime: &Runtime,
    root: NodeId,
    scale_factor: f32,
    resources: &mut ResourceLoader,
    text_painter: Option<&mut dyn TextPainter>,
) -> DisplayList {
    let start = Instant::now();
    let mut list = DisplayList::new(scale_factor);
    let mut painter = text_painter;

    runtime.with_inner_mut(|inner| {
        emit_node_recursive(inner, root, resources, &mut painter, &mut list);
    });

    let elapsed = start.elapsed();
    let cmd_count = list.commands.len();
    runtime.with_inner_mut(|inner| {
        inner.scheduler.metrics.paint_commands = cmd_count;
        inner.scheduler.metrics.paint_time = elapsed;
    });
    list
}

fn emit_node_recursive(
    inner: &mut RuntimeInner,
    node_id: NodeId,
    resources: &mut ResourceLoader,
    text_painter: &mut Option<&mut dyn TextPainter>,
    list: &mut DisplayList,
) {
    let Some(node) = inner.nodes.get_mut(node_id) else {
        return;
    };
    if node.visual_props.hidden {
        node.dirty.remove(DirtyFlags::PAINT);
        return;
    }

    let bounds = node.bounds();
    let visual = node.visual_props.clone();
    let children = node.children.clone();
    node.dirty.remove(DirtyFlags::PAINT);

    let needs_layer = visual.opacity < 0.999;
    if needs_layer {
        list.push_layer(bounds, visual.opacity, BlendMode::Normal);
    }

    if let Some(shadow) = visual.shadow {
        list.draw_box_shadow(
            bounds,
            visual.corner_radii.max_radius(),
            shadow.offset,
            shadow.blur_radius,
            shadow.color,
        );
    }

    if let Some(bg) = visual.background {
        list.fill_rounded_rect(bounds, visual.corner_radii, bg);
    }

    if visual.clip_to_bounds {
        if visual.corner_radii.max_radius() > 0.0 {
            list.push_clip_rounded_rect(bounds, visual.corner_radii);
        } else {
            list.push_clip_rect(bounds);
        }
    }

    // Clone node payload briefly without holding `inner.nodes` across canvas callbacks.
    enum LeafEmit {
        None,
        Divider(rustui_core::Color),
        Image(rustui_core::ImageNodeData),
        Text(TextNodeData),
        Canvas(rustui_core::CanvasPaintFn),
        Shader(rustui_core::ShaderNodeData),
        ScrollClip(Rect),
    }

    let leaf = inner
        .nodes
        .get(node_id)
        .map(|n| match &n.kind {
            NodeKind::Container(ContainerKind::Divider { color, .. }) => LeafEmit::Divider(*color),
            NodeKind::Image(img) => LeafEmit::Image(img.clone()),
            NodeKind::Text(t) => LeafEmit::Text(t.clone()),
            NodeKind::Canvas(c) => LeafEmit::Canvas(c.painter.clone()),
            NodeKind::Shader(s) => LeafEmit::Shader(s.clone()),
            NodeKind::Scroll(_) => LeafEmit::ScrollClip(bounds),
            _ => LeafEmit::None,
        })
        .unwrap_or(LeafEmit::None);

    let mut pushed_scroll_clip = false;
    match leaf {
        LeafEmit::None => {}
        LeafEmit::Divider(c) => {
            list.fill_rect(bounds, c);
        }
        LeafEmit::Image(img) => {
            if let Some(img_id) = resources.resolve_source(&img.source) {
                let tint = img.tint.unwrap_or(rustui_core::Color::WHITE);
                list.draw_image(bounds, img_id, Rect::new(0.0, 0.0, 1.0, 1.0), tint);
            } else {
                list.fill_rounded_rect(
                    bounds,
                    visual.corner_radii,
                    rustui_core::Color::rgba(0.5, 0.5, 0.55, 0.25),
                );
            }
        }
        LeafEmit::Text(t) => {
            if let Some(tp) = text_painter.as_deref_mut() {
                tp.paint_text(&t, bounds, list);
            } else {
                // Minimal fallback marker when rendering `rustui-render` standalone without `rustui-text`.
                let c = t.color.unwrap_or(rustui_core::Color::BLACK);
                if !t.text.is_empty() && bounds.width > 0.0 && bounds.height > 0.0 {
                    let glyph_h = (t.font.size * 0.65).min(bounds.height);
                    list.fill_rounded_rect(
                        Rect::new(
                            bounds.x,
                            bounds.y + (bounds.height - glyph_h) * 0.5,
                            bounds.width.min((t.text.len() as f32) * t.font.size * 0.5),
                            glyph_h,
                        ),
                        rustui_core::CornerRadii::all(2.0),
                        c.with_alpha(c.a * 0.18),
                    );
                }
            }
        }
        LeafEmit::Canvas(paint_fn) => {
            paint_fn(bounds, list as &mut dyn std::any::Any);
        }
        LeafEmit::Shader(s) => {
            list.draw_custom_shader(bounds, s.shader_id, s.wgsl, s.uniform_data);
        }
        LeafEmit::ScrollClip(clip_rect) => {
            list.push_clip_rect(clip_rect);
            pushed_scroll_clip = true;
        }
    }

    for cid in children {
        emit_node_recursive(inner, cid, resources, text_painter, list);
    }

    if pushed_scroll_clip {
        list.pop_clip();
    }
    if visual.clip_to_bounds {
        list.pop_clip();
    }
    if let Some((bw, bc)) = visual.border {
        list.stroke_rounded_rect(bounds, visual.corner_radii, bw, bc);
    }
    if needs_layer {
        list.pop_layer();
    }

    let _ = Point::ZERO;
}
