//! Built-in layout containers: `VStack`, `HStack`, `ZStack`, `Spacer`, `Grid`, `ScrollView`, `LazyVStack`.

use rustui_core::{
    Alignment, GridTrack, HorizontalAlignment, Point, Rect, ScrollAxes, Size, TextDirection,
    VerticalAlignment, VirtualListNodeData,
};

use crate::protocol::{Children, Layout};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Vertical,
    Horizontal,
}

impl Axis {
    fn main(self, s: Size<f32>) -> f32 {
        match self {
            Self::Vertical => s.height,
            Self::Horizontal => s.width,
        }
    }

    fn cross(self, s: Size<f32>) -> f32 {
        match self {
            Self::Vertical => s.width,
            Self::Horizontal => s.height,
        }
    }

    fn main_opt(self, s: Size<Option<f32>>) -> Option<f32> {
        match self {
            Self::Vertical => s.height,
            Self::Horizontal => s.width,
        }
    }

    fn cross_opt(self, s: Size<Option<f32>>) -> Option<f32> {
        match self {
            Self::Vertical => s.width,
            Self::Horizontal => s.height,
        }
    }

    fn make_size<T>(self, main: T, cross: T) -> Size<T> {
        match self {
            Self::Vertical => Size::new(cross, main),
            Self::Horizontal => Size::new(main, cross),
        }
    }
}

fn solve_stack(
    axis: Axis,
    spacing: f32,
    proposed: Size<Option<f32>>,
    children: &mut Children<'_>,
) -> (Vec<f32>, Size<f32>) {
    let n = children.len();
    if n == 0 {
        return (Vec::new(), Size::ZERO);
    }

    let total_spacing = spacing * (n.saturating_sub(1) as f32);
    let main_prop = axis.main_opt(proposed);
    let cross_prop = axis.cross_opt(proposed);

    let mut allocated_main = vec![0.0f32; n];
    let mut max_cross = 0.0f32;

    match main_prop {
        Some(total_main) if total_main.is_finite() => {
            let mut entries: Vec<(usize, f64, f32, f32)> = Vec::with_capacity(n);
            for i in 0..n {
                let mut child = children.child(i);
                let min_s = axis.main(child.size(axis.make_size(Some(0.0), cross_prop)));
                let max_s =
                    axis.main(child.size(axis.make_size(Some(f32::INFINITY), cross_prop)));
                let flex = (max_s - min_s).max(0.0);
                let prio = child.priority();
                entries.push((i, prio, flex, min_s));
            }

            // Highest priority first; within same priority, least flexible children get sized first.
            entries.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
            });

            let mut remaining = (total_main - total_spacing).max(0.0);
            let mut min_sum_remaining: f32 = entries.iter().map(|e| e.3).sum();

            for (pos, &(idx, _, _, min_s)) in entries.iter().enumerate() {
                min_sum_remaining = (min_sum_remaining - min_s).max(0.0);
                let rem_count = (n - pos) as f32;
                let max_for_this = (remaining - min_sum_remaining).max(min_s);
                let fair_share = (remaining / rem_count).max(min_s).min(max_for_this);
                let measured = children
                    .child(idx)
                    .size(axis.make_size(Some(fair_share), cross_prop));
                let m = axis.main(measured);
                allocated_main[idx] = m;
                max_cross = max_cross.max(axis.cross(measured));
                remaining = (remaining - m).max(0.0);
            }
        }
        _ => {
            for (i, slot) in allocated_main.iter_mut().enumerate() {
                let measured = children
                    .child(i)
                    .size(axis.make_size(main_prop, cross_prop));
                *slot = axis.main(measured);
                max_cross = max_cross.max(axis.cross(measured));
            }
        }
    }

    let sum_main: f32 = allocated_main.iter().sum::<f32>() + total_spacing;
    let total_size = axis.make_size(sum_main, max_cross);
    (allocated_main, total_size)
}

fn place_stack(
    axis: Axis,
    spacing: f32,
    cross_align: impl Fn(TextDirection, Option<f32>, Option<f32>) -> f32,
    bounds: Rect,
    children: &mut Children<'_>,
) {
    let n = children.len();
    if n == 0 {
        return;
    }
    let dir = children.direction();
    let proposed = Size::new(Some(bounds.width), Some(bounds.height));
    let (main_sizes, _) = solve_stack(axis, spacing, proposed, children);

    let rtl_horizontal = axis == Axis::Horizontal && dir == TextDirection::Rtl;
    let mut cursor = if rtl_horizontal {
        bounds.max_x()
    } else {
        match axis {
            Axis::Vertical => bounds.y,
            Axis::Horizontal => bounds.x,
        }
    };

    let cross_bounds_len = match axis {
        Axis::Vertical => bounds.width,
        Axis::Horizontal => bounds.height,
    };

    for (i, &main_sz) in main_sizes.iter().enumerate() {
        let child_prop = axis.make_size(Some(main_sz), Some(cross_bounds_len));
        let (guides_x, guides_y, measured) = {
            let mut child = children.child(i);
            let m = child.size(child_prop);
            let (gx, gy) = child.alignment_guides();
            (gx, gy, m)
        };
        let child_cross = axis.cross(measured);
        let align_factor = cross_align(dir, guides_x, guides_y);
        let cross_pos = match axis {
            Axis::Vertical => bounds.x + (bounds.width - child_cross) * align_factor,
            Axis::Horizontal => bounds.y + (bounds.height - child_cross) * align_factor,
        };

        let origin = if rtl_horizontal {
            cursor -= main_sz;
            let p = Point::new(cursor, cross_pos);
            cursor -= spacing;
            p
        } else {
            let p = match axis {
                Axis::Vertical => Point::new(cross_pos, cursor),
                Axis::Horizontal => Point::new(cursor, cross_pos),
            };
            cursor += main_sz + spacing;
            p
        };

        children.child(i).place(origin, child_prop);
    }
}

/// Vertical stack layout distributing children along the Y axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VStackLayout {
    pub spacing: f32,
    pub alignment: HorizontalAlignment,
}

impl Layout for VStackLayout {
    fn size(&self, proposed: Size<Option<f32>>, children: &mut Children<'_>) -> Size<f32> {
        solve_stack(Axis::Vertical, self.spacing, proposed, children).1
    }

    fn place(&self, bounds: Rect, children: &mut Children<'_>) {
        place_stack(
            Axis::Vertical,
            self.spacing,
            |dir, gx, _| gx.unwrap_or_else(|| self.alignment.factor(dir)),
            bounds,
            children,
        );
    }
}

/// Horizontal stack layout distributing children along the X axis (mirrors automatically in RTL).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HStackLayout {
    pub spacing: f32,
    pub alignment: VerticalAlignment,
}

impl Layout for HStackLayout {
    fn size(&self, proposed: Size<Option<f32>>, children: &mut Children<'_>) -> Size<f32> {
        solve_stack(Axis::Horizontal, self.spacing, proposed, children).1
    }

    fn place(&self, bounds: Rect, children: &mut Children<'_>) {
        place_stack(
            Axis::Horizontal,
            self.spacing,
            |_, _, gy| gy.unwrap_or_else(|| self.alignment.factor()),
            bounds,
            children,
        );
    }
}

/// Overlay stack layout placing children on top of one another using 2D [`Alignment`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZStackLayout {
    pub alignment: Alignment,
}

impl Layout for ZStackLayout {
    fn size(&self, proposed: Size<Option<f32>>, children: &mut Children<'_>) -> Size<f32> {
        let mut max_w = 0.0f32;
        let mut max_h = 0.0f32;
        for i in 0..children.len() {
            let sz = children.child(i).size(proposed);
            max_w = max_w.max(sz.width);
            max_h = max_h.max(sz.height);
        }
        Size::new(max_w, max_h)
    }

    fn place(&self, bounds: Rect, children: &mut Children<'_>) {
        let dir = children.direction();
        let child_prop = Size::new(Some(bounds.width), Some(bounds.height));
        for i in 0..children.len() {
            let (sz, gx, gy) = {
                let mut c = children.child(i);
                let sz = c.size(child_prop);
                let (gx, gy) = c.alignment_guides();
                (sz, gx, gy)
            };
            let ax = gx.unwrap_or_else(|| self.alignment.horizontal.factor(dir));
            let ay = gy.unwrap_or_else(|| self.alignment.vertical.factor());
            let x = bounds.x + (bounds.width - sz.width) * ax;
            let y = bounds.y + (bounds.height - sz.height) * ay;
            children.child(i).place(Point::new(x, y), child_prop);
        }
    }
}

/// Flexible empty spacer layout expanding to fill available stack space above `min_length`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpacerLayout {
    pub min_length: f32,
}

impl SpacerLayout {
    pub fn measure(&self, proposed: Size<Option<f32>>) -> Size<f32> {
        let resolve = |opt: Option<f32>| match opt {
            None => self.min_length,
            Some(v) if v.is_infinite() => 10_000.0_f32.max(self.min_length),
            Some(v) => v.max(self.min_length),
        };
        Size::new(resolve(proposed.width), resolve(proposed.height))
    }
}

impl Layout for SpacerLayout {
    fn size(&self, proposed: Size<Option<f32>>, _children: &mut Children<'_>) -> Size<f32> {
        self.measure(proposed)
    }

    fn place(&self, _bounds: Rect, _children: &mut Children<'_>) {}
}

/// Row/column grid layout with configurable [`GridTrack`] column sizing rules.
#[derive(Debug, Clone, PartialEq)]
pub struct GridLayout {
    pub columns: Vec<GridTrack>,
    pub h_spacing: f32,
    pub v_spacing: f32,
}

impl GridLayout {
    fn compute_col_widths(&self, proposed_width: Option<f32>) -> Vec<f32> {
        if self.columns.is_empty() {
            return vec![proposed_width.filter(|w| w.is_finite()).unwrap_or(100.0)];
        }
        let col_count = self.columns.len();
        let total_h_spacing = self.h_spacing * (col_count.saturating_sub(1) as f32);
        let mut widths = vec![0.0f32; col_count];
        let mut fixed_sum = 0.0f32;
        let mut flex_indices = Vec::new();

        for (i, track) in self.columns.iter().enumerate() {
            match *track {
                GridTrack::Fixed(w) => {
                    widths[i] = w;
                    fixed_sum += w;
                }
                GridTrack::Flexible { min, .. } => {
                    widths[i] = min;
                    flex_indices.push(i);
                }
            }
        }

        if let Some(avail) = proposed_width.filter(|w| w.is_finite())
            && !flex_indices.is_empty()
        {
            let rem = (avail - total_h_spacing - fixed_sum).max(0.0);
            let each = rem / flex_indices.len() as f32;
            for idx in flex_indices {
                if let GridTrack::Flexible { min, max } = self.columns[idx] {
                    widths[idx] = each.clamp(min, max);
                }
            }
        }
        widths
    }
}

impl Layout for GridLayout {
    fn size(&self, proposed: Size<Option<f32>>, children: &mut Children<'_>) -> Size<f32> {
        let n = children.len();
        if n == 0 {
            return Size::ZERO;
        }
        let col_widths = self.compute_col_widths(proposed.width);
        let cols = col_widths.len();
        let mut total_h = 0.0f32;
        let mut row_idx = 0usize;

        while row_idx * cols < n {
            let mut row_h = 0.0f32;
            for (c, &cw) in col_widths.iter().enumerate() {
                let idx = row_idx * cols + c;
                if idx >= n {
                    break;
                }
                let sz = children.child(idx).size(Size::new(Some(cw), None));
                row_h = row_h.max(sz.height);
            }
            if row_idx > 0 {
                total_h += self.v_spacing;
            }
            total_h += row_h;
            row_idx += 1;
        }

        let total_w =
            col_widths.iter().sum::<f32>() + self.h_spacing * (cols.saturating_sub(1) as f32);
        Size::new(total_w, total_h)
    }

    fn place(&self, bounds: Rect, children: &mut Children<'_>) {
        let n = children.len();
        if n == 0 {
            return;
        }
        let dir = children.direction();
        let col_widths = self.compute_col_widths(Some(bounds.width));
        let cols = col_widths.len();
        let mut y = bounds.y;
        let mut row_idx = 0usize;

        while row_idx * cols < n {
            let mut row_h = 0.0f32;
            for (c, &cw) in col_widths.iter().enumerate() {
                let idx = row_idx * cols + c;
                if idx >= n {
                    break;
                }
                let sz = children.child(idx).size(Size::new(Some(cw), None));
                row_h = row_h.max(sz.height);
            }

            let mut x = if dir == TextDirection::Rtl {
                bounds.max_x()
            } else {
                bounds.x
            };
            for (c, &cw) in col_widths.iter().enumerate() {
                let idx = row_idx * cols + c;
                if idx >= n {
                    break;
                }
                let pos_x = if dir == TextDirection::Rtl {
                    x -= cw;
                    let px = x;
                    x -= self.h_spacing;
                    px
                } else {
                    let px = x;
                    x += cw + self.h_spacing;
                    px
                };
                children
                    .child(idx)
                    .place(Point::new(pos_x, y), Size::new(Some(cw), Some(row_h)));
            }
            y += row_h + self.v_spacing;
            row_idx += 1;
        }
    }
}

/// Scroll container layout proposing unbounded size on its scroll axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollViewLayout {
    pub axes: ScrollAxes,
}

impl Layout for ScrollViewLayout {
    fn size(&self, proposed: Size<Option<f32>>, children: &mut Children<'_>) -> Size<f32> {
        let child_prop = Size::new(
            match self.axes {
                ScrollAxes::Horizontal | ScrollAxes::Both => None,
                ScrollAxes::Vertical => proposed.width,
            },
            match self.axes {
                ScrollAxes::Vertical | ScrollAxes::Both => None,
                ScrollAxes::Horizontal => proposed.height,
            },
        );
        let content = if !children.is_empty() {
            children.child(0).size(child_prop)
        } else {
            Size::ZERO
        };
        Size::new(
            proposed
                .width
                .filter(|w| w.is_finite())
                .unwrap_or(content.width),
            proposed
                .height
                .filter(|h| h.is_finite())
                .unwrap_or(content.height),
        )
    }

    fn place(&self, bounds: Rect, children: &mut Children<'_>) {
        if !children.is_empty() {
            let prop = Size::new(Some(bounds.width), Some(bounds.height));
            children.child(0).place(bounds.origin(), prop);
        }
    }
}

/// Virtualized vertical stack layout (`LazyVStack`) measuring and placing only visible rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LazyVStackLayout {
    pub item_count: usize,
    pub estimated_row_height: f32,
    pub scroll_offset: f32,
    pub overscan: usize,
    pub spacing: f32,
}

impl LazyVStackLayout {
    pub fn from_data(data: &VirtualListNodeData) -> Self {
        Self {
            item_count: data.item_count,
            estimated_row_height: data.estimated_row_height.max(1.0),
            scroll_offset: data.scroll_offset.max(0.0),
            overscan: data.overscan,
            spacing: data.spacing,
        }
    }

    /// Computes `[start_index, end_index)` visible row slice for `viewport_height` in O(1) time.
    pub fn visible_range(&self, viewport_height: f32) -> (usize, usize) {
        if self.item_count == 0 {
            return (0, 0);
        }
        let pitch = (self.estimated_row_height + self.spacing).max(1.0);
        let first_visible = (self.scroll_offset / pitch).floor().max(0.0) as usize;
        let last_visible =
            (((self.scroll_offset + viewport_height.max(1.0)) / pitch).ceil().max(0.0) as usize)
                .min(self.item_count);
        let start = first_visible.saturating_sub(self.overscan);
        let end = (last_visible + self.overscan).min(self.item_count);
        (start, end)
    }

    pub fn total_content_height(&self) -> f32 {
        if self.item_count == 0 {
            0.0
        } else {
            self.item_count as f32 * self.estimated_row_height
                + self.item_count.saturating_sub(1) as f32 * self.spacing
        }
    }
}

impl Layout for LazyVStackLayout {
    fn size(&self, proposed: Size<Option<f32>>, children: &mut Children<'_>) -> Size<f32> {
        let viewport_h = proposed
            .height
            .filter(|h| h.is_finite())
            .unwrap_or_else(|| self.total_content_height().min(600.0));
        let (start, end) = self.visible_range(viewport_h);
        let visible_count = end.saturating_sub(start).min(children.len());
        let width_prop = proposed.width.filter(|w| w.is_finite());
        let mut max_w = width_prop.unwrap_or(0.0);

        for i in 0..visible_count {
            let sz = children
                .child(i)
                .size(Size::new(width_prop, Some(self.estimated_row_height)));
            max_w = max_w.max(sz.width);
        }

        let height = proposed
            .height
            .filter(|h| h.is_finite())
            .unwrap_or_else(|| self.total_content_height());
        Size::new(max_w, height)
    }

    fn place(&self, bounds: Rect, children: &mut Children<'_>) {
        let (start, end) = self.visible_range(bounds.height);
        let visible_count = end.saturating_sub(start).min(children.len());
        let pitch = self.estimated_row_height + self.spacing;
        let row_prop = Size::new(Some(bounds.width), Some(self.estimated_row_height));

        for i in 0..visible_count {
            let row_idx = start + i;
            let y = bounds.y + row_idx as f32 * pitch - self.scroll_offset;
            children.child(i).place(Point::new(bounds.x, y), row_prop);
        }
    }
}
