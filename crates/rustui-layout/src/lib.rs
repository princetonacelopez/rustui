//! SwiftUI-style proposal/response layout engine and containers for RustUI.

pub mod containers;
pub mod protocol;

pub use containers::{
    GridLayout, HStackLayout, LazyVStackLayout, ScrollViewLayout, SpacerLayout, VStackLayout,
    ZStackLayout,
};
pub use protocol::{
    ChildProxy, Children, CustomLayoutBox, DefaultTextMeasurer, Layout, LayoutEngine, TextMeasurer,
    snap_to_pixel,
};

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use rustui_core::{
        Alignment, ContainerKind, DirtyFlags, Frame, GridTrack, HorizontalAlignment, NodeId,
        NodeKind, Point, Rect, Runtime, Size, TextDirection, TextNodeData, VerticalAlignment,
        VirtualListNodeData,
    };

    use super::*;

    fn make_box(rt: &Runtime, w: f32, h: f32, parent: Option<NodeId>) -> NodeId {
        let id = rt.alloc_node(NodeKind::Modifier, parent);
        rt.with_inner_mut(|inner| {
            let n = inner.nodes.get_mut(id).unwrap();
            n.layout_props.frame = Some(Frame::fixed(w, h));
        });
        id
    }

    #[test]
    fn vstack_places_children_with_spacing_and_alignment() {
        let rt = Runtime::new();
        let stack = rt.alloc_node(
            NodeKind::Container(ContainerKind::VStack {
                spacing: 10.0,
                alignment: HorizontalAlignment::Center,
            }),
            None,
        );
        let a = make_box(&rt, 100.0, 20.0, Some(stack));
        let b = make_box(&rt, 60.0, 30.0, Some(stack));
        rt.attach_child(stack, a);
        rt.attach_child(stack, b);

        let engine = LayoutEngine::new();
        let mut measurer = DefaultTextMeasurer;
        engine.layout(&rt, stack, Size::new(200.0, 200.0), &mut measurer);

        rt.with_inner(|inner| {
            assert_eq!(
                inner.nodes.get(a).unwrap().bounds(),
                Rect::new(0.0, 0.0, 100.0, 20.0)
            );
            assert_eq!(
                inner.nodes.get(b).unwrap().bounds(),
                Rect::new(20.0, 30.0, 60.0, 30.0)
            );
        });
    }

    #[test]
    fn hstack_distributes_spacer_and_respects_layout_priority() {
        let rt = Runtime::new();
        let stack = rt.alloc_node(
            NodeKind::Container(ContainerKind::HStack {
                spacing: 10.0,
                alignment: VerticalAlignment::Center,
            }),
            None,
        );
        let left = make_box(&rt, 40.0, 20.0, Some(stack));
        let spacer = rt.alloc_node(
            NodeKind::Container(ContainerKind::Spacer { min_length: 8.0 }),
            Some(stack),
        );
        let right = make_box(&rt, 50.0, 20.0, Some(stack));
        rt.with_inner_mut(|inner| {
            inner
                .nodes
                .get_mut(right)
                .unwrap()
                .layout_props
                .layout_priority = 1.0;
        });
        rt.attach_child(stack, left);
        rt.attach_child(stack, spacer);
        rt.attach_child(stack, right);

        let engine = LayoutEngine::new();
        let mut measurer = DefaultTextMeasurer;
        engine.layout(&rt, stack, Size::new(210.0, 40.0), &mut measurer);

        rt.with_inner(|inner| {
            let lb = inner.nodes.get(left).unwrap().bounds();
            let sb = inner.nodes.get(spacer).unwrap().bounds();
            let rb = inner.nodes.get(right).unwrap().bounds();
            assert_eq!(lb.x, 0.0);
            assert_eq!(lb.width, 40.0);
            assert_eq!(rb.width, 50.0);
            // Total 210 - 20 spacing - 40 - 50 = 100 leftover for spacer!
            assert!((sb.width - 100.0).abs() < 0.01);
            assert!((rb.x - 160.0).abs() < 0.01);
        });
    }

    #[test]
    fn hstack_rtl_mirrors_leading_and_trailing_automatically() {
        let rt = Runtime::new();
        let stack = rt.alloc_node(
            NodeKind::Container(ContainerKind::HStack {
                spacing: 10.0,
                alignment: VerticalAlignment::Top,
            }),
            None,
        );
        let first = make_box(&rt, 40.0, 20.0, Some(stack));
        let second = make_box(&rt, 60.0, 20.0, Some(stack));
        rt.attach_child(stack, first);
        rt.attach_child(stack, second);

        let engine = LayoutEngine::with_direction(TextDirection::Rtl);
        let mut measurer = DefaultTextMeasurer;
        engine.layout(&rt, stack, Size::new(200.0, 40.0), &mut measurer);

        rt.with_inner(|inner| {
            let b1 = inner.nodes.get(first).unwrap().bounds();
            let b2 = inner.nodes.get(second).unwrap().bounds();
            assert!(
                b1.x > b2.x,
                "first child must be placed to the right of second in RTL: {:?} vs {:?}",
                b1,
                b2
            );
            assert_eq!(b1.max_x(), 110.0);
            assert_eq!(b2.x, 0.0);
        });
    }

    #[test]
    fn zstack_and_grid_layout_snapshot() {
        let rt = Runtime::new();
        let z = rt.alloc_node(
            NodeKind::Container(ContainerKind::ZStack {
                alignment: Alignment::BOTTOM_TRAILING,
            }),
            None,
        );
        let bg = make_box(&rt, 120.0, 80.0, Some(z));
        let badge = make_box(&rt, 20.0, 16.0, Some(z));
        rt.attach_child(z, bg);
        rt.attach_child(z, badge);

        let engine = LayoutEngine::new();
        let mut measurer = DefaultTextMeasurer;
        engine.layout(&rt, z, Size::new(120.0, 80.0), &mut measurer);

        rt.with_inner(|inner| {
            assert_eq!(
                inner.nodes.get(badge).unwrap().bounds(),
                Rect::new(100.0, 64.0, 20.0, 16.0)
            );
        });

        let grid = rt.alloc_node(
            NodeKind::Container(ContainerKind::Grid {
                columns: vec![
                    GridTrack::Fixed(50.0),
                    GridTrack::Flexible {
                        min: 20.0,
                        max: 200.0,
                    },
                ],
                h_spacing: 10.0,
                v_spacing: 8.0,
            }),
            None,
        );
        let c0 = make_box(&rt, 50.0, 20.0, Some(grid));
        let c1 = make_box(&rt, 80.0, 30.0, Some(grid));
        let c2 = make_box(&rt, 50.0, 25.0, Some(grid));
        for c in [c0, c1, c2] {
            rt.attach_child(grid, c);
        }
        engine.layout(&rt, grid, Size::new(200.0, 200.0), &mut measurer);

        rt.with_inner(|inner| {
            assert_eq!(
                inner.nodes.get(c0).unwrap().bounds(),
                Rect::new(0.0, 0.0, 50.0, 20.0)
            );
            assert_eq!(
                inner.nodes.get(c1).unwrap().bounds(),
                Rect::new(60.0, 0.0, 80.0, 30.0)
            );
            assert_eq!(
                inner.nodes.get(c2).unwrap().bounds(),
                Rect::new(0.0, 38.0, 50.0, 25.0)
            );
        });
    }

    #[test]
    fn custom_layout_reads_layout_value_metadata() {
        #[derive(Clone)]
        struct TagOffset(f32);

        struct DiagonalLayout;
        impl Layout for DiagonalLayout {
            fn size(&self, _proposed: Size<Option<f32>>, children: &mut Children<'_>) -> Size<f32> {
                let mut total = 0.0f32;
                for i in 0..children.len() {
                    let s = children.child(i).size(Size::UNSPECIFIED);
                    total += s.width.max(s.height);
                }
                Size::new(total, total)
            }

            fn place(&self, bounds: Rect, children: &mut Children<'_>) {
                let mut offset = 0.0f32;
                for i in 0..children.len() {
                    let extra = children
                        .child(i)
                        .layout_value::<TagOffset>()
                        .map(|t| t.0)
                        .unwrap_or(0.0);
                    let sz = children.child(i).size(Size::UNSPECIFIED);
                    children.child(i).place(
                        Point::new(bounds.x + offset + extra, bounds.y + offset),
                        Size::UNSPECIFIED,
                    );
                    offset += sz.height;
                }
            }
        }

        let rt = Runtime::new();
        let container = rt.alloc_node(NodeKind::Container(ContainerKind::CustomLayout), None);
        rt.with_inner_mut(|inner| {
            inner.nodes.get_mut(container).unwrap().custom_layout =
                Some(Rc::new(CustomLayoutBox(Rc::new(DiagonalLayout))));
        });
        let a = make_box(&rt, 20.0, 20.0, Some(container));
        let b = make_box(&rt, 30.0, 30.0, Some(container));
        rt.with_inner_mut(|inner| {
            inner
                .nodes
                .get_mut(b)
                .unwrap()
                .layout_props
                .layout_values
                .insert(TagOffset(15.0));
        });
        rt.attach_child(container, a);
        rt.attach_child(container, b);

        let engine = LayoutEngine::new();
        let mut measurer = DefaultTextMeasurer;
        engine.layout(&rt, container, Size::new(200.0, 200.0), &mut measurer);

        rt.with_inner(|inner| {
            assert_eq!(
                inner.nodes.get(a).unwrap().bounds(),
                Rect::new(0.0, 0.0, 20.0, 20.0)
            );
            assert_eq!(
                inner.nodes.get(b).unwrap().bounds(),
                Rect::new(35.0, 20.0, 30.0, 30.0)
            );
        });
    }

    #[test]
    fn incremental_layout_cache_skips_clean_nodes() {
        let rt = Runtime::new();
        let root = rt.alloc_node(
            NodeKind::Container(ContainerKind::VStack {
                spacing: 4.0,
                alignment: HorizontalAlignment::Leading,
            }),
            None,
        );
        let fixed_wrap = rt.alloc_node(NodeKind::Modifier, Some(root));
        rt.with_inner_mut(|inner| {
            inner.nodes.get_mut(fixed_wrap).unwrap().layout_props.frame =
                Some(Frame::fixed(150.0, 30.0));
        });
        let text = rt.alloc_node(
            NodeKind::Text(TextNodeData {
                text: "Hello".into(),
                ..Default::default()
            }),
            Some(fixed_wrap),
        );
        let sibling = make_box(&rt, 80.0, 20.0, Some(root));
        rt.attach_child(fixed_wrap, text);
        rt.attach_child(root, fixed_wrap);
        rt.attach_child(root, sibling);

        let engine = LayoutEngine::new();
        let mut measurer = DefaultTextMeasurer;
        let first_pass = engine.layout(&rt, root, Size::new(300.0, 300.0), &mut measurer);
        assert!(first_pass >= 4);

        let second_pass = engine.layout(&rt, root, Size::new(300.0, 300.0), &mut measurer);
        assert!(second_pass <= first_pass);

        // Mutating text inside a fixed-size boundary leaves sibling clean.
        rt.with_inner_mut(|inner| {
            if let NodeKind::Text(t) = &mut inner.nodes.get_mut(text).unwrap().kind {
                t.text = "Hello World!".into();
            }
            inner.mark_dirty(text, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
            assert!(
                !inner
                    .nodes
                    .get(sibling)
                    .unwrap()
                    .dirty
                    .intersects(DirtyFlags::LAYOUT)
            );
        });
    }

    #[test]
    fn lazy_vstack_computes_1m_rows_in_constant_time_and_snaps_pixels() {
        let vl = VirtualListNodeData {
            item_count: 1_000_000,
            estimated_row_height: 28.0,
            scroll_offset: 500_000.0 * 30.0,
            overscan: 2,
            visible_start: 0,
            visible_end: 0,
            spacing: 2.0,
        };
        let layout = LazyVStackLayout::from_data(&vl);
        let (start, end) = layout.visible_range(600.0);
        assert_eq!(start, 499_998);
        assert_eq!(end, 500_022);
        assert!((layout.total_content_height() - 29_999_998.0).abs() < 1.0);

        let snapped = snap_to_pixel(Rect::new(10.3, 20.7, 50.25, 19.8), 2.0);
        assert_eq!(snapped, Rect::new(10.5, 20.5, 50.0, 20.0));
    }
}
