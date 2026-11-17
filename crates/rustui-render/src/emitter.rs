//! Emits a [`DisplayList`] from the retained UI [`rustui_core::Node`] tree.

use std::time::Instant;

use rustui_core::{
    AccessibilityRole, ContainerKind, DirtyFlags, NodeId, NodeKind, Point, Rect, Runtime,
    TextNodeData, ThemePalette,
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
    let root_palette = runtime
        .with_inner(|inner| {
            inner
                .nodes
                .get(root)
                .and_then(|n| n.env.get::<ThemePalette>())
        })
        .unwrap_or_default();
    let mut deferred_overlays = Vec::new();

    runtime.enter(|| {
        emit_node_recursive(
            runtime,
            root,
            root_palette,
            false,
            resources,
            &mut painter,
            &mut list,
            &mut deferred_overlays,
        );
        while !deferred_overlays.is_empty() {
            let batch = std::mem::take(&mut deferred_overlays);
            for (overlay_id, overlay_palette) in batch {
                emit_node_recursive(
                    runtime,
                    overlay_id,
                    overlay_palette,
                    true,
                    resources,
                    &mut painter,
                    &mut list,
                    &mut deferred_overlays,
                );
            }
        }
    });

    let elapsed = start.elapsed();
    let cmd_count = list.commands.len();
    runtime.with_inner_mut(|inner| {
        inner.scheduler.metrics.paint_commands = cmd_count;
        inner.scheduler.metrics.paint_time = elapsed;
    });
    list
}

#[allow(clippy::too_many_arguments)]
fn emit_node_recursive(
    runtime: &Runtime,
    node_id: NodeId,
    inherited_palette: ThemePalette,
    allow_overlay_root: bool,
    resources: &mut ResourceLoader,
    text_painter: &mut Option<&mut dyn TextPainter>,
    list: &mut DisplayList,
    deferred_overlays: &mut Vec<(NodeId, ThemePalette)>,
) {
    enum LeafEmit {
        None,
        Divider(rustui_core::Color),
        Image(rustui_core::ImageNodeData),
        Text(TextNodeData),
        Canvas(rustui_core::CanvasPaintFn),
        Shader(rustui_core::ShaderNodeData),
        ScrollClip(Rect),
    }

    let snapshot = runtime.with_inner_mut(|inner| {
        let (hidden, parent_id, is_popover, local_palette) = {
            let node = inner.nodes.get(node_id)?;
            (
                node.visual_props.hidden,
                node.parent,
                node.accessibility.role == AccessibilityRole::Popover,
                node.env.get::<ThemePalette>(),
            )
        };
        let active_palette = local_palette.unwrap_or(inherited_palette);
        if hidden {
            if let Some(node) = inner.nodes.get_mut(node_id) {
                node.dirty.remove(DirtyFlags::PAINT);
            }
            return None;
        }
        if is_popover && !allow_overlay_root {
            deferred_overlays.push((node_id, active_palette));
            return None;
        }

        let parent_focused = parent_id
            .and_then(|pid| inner.nodes.get(pid))
            .is_some_and(|p| p.focus.focused);

        let node = inner.nodes.get_mut(node_id)?;
        let bounds = node.bounds();
        let mut visual = node.visual_props.clone();
        if let NodeKind::Control(ctrl) = &node.kind
            && !ctrl.is_disabled
            && let Some(bg) = visual.background
        {
            let resolved = active_palette.resolve(bg);
            visual.background = Some(if ctrl.is_pressed {
                if resolved.a > 0.01 {
                    resolved.lerp(rustui_core::Color::BLACK, 0.15)
                } else {
                    active_palette.border.with_alpha(0.5)
                }
            } else if ctrl.is_hovered {
                if resolved.a > 0.01 {
                    resolved.lerp(rustui_core::Color::WHITE, 0.10)
                } else {
                    active_palette.surface.with_alpha(0.75)
                }
            } else {
                resolved
            });
        }
        let show_focus_ring =
            node.focus.focusable && node.focus.focused && bounds.width > 0.0 && bounds.height > 0.0;
        let children = node.children.clone();
        node.dirty.remove(DirtyFlags::PAINT);
        let leaf = match &node.kind {
            NodeKind::Container(ContainerKind::Divider { color, .. }) => LeafEmit::Divider(*color),
            NodeKind::Image(img) => LeafEmit::Image(img.clone()),
            NodeKind::Text(t) => {
                let mut cloned = t.clone();
                if !parent_focused && !node.focus.focused {
                    cloned.caret = None;
                }
                LeafEmit::Text(cloned)
            }
            NodeKind::Canvas(c) => LeafEmit::Canvas(c.painter.clone()),
            NodeKind::Shader(s) => LeafEmit::Shader(s.clone()),
            NodeKind::Scroll(_) => LeafEmit::ScrollClip(bounds),
            _ => LeafEmit::None,
        };
        Some((
            bounds,
            visual,
            children,
            leaf,
            active_palette,
            show_focus_ring,
        ))
    });

    let Some((bounds, visual, children, leaf, palette, show_focus_ring)) = snapshot else {
        return;
    };

    let has_area = bounds.width > 0.0 && bounds.height > 0.0;
    let needs_layer = visual.opacity < 0.999;
    if needs_layer {
        list.push_layer(bounds, visual.opacity, BlendMode::Normal);
    }

    if has_area {
        if let Some(shadow) = visual.shadow {
            list.draw_box_shadow(
                bounds,
                visual.corner_radii.max_radius(),
                shadow.offset,
                shadow.blur_radius,
                palette.resolve(shadow.color),
            );
        }

        if let Some(bg) = visual.background {
            list.fill_rounded_rect(bounds, visual.corner_radii, palette.resolve(bg));
        }
    }

    if visual.clip_to_bounds {
        if visual.corner_radii.max_radius() > 0.0 {
            list.push_clip_rounded_rect(bounds, visual.corner_radii);
        } else {
            list.push_clip_rect(bounds);
        }
    }

    let mut pushed_scroll_clip = false;
    match leaf {
        LeafEmit::None => {}
        LeafEmit::Divider(c) => {
            if has_area {
                list.fill_rect(bounds, palette.resolve(c));
            }
        }
        LeafEmit::Image(img) => {
            if has_area {
                if let Some(img_id) = resources.resolve_source(&img.source) {
                    let tint = img
                        .tint
                        .map(|t| palette.resolve(t))
                        .unwrap_or(rustui_core::Color::WHITE);
                    list.draw_image(bounds, img_id, Rect::new(0.0, 0.0, 1.0, 1.0), tint);
                } else {
                    list.fill_rounded_rect(
                        bounds,
                        visual.corner_radii,
                        rustui_core::Color::rgba(0.5, 0.5, 0.55, 0.25),
                    );
                }
            }
        }
        LeafEmit::Text(mut t) => {
            if has_area {
                let c = palette.resolve(t.color.unwrap_or_else(rustui_core::Color::primary));
                t.color = Some(c);
                if let Some(tp) = text_painter.as_deref_mut() {
                    tp.paint_text(&t, bounds, list);
                } else if !t.text.is_empty() {
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
            if has_area {
                paint_fn(bounds, list as &mut dyn std::any::Any);
            }
        }
        LeafEmit::Shader(s) => {
            if has_area {
                list.draw_custom_shader(bounds, s.shader_id, s.wgsl, s.uniform_data);
            }
        }
        LeafEmit::ScrollClip(clip_rect) => {
            list.push_clip_rect(clip_rect);
            pushed_scroll_clip = true;
        }
    }

    for cid in children {
        emit_node_recursive(
            runtime,
            cid,
            palette,
            allow_overlay_root,
            resources,
            text_painter,
            list,
            deferred_overlays,
        );
    }

    if pushed_scroll_clip {
        list.pop_clip();
    }
    if visual.clip_to_bounds {
        list.pop_clip();
    }
    if has_area {
        if let Some((bw, bc)) = visual.border {
            list.stroke_rounded_rect(bounds, visual.corner_radii, bw, palette.resolve(bc));
        }
        if show_focus_ring {
            list.stroke_rounded_rect(bounds, visual.corner_radii, 1.5, palette.accent);
        }
    }
    if needs_layer {
        list.pop_layer();
    }

    let _ = Point::ZERO;
}
