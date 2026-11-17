//! Display list, wgpu/Vello GPU backend, glyph atlas, damage tracker, resource loader, and inspector.

pub mod atlas;
pub mod backend;
pub mod damage;
pub mod display_list;
pub mod emitter;
pub mod inspector;
pub mod resources;

pub use atlas::{AtlasGlyphEntry, GlyphAtlas};
pub use backend::{
    GpuDiagnostics, SoftwareRasterizer, WgpuRenderer, encode_rgba_to_png, platform_backends,
    probe_gpu_adapter,
};
pub use damage::DamageTracker;
pub use display_list::{
    BatchKey, BlendMode, DisplayCommand, DisplayList, FontBlob, GlyphKey, GlyphRunItem,
    GradientKind, GradientStop, ImageId, PathOp, SubpixelBin,
};
pub use emitter::{TextPainter, emit_display_list};
pub use inspector::{
    InspectorFrameSnapshot, InspectorNodeSnapshot, InspectorOverlay, InspectorServer,
    export_perfetto_trace,
};
pub use resources::{AsyncImageState, DecodedImage, ResourceLoader};

#[cfg(test)]
mod tests {
    use rustui_core::{Color, CornerRadii, Edges, Point, Rect};

    use super::*;

    #[test]
    fn headless_renders_rounded_rects_gradients_paths_and_image_to_png() {
        let mut list = DisplayList::new(1.0);
        let mut resources = ResourceLoader::new();
        let mut atlas = GlyphAtlas::default();

        // 4x4 RGBA checkerboard image
        let mut rgba = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4 {
            for x in 0..4 {
                if (x + y) % 2 == 0 {
                    rgba.extend_from_slice(&[255, 220, 40, 255]);
                } else {
                    rgba.extend_from_slice(&[40, 120, 255, 255]);
                }
            }
        }
        resources.insert_rgba(1, 4, 4, rgba);

        list.fill_rounded_rect(
            Rect::new(10.0, 10.0, 100.0, 60.0),
            CornerRadii::all(12.0),
            Color::RED,
        );
        list.stroke_rounded_rect(
            Rect::new(10.0, 10.0, 100.0, 60.0),
            CornerRadii::all(12.0),
            2.0,
            Color::WHITE,
        );
        list.draw_gradient(
            Rect::new(120.0, 10.0, 80.0, 60.0),
            CornerRadii::all(8.0),
            GradientKind::Linear {
                start: Point::new(0.0, 0.0),
                end: Point::new(80.0, 60.0),
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: Color::BLUE,
                    },
                    GradientStop {
                        offset: 1.0,
                        color: Color::GREEN,
                    },
                ],
            },
        );
        list.draw_image(
            Rect::new(210.0, 10.0, 40.0, 40.0),
            1,
            Rect::new(0.0, 0.0, 1.0, 1.0),
            Color::WHITE,
        );
        list.draw_nine_slice(
            Rect::new(10.0, 80.0, 80.0, 30.0),
            1,
            Edges::all(1.0),
            Color::WHITE,
        );
        list.draw_path(
            vec![
                PathOp::MoveTo(Point::new(120.0, 85.0)),
                PathOp::LineTo(Point::new(180.0, 85.0)),
                PathOp::LineTo(Point::new(150.0, 120.0)),
                PathOp::Close,
            ],
            Some(Color::ORANGE),
            Some((2.0, Color::WHITE)),
        );

        let mut renderer = WgpuRenderer::new();
        let pixels = renderer.render_offscreen(
            &list,
            &mut atlas,
            &resources,
            280,
            140,
            Color::from_hex(0x141820),
        );
        assert_eq!(pixels.len(), 280 * 140 * 4);

        // Verify red rounded rect center pixel
        let idx = ((40 * 280 + 60) * 4) as usize;
        assert!(pixels[idx] > 180, "expected red fill inside rounded rect");

        // Verify image region has non-background color
        let img_idx = ((30 * 280 + 230) * 4) as usize;
        assert!(pixels[img_idx] > 30 || pixels[img_idx + 2] > 100);

        let png_bytes = encode_rgba_to_png(&pixels, 280, 140).expect("PNG encode");
        assert_eq!(&png_bytes[0..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn batch_coalescing_keeps_draw_calls_low() {
        let mut list = DisplayList::new(1.0);
        for i in 0..50 {
            list.fill_rounded_rect(
                Rect::new(i as f32 * 4.0, 0.0, 3.0, 10.0),
                CornerRadii::all(1.0),
                Color::BLUE,
            );
        }
        assert_eq!(list.batch_count(), 1);
    }

    #[test]
    fn svg_decoding_via_resvg_and_glyph_atlas_subpixel() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><circle cx="16" cy="16" r="14" fill="rgb(255,0,0)"/></svg>"#;
        let decoded = ResourceLoader::decode_svg(42, svg).expect("decode svg");
        assert_eq!(decoded.width, 32);
        assert_eq!(decoded.height, 32);
        let center_idx = ((16 * 32 + 16) * 4) as usize;
        assert!(decoded.rgba[center_idx] > 200);

        let mut atlas = GlyphAtlas::new(128, 128);
        let key = GlyphKey {
            font_id: 1,
            glyph_id: 65,
            size_q4: 16 * 16,
            subpixel_x: SubpixelBin::Quarter,
        };
        let entry = atlas
            .insert_raw(key, 8, 12, 1, 10, false, &vec![255u8; 8 * 12 * 4])
            .expect("insert glyph");
        assert_eq!(entry.width, 8);
        assert_eq!(atlas.entry_count(), 1);
    }

    #[test]
    fn damage_tracker_tracks_changed_node_regions() {
        let rt = rustui_core::Runtime::new();
        let n1 = rt.alloc_node(rustui_core::NodeKind::Modifier, None);
        rt.with_inner_mut(|inner| {
            inner.nodes.get_mut(n1).unwrap().layout_cache.bounds =
                Rect::new(20.0, 20.0, 60.0, 30.0);
        });
        let mut dmg = DamageTracker::new();
        let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
        let first = dmg.collect(&rt, viewport);
        assert_eq!(first, vec![viewport]);

        // Unchanged frame produces zero damage.
        let second = dmg.collect(&rt, viewport);
        assert!(second.is_empty());

        // Marking node dirty damages only that node's region.
        rt.with_inner_mut(|inner| {
            inner.mark_dirty(n1, rustui_core::DirtyFlags::PAINT);
        });
        let third = dmg.collect(&rt, viewport);
        assert_eq!(third.len(), 1);
        assert!(third[0].width < 100.0);
    }

    #[test]
    fn inspector_server_and_atlas_eviction_and_perfetto_work() {
        let rt = rustui_core::Runtime::new();
        let n = rt.alloc_node(rustui_core::NodeKind::Modifier, None);
        rt.with_inner_mut(|inner| {
            inner.root = Some(n);
            inner.nodes.get_mut(n).unwrap().accessibility.label = Some("RootLabel".into());
        });
        let dmg = DamageTracker::new();
        let insp = InspectorOverlay::new(true);
        let snap = insp.capture_snapshot(&rt, &dmg);
        assert_eq!(snap.node_count, 1);
        assert!(snap.to_json_string().contains("RootLabel"));

        let srv = InspectorServer::bind(0).expect("ephemeral inspector server");
        let addr = srv.local_addr();
        srv.publish_and_poll(&snap);
        let mut client = std::net::TcpStream::connect(addr).expect("connect inspector");
        std::io::Write::write_all(&mut client, b"GET\n").unwrap();
        srv.publish_and_poll(&snap);
        let mut resp = String::new();
        std::io::Read::read_to_string(&mut client, &mut resp).unwrap();
        assert!(resp.contains("RootLabel"));

        let trace = export_perfetto_trace(&snap.metrics);
        assert!(trace.contains("\"traceEvents\""));

        let mut atlas = GlyphAtlas::new(128, 128);
        for gid in 1..=4 {
            let k = GlyphKey {
                font_id: 1,
                glyph_id: gid,
                size_q4: 256,
                subpixel_x: SubpixelBin::Zero,
            };
            atlas.insert_raw(k, 4, 4, 0, 4, false, &[255u8; 4 * 4 * 4]);
        }
        assert_eq!(atlas.evict_if_over_budget(2), 2);
        assert_eq!(atlas.entry_count(), 2);
    }
}
