//! Tier 3 collection widgets (`ScrollView`, `LazyVStack`, `List`, `Table`, `Tree`).

use std::fmt::Write as _;
use std::rc::Rc;

use rustui_core::{
    AccessibilityRole, AnyView, Binding, BuildCx, ContainerKind, CornerRadii, Cx, DirtyFlags,
    Edges, EmptyView, Frame, GridTrack, HorizontalAlignment, IntoPx, NodeId, NodeKind, Primitive,
    ScrollAxes, ScrollNodeData, View, ViewExt, VirtualListNodeData,
};

use crate::core_widgets::{Button, Divider, HStack, Text};
use crate::theme::{ButtonStyle, Font, ListStyle, Theme};

fn resolve_theme(cx: &BuildCx) -> Theme {
    cx.env::<Theme>().unwrap_or_default()
}

/// Scrollable viewport container (`ScrollView`).
#[derive(Debug)]
pub struct ScrollView<V: View> {
    axes: ScrollAxes,
    content: V,
}

impl<V: View> ScrollView<V> {
    pub fn vertical(content: V) -> Self {
        Self {
            axes: ScrollAxes::Vertical,
            content,
        }
    }

    pub fn horizontal(content: V) -> Self {
        Self {
            axes: ScrollAxes::Horizontal,
            content,
        }
    }

    pub fn both(content: V) -> Self {
        Self {
            axes: ScrollAxes::Both,
            content,
        }
    }
}

impl<V: View> Primitive for ScrollView<V> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let node = cx.alloc_node(NodeKind::Scroll(ScrollNodeData {
            axes: self.axes,
            ..Default::default()
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(node) {
                n.visual_props.clip_to_bounds = true;
                n.accessibility.role = AccessibilityRole::ScrollView;
            }
        });
        cx.build_child(node, self.content);
        node
    }
}

impl<V: View> View for ScrollView<V> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

type PoolUpdaterFn = Rc<dyn Fn(usize, &mut rustui_core::Node)>;
type ListRowBuilderFn<Item> = Rc<dyn Fn(usize, &Item, bool) -> AnyView>;

/// Virtualized vertical stack (`LazyVStack`) capable of 1,000,000+ rows at 60 fps.
///
/// Only a small visible pool of row nodes is constructed and recycled in-place during scrolling.
pub struct LazyVStack {
    item_count: usize,
    row_height: f32,
    spacing: f32,
    overscan: usize,
    pool_size: usize,
    row_builder: Rc<dyn Fn(usize) -> AnyView>,
    pool_updater: Option<PoolUpdaterFn>,
}

impl std::fmt::Debug for LazyVStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyVStack")
            .field("item_count", &self.item_count)
            .field("row_height", &self.row_height)
            .field("pool_size", &self.pool_size)
            .finish()
    }
}

impl LazyVStack {
    pub fn new<V: View>(
        item_count: usize,
        row_height: impl IntoPx,
        row_builder: impl Fn(usize) -> V + 'static,
    ) -> Self {
        Self {
            item_count,
            row_height: row_height.into_px().max(12.0),
            spacing: 2.0,
            overscan: 2,
            pool_size: 40,
            row_builder: Rc::new(move |idx| AnyView::new(row_builder(idx))),
            pool_updater: None,
        }
    }

    pub fn spacing(mut self, spacing: impl IntoPx) -> Self {
        self.spacing = spacing.into_px().max(0.0);
        self
    }

    /// Optional zero-allocation in-place updater invoked on pooled row nodes when scrolling.
    pub fn with_pool_updater(
        mut self,
        updater: impl Fn(usize, &mut rustui_core::Node) + 'static,
    ) -> Self {
        self.pool_updater = Some(Rc::new(updater));
        self
    }
}

impl Primitive for LazyVStack {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let initial_rows = self.item_count.min(self.pool_size);
        let node = cx.alloc_node(NodeKind::VirtualList(VirtualListNodeData {
            item_count: self.item_count,
            estimated_row_height: self.row_height,
            scroll_offset: 0.0,
            overscan: self.overscan,
            visible_start: 0,
            visible_end: initial_rows,
            spacing: self.spacing,
        }));

        for idx in 0..initial_rows {
            let row_view = (self.row_builder)(idx);
            let child = cx.build_child(node, row_view);
            cx.runtime.with_inner_mut(|inner| {
                if let Some(c) = inner.nodes.get_mut(child) {
                    c.accessibility.role = AccessibilityRole::ListItem;
                }
            });
        }

        let rt = cx.runtime.clone();
        let item_count = self.item_count;
        let row_h = self.row_height;
        let spacing = self.spacing;
        let overscan = self.overscan;
        let pool_updater = self.pool_updater;

        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(node) {
                n.visual_props.clip_to_bounds = true;
                n.accessibility.role = AccessibilityRole::List;
                n.events.on_scroll = Some(Rc::new(move |ev| {
                    let mut children_to_update = Vec::new();
                    let mut start_idx = 0usize;
                    rt.with_inner_mut(|rt_inner| {
                        let Some(vn) = rt_inner.nodes.get_mut(node) else {
                            return;
                        };
                        let bounds = vn.bounds();
                        let pitch = (row_h + spacing).max(1.0);
                        let total_h = item_count as f32 * pitch;
                        let max_y = (total_h - bounds.height).max(0.0);
                        if let NodeKind::VirtualList(vl) = &mut vn.kind {
                            vl.scroll_offset = (vl.scroll_offset - ev.delta_y).clamp(0.0, max_y);
                            let first = (vl.scroll_offset / pitch).floor().max(0.0) as usize;
                            start_idx = first.saturating_sub(overscan);
                            let last = (((vl.scroll_offset + bounds.height.max(400.0)) / pitch)
                                .ceil() as usize)
                                .min(item_count);
                            let end = (last + overscan).min(item_count);
                            vl.visible_start = start_idx;
                            vl.visible_end = end;
                            children_to_update = vn.children.clone();
                        }
                        rt_inner.mark_dirty(node, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
                    });

                    if let Some(upd) = &pool_updater {
                        rt.with_inner_mut(|rt_inner| {
                            for (slot, cid) in children_to_update.into_iter().enumerate() {
                                if let Some(cn) = rt_inner.nodes.get_mut(cid) {
                                    upd(start_idx + slot, cn);
                                }
                            }
                        });
                    }
                    true
                }));
            }
        });

        node
    }
}

impl View for LazyVStack {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Zero-allocation helper updating a simple text row inside a pooled `LazyVStack`.
pub fn format_virtual_row_text(node: &mut rustui_core::Node, prefix: &str, row_idx: usize) {
    if let NodeKind::Text(t) = &mut node.kind {
        t.text.clear();
        let _ = write!(&mut t.text, "{prefix}{row_idx}");
        node.dirty.insert(DirtyFlags::PAINT);
    }
}

/// Virtualized selectable list widget (`List`) styled via [`ListStyle`].
pub struct List<Item: Clone + 'static> {
    items: Vec<Item>,
    selected: Binding<Option<usize>>,
    row_builder: ListRowBuilderFn<Item>,
}

impl<Item: Clone + 'static> std::fmt::Debug for List<Item> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("List")
            .field("item_count", &self.items.len())
            .finish()
    }
}

impl<Item: Clone + 'static> List<Item> {
    pub fn new<V: View>(
        items: Vec<Item>,
        selected: Binding<Option<usize>>,
        row_builder: impl Fn(usize, &Item, bool) -> V + 'static,
    ) -> Self {
        Self {
            items,
            selected,
            row_builder: Rc::new(move |idx, item, is_sel| {
                AnyView::new(row_builder(idx, item, is_sel))
            }),
        }
    }
}

impl<Item: Clone + 'static> Primitive for List<Item> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let style = cx
            .env::<ListStyle>()
            .unwrap_or_else(|| ListStyle::from_theme(&theme));
        let row_h = style.row_height;
        let items = Rc::new(self.items);
        let sel_b = self.selected;
        let row_builder = self.row_builder;

        let count = items.len();
        LazyVStack::new(count, row_h, move |idx| {
            let is_sel = sel_b.get() == Some(idx);
            let bg = if is_sel {
                style.selected_background
            } else if idx % 2 == 1 {
                style
                    .alternate_row_background
                    .unwrap_or(style.row_background)
            } else {
                style.row_background
            };
            let item = items[idx].clone();
            let sel_click = sel_b.clone();
            let content = (row_builder)(idx, &item, is_sel);

            let mut row = rustui_core::ViewExt::padding(content, Edges::symmetric(6.0, 10.0))
                .background(bg)
                .corner_radius(style.corner_radii.top_left)
                .on_tap(move || sel_click.set(Some(idx)))
                .focusable()
                .accessibility_role(AccessibilityRole::ListItem);
            let _ = &mut row;
            row
        })
        .build_node(cx)
    }
}

impl<Item: Clone + 'static> View for List<Item> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Column definition for [`Table`].
#[derive(Debug, Clone)]
pub struct TableColumn {
    pub title: String,
    pub track: GridTrack,
}

impl TableColumn {
    pub fn fixed(title: impl Into<String>, width: f32) -> Self {
        Self {
            title: title.into(),
            track: GridTrack::Fixed(width),
        }
    }

    pub fn flexible(title: impl Into<String>, min: f32, max: f32) -> Self {
        Self {
            title: title.into(),
            track: GridTrack::Flexible { min, max },
        }
    }
}

/// Multi-column virtualized data table (`Table`).
pub struct Table {
    columns: Vec<TableColumn>,
    row_count: usize,
    row_height: f32,
    selected: Binding<Option<usize>>,
    cell_text: Rc<dyn Fn(usize, usize) -> String>,
}

impl std::fmt::Debug for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Table")
            .field("columns", &self.columns.len())
            .field("row_count", &self.row_count)
            .finish()
    }
}

impl Table {
    pub fn new(
        columns: Vec<TableColumn>,
        row_count: usize,
        selected: Binding<Option<usize>>,
        cell_text: impl Fn(usize, usize) -> String + 'static,
    ) -> Self {
        Self {
            columns,
            row_count,
            row_height: 30.0,
            selected,
            cell_text: Rc::new(cell_text),
        }
    }
}

impl Primitive for Table {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let root = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
            spacing: 0.0,
            alignment: HorizontalAlignment::Leading,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(root) {
                n.visual_props.border = Some((1.0, theme.colors.border));
                n.visual_props.corner_radii = CornerRadii::all(theme.radius_md);
                n.visual_props.clip_to_bounds = true;
                n.accessibility.role = AccessibilityRole::Table;
                n.accessibility.label = Some("Table".into());
            }
        });

        let header_cells: Vec<AnyView> = self
            .columns
            .iter()
            .map(|c| {
                let w = match c.track {
                    GridTrack::Fixed(w) => w,
                    GridTrack::Flexible { min, .. } => min.max(100.0),
                };
                AnyView::new(rustui_core::ViewExt::accessibility_role(
                    rustui_core::ViewExt::frame(
                        rustui_core::ViewExt::padding(
                            Text::new(c.title.clone()).font(Font::headline()),
                            Edges::symmetric(6.0, 10.0),
                        ),
                        Frame::new().width(w),
                    ),
                    AccessibilityRole::TableColumnHeader,
                ))
            })
            .collect();
        let header =
            rustui_core::ViewExt::background(HStack::new(0, header_cells), theme.colors.surface);
        cx.build_child(root, header);
        cx.build_child(root, Divider::new());

        let cols = Rc::new(self.columns);
        let cell_fn = self.cell_text;
        let sel_b = self.selected;
        let surface_elevated = theme.colors.surface_elevated;
        let accent_sel = theme.colors.accent.with_alpha(0.15);

        let body = LazyVStack::new(self.row_count, self.row_height, move |row_idx| {
            let cells: Vec<AnyView> = (0..cols.len())
                .map(|col_idx| {
                    let w = match cols[col_idx].track {
                        GridTrack::Fixed(w) => w,
                        GridTrack::Flexible { min, .. } => min.max(100.0),
                    };
                    let txt = (cell_fn)(row_idx, col_idx);
                    AnyView::new(rustui_core::ViewExt::accessibility_role(
                        rustui_core::ViewExt::frame(
                            rustui_core::ViewExt::padding(
                                Text::new(txt),
                                Edges::symmetric(4.0, 10.0),
                            ),
                            Frame::new().width(w),
                        ),
                        AccessibilityRole::TableCell,
                    ))
                })
                .collect();
            let sel_click = sel_b.clone();
            let bg = if sel_b.get() == Some(row_idx) {
                accent_sel
            } else {
                surface_elevated
            };
            rustui_core::ViewExt::accessibility_role(
                rustui_core::ViewExt::on_tap(
                    rustui_core::ViewExt::background(HStack::new(0, cells), bg),
                    move || sel_click.set(Some(row_idx)),
                ),
                AccessibilityRole::TableRow,
            )
        });
        cx.build_child(root, body);
        root
    }
}

impl View for Table {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Expandable hierarchical item for [`Tree`].
#[derive(Debug, Clone)]
pub struct TreeItemData {
    pub id: String,
    pub label: String,
    pub expanded: bool,
    pub children: Vec<TreeItemData>,
}

/// Hierarchical expandable tree view (`Tree`).
pub struct Tree {
    roots: Vec<TreeItemData>,
    on_toggle: Rc<dyn Fn(&str)>,
}

impl std::fmt::Debug for Tree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tree")
            .field("roots", &self.roots.len())
            .finish()
    }
}

impl Tree {
    pub fn new(roots: Vec<TreeItemData>, on_toggle: impl Fn(&str) + 'static) -> Self {
        Self {
            roots,
            on_toggle: Rc::new(on_toggle),
        }
    }
}

impl Primitive for Tree {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let container = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
            spacing: 4.0,
            alignment: HorizontalAlignment::Leading,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(container) {
                n.accessibility.role = AccessibilityRole::Tree;
                n.accessibility.label = Some("Tree".into());
            }
        });

        let mut rows = Vec::new();
        flatten_tree_items(&self.roots, 0, &mut rows);

        for (depth, item) in rows {
            let indent = (depth as f32) * 16.0;
            let glyph = if item.children.is_empty() {
                "•"
            } else if item.expanded {
                "▾"
            } else {
                "▸"
            };
            let id_c = item.id.clone();
            let toggle = self.on_toggle.clone();
            let row = rustui_core::ViewExt::accessibility_role(
                rustui_core::ViewExt::padding(
                    HStack::new(
                        6,
                        (
                            Button::new(glyph, move || toggle(&id_c))
                                .style(ButtonStyle::plain(&theme)),
                            Text::new(item.label),
                        ),
                    ),
                    Edges::new(2.0, 6.0, 2.0, indent),
                ),
                AccessibilityRole::TreeItem,
            );
            cx.build_child(container, row);
        }

        container
    }
}

impl View for Tree {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

fn flatten_tree_items(items: &[TreeItemData], depth: usize, out: &mut Vec<(usize, TreeItemData)>) {
    for item in items {
        out.push((depth, item.clone()));
        if item.expanded {
            flatten_tree_items(&item.children, depth + 1, out);
        }
    }
}
