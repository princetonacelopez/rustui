//! Tier 4 container widgets (`NavigationStack`, `TabView`, `SplitView`, `Sheet`, `Popover`, `Menu`, `Toolbar`).

use std::cell::Cell;
use std::rc::Rc;

use rustui_core::{
    AccessibilityRole, Alignment, AnyView, Binding, BuildCx, Color, ContainerKind, Cx, DirtyFlags,
    Edge, Edges, EmptyView, Frame, HorizontalAlignment, If, NodeId, NodeKind, Point, Primitive,
    Rect, Size, Transition, VerticalAlignment, View, ViewExt,
};
use rustui_layout::{Children, CustomLayoutBox, Layout};

use crate::core_widgets::{Button, Divider, HStack, Spacer, Text, VStack, ZStack};
use crate::theme::{ButtonStyle, Font, Theme};

fn resolve_theme(cx: &BuildCx) -> Theme {
    cx.env::<Theme>().unwrap_or_default()
}

/// Page entry inside a [`NavigationStack`].
pub struct NavigationPage {
    pub title: String,
    pub builder: Rc<dyn Fn() -> AnyView>,
}

impl std::fmt::Debug for NavigationPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationPage")
            .field("title", &self.title)
            .finish()
    }
}

impl NavigationPage {
    pub fn new<V: View>(title: impl Into<String>, builder: impl Fn() -> V + 'static) -> Self {
        Self {
            title: title.into(),
            builder: Rc::new(move || AnyView::new(builder())),
        }
    }
}

/// Push/pop stack navigation container (`NavigationStack`).
pub struct NavigationStack {
    depth: Binding<usize>,
    pages: Vec<NavigationPage>,
}

impl std::fmt::Debug for NavigationStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationStack")
            .field("pages", &self.pages.len())
            .finish()
    }
}

impl NavigationStack {
    pub fn new(depth: Binding<usize>, pages: Vec<NavigationPage>) -> Self {
        Self { depth, pages }
    }
}

impl Primitive for NavigationStack {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let root = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
            spacing: 0.0,
            alignment: HorizontalAlignment::Leading,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(root) {
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .max_height(f32::INFINITY)
                        .alignment(Alignment::TOP_LEADING),
                );
                n.accessibility.role = AccessibilityRole::Navigation;
                n.accessibility.label = Some("NavigationStack".into());
            }
        });

        let pages = Rc::new(self.pages);
        let depth_b = self.depth.clone();
        let depth_for_title = self.depth.clone();
        let depth_for_back = self.depth.clone();
        let pages_for_title = pages.clone();
        let theme_btn = theme.clone();
        let header_bg = if theme.uses_default_palette() {
            Color::surface()
        } else {
            theme.colors.surface
        };

        let header = HStack::new(
            12,
            (
                If::new(move || depth_for_back.get() > 0, {
                    let d = depth_b.clone();
                    let t = theme_btn.clone();
                    move || {
                        Button::new("‹ Back", {
                            let d = d.clone();
                            move || d.update(|v| *v = v.saturating_sub(1))
                        })
                        .style(ButtonStyle::plain(&t))
                    }
                }),
                Text::new(move || {
                    let idx = depth_for_title
                        .get()
                        .min(pages_for_title.len().saturating_sub(1));
                    pages_for_title
                        .get(idx)
                        .map(|p| p.title.clone())
                        .unwrap_or_default()
                })
                .font(Font::headline()),
                Spacer::new(),
            ),
        )
        .padding(Edges::symmetric(10.0, 14.0))
        .background(header_bg);

        cx.build_child(root, header);
        cx.build_child(root, Divider::new());

        let depth_for_switch = self.depth;
        let pages_len = pages.len().max(1);
        let mut sw = rustui_core::Switch::new(move || depth_for_switch.get().min(pages_len - 1));
        for (i, page) in pages.iter().enumerate() {
            let b = page.builder.clone();
            sw = sw.case(i, move || b().transition(Transition::Move(Edge::Trailing)));
        }
        cx.build_child(root, sw);
        root
    }
}

impl View for NavigationStack {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Single tab definition inside [`TabView`].
pub struct TabItem {
    pub title: String,
    pub content: Rc<dyn Fn() -> AnyView>,
}

impl std::fmt::Debug for TabItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TabItem")
            .field("title", &self.title)
            .finish()
    }
}

impl TabItem {
    pub fn new<V: View>(title: impl Into<String>, content: impl Fn() -> V + 'static) -> Self {
        Self {
            title: title.into(),
            content: Rc::new(move || AnyView::new(content())),
        }
    }
}

/// Tabbed container with accessible tab bar (`TabView`).
#[derive(Debug)]
pub struct TabView {
    selected: Binding<usize>,
    tabs: Vec<TabItem>,
}

impl TabView {
    pub fn new(selected: Binding<usize>, tabs: Vec<TabItem>) -> Self {
        Self { selected, tabs }
    }
}

impl Primitive for TabView {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let standard = theme.uses_default_palette();
        let root = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
            spacing: 0.0,
            alignment: HorizontalAlignment::Leading,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(root) {
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .max_height(f32::INFINITY)
                        .alignment(Alignment::TOP_LEADING),
                );
            }
        });

        let bar_bg = if standard {
            Color::surface()
        } else {
            theme.colors.surface
        };
        let active_bg = if standard {
            Color::accent()
        } else {
            theme.colors.accent
        };
        let active_fg = theme.colors.on_accent;
        let inactive_fg = if standard {
            Color::primary()
        } else {
            theme.colors.primary
        };

        let tab_bar = cx.alloc_node(NodeKind::Container(ContainerKind::HStack {
            spacing: 6.0,
            alignment: VerticalAlignment::Center,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(tab_bar) {
                n.layout_props.padding = Edges::symmetric(8.0, 10.0);
                n.visual_props.background = Some(bar_bg);
                n.accessibility.role = AccessibilityRole::TabList;
            }
        });
        cx.runtime.attach_child(root, tab_bar);

        let mut tab_bcx = BuildCx::new(cx.runtime.clone(), Some(tab_bar));
        for (idx, tab) in self.tabs.iter().enumerate() {
            let sel = self.selected.clone();
            let btn_node =
                Button::new(tab.title.clone(), move || sel.set(idx)).build_node(&mut tab_bcx);
            cx.runtime.with_inner_mut(|inner| {
                if let Some(n) = inner.nodes.get_mut(btn_node) {
                    n.accessibility.role = AccessibilityRole::Tab;
                }
            });
            cx.runtime.attach_child(tab_bar, btn_node);

            let text_child = cx.runtime.with_inner(|inner| {
                inner
                    .nodes
                    .get(btn_node)
                    .and_then(|n| n.children.first().copied())
            });
            let sel_bg = self.selected.clone();
            cx.runtime.register_node_updater(
                btn_node,
                move || sel_bg.get() == idx,
                move |n, active| {
                    n.visual_props.background = Some(if active {
                        active_bg
                    } else {
                        Color::TRANSPARENT
                    });
                    DirtyFlags::PAINT
                },
            );
            if let Some(tid) = text_child {
                let sel_fg = self.selected.clone();
                cx.runtime.register_node_updater(
                    tid,
                    move || sel_fg.get() == idx,
                    move |n, active| {
                        if let NodeKind::Text(t) = &mut n.kind {
                            t.color = Some(if active { active_fg } else { inactive_fg });
                        }
                        DirtyFlags::PAINT
                    },
                );
            }
        }
        let bar_spacer = Spacer::new().build_node(&mut tab_bcx);
        cx.runtime.attach_child(tab_bar, bar_spacer);
        cx.build_child(root, Divider::new());

        let count = self.tabs.len().max(1);
        let sel_sw = self.selected;
        let mut sw = rustui_core::Switch::new(move || sel_sw.get().min(count - 1));
        for (idx, tab) in self.tabs.into_iter().enumerate() {
            let content = tab.content;
            sw = sw.case(idx, move || content());
        }
        cx.build_child(root, sw);
        root
    }
}

impl View for TabView {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

struct SplitViewLayout {
    ratio: Rc<Cell<f32>>,
}

impl Layout for SplitViewLayout {
    fn size(&self, proposed: Size<Option<f32>>, children: &mut Children<'_>) -> Size<f32> {
        let w = match proposed.width {
            Some(w) if w.is_finite() => w,
            Some(_) => 10_000.0,
            None => {
                let left = if !children.is_empty() {
                    children.child(0).size(Size::UNSPECIFIED).width
                } else {
                    200.0
                };
                let right = if children.len() > 2 {
                    children.child(2).size(Size::UNSPECIFIED).width
                } else {
                    400.0
                };
                left + 6.0 + right
            }
        };
        let h = match proposed.height {
            Some(h) if h.is_finite() => h,
            Some(_) => 10_000.0,
            None => {
                let mut max_h = 200.0f32;
                for i in 0..children.len() {
                    max_h = max_h.max(children.child(i).size(Size::UNSPECIFIED).height);
                }
                max_h
            }
        };
        Size::new(w, h)
    }

    fn place(&self, bounds: Rect, children: &mut Children<'_>) {
        if children.len() < 3 {
            return;
        }
        let splitter_w = 6.0f32.min(bounds.width);
        let avail_w = (bounds.width - splitter_w).max(0.0);
        let r = self.ratio.get().clamp(0.15, 0.85);
        let left_w = (avail_w * r).round();
        let right_w = (avail_w - left_w).max(0.0);

        children.child(0).place(
            Point::new(bounds.x, bounds.y),
            Size::new(Some(left_w), Some(bounds.height)),
        );
        children.child(1).place(
            Point::new(bounds.x + left_w, bounds.y),
            Size::new(Some(splitter_w), Some(bounds.height)),
        );
        children.child(2).place(
            Point::new(bounds.x + left_w + splitter_w, bounds.y),
            Size::new(Some(right_w), Some(bounds.height)),
        );
    }
}

/// Resizable horizontal split view with draggable splitter (`SplitView`).
#[derive(Debug)]
pub struct SplitView<L: View, R: View> {
    ratio: Binding<f32>,
    left: L,
    right: R,
}

impl<L: View, R: View> SplitView<L, R> {
    pub fn horizontal(ratio: Binding<f32>, left: L, right: R) -> Self {
        Self { ratio, left, right }
    }
}

impl<L: View, R: View> Primitive for SplitView<L, R> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let border_col = if theme.uses_default_palette() {
            Color::border()
        } else {
            theme.colors.border
        };
        let initial_ratio = cx.runtime.enter(|| self.ratio.get()).clamp(0.15, 0.85);
        let cached_ratio = Rc::new(Cell::new(initial_ratio));
        let root = cx.alloc_node(NodeKind::Container(ContainerKind::CustomLayout));
        let left_node = cx.build_child(root, self.left);
        let splitter = cx.alloc_node(NodeKind::Modifier);
        let right_node = cx.build_child(root, self.right);

        let rt = cx.runtime.clone();
        let drag_ratio = self.ratio.clone();
        cx.runtime.with_inner_mut(|inner| {
            if let Some(p) = inner.nodes.get_mut(root) {
                p.custom_layout = Some(Rc::new(CustomLayoutBox(Rc::new(SplitViewLayout {
                    ratio: cached_ratio.clone(),
                }))));
                p.children = vec![left_node, splitter, right_node];
            }
            if let Some(sp) = inner.nodes.get_mut(splitter) {
                sp.parent = Some(root);
                sp.layout_props.frame = Some(Frame::new().width(6.0).max_height(f32::INFINITY));
                sp.visual_props.background = Some(border_col);
                sp.accessibility.role = AccessibilityRole::Splitter;
                sp.accessibility.label = Some("Split divider".into());
                sp.events.on_drag = Some(Rc::new(move |ev| {
                    let root_b = rt
                        .with_inner(|i| i.nodes.get(root).map(|n| n.bounds()))
                        .unwrap_or(Rect::new(0.0, 0.0, 800.0, 500.0));
                    let r = ((ev.current.x - root_b.x) / root_b.width.max(100.0)).clamp(0.15, 0.85);
                    drag_ratio.set(r);
                }));
            }
        });

        let r_watch = self.ratio;
        let ratio_cell = cached_ratio;
        cx.runtime.register_node_updater(
            root,
            move || r_watch.get(),
            move |_n, val| {
                ratio_cell.set(val.clamp(0.15, 0.85));
                DirtyFlags::LAYOUT | DirtyFlags::PAINT
            },
        );
        root
    }
}

impl<L: View, R: View> View for SplitView<L, R> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Modal sheet overlay (`Sheet`).
pub struct Sheet<C: View, S: View> {
    is_presented: Binding<bool>,
    content: C,
    sheet_builder: Rc<dyn Fn() -> S>,
}

impl<C: View, S: View> std::fmt::Debug for Sheet<C, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sheet").finish_non_exhaustive()
    }
}

impl<C: View, S: View> Sheet<C, S> {
    pub fn new(is_presented: Binding<bool>, content: C, sheet: impl Fn() -> S + 'static) -> Self {
        Self {
            is_presented,
            content,
            sheet_builder: Rc::new(sheet),
        }
    }
}

impl<C: View, S: View> View for Sheet<C, S> {
    fn body(self, _cx: &mut Cx) -> impl View {
        let presented = self.is_presented.clone();
        let dismiss = self.is_presented;
        let builder = self.sheet_builder;
        ZStack::new((
            self.content,
            If::new(
                move || presented.get(),
                move || {
                    let dismiss_bg = dismiss.clone();
                    ZStack::new((
                        EmptyView
                            .frame(
                                Frame::new()
                                    .max_width(f32::INFINITY)
                                    .max_height(f32::INFINITY),
                            )
                            .background(Color::rgba(0.0, 0.0, 0.0, 0.4))
                            .on_tap(move || dismiss_bg.set(false)),
                        builder()
                            .padding(20)
                            .background(Color::surface_elevated())
                            .border(1, Color::border())
                            .corner_radius(12)
                            .shadow(Color::rgba(0.0, 0.0, 0.0, 0.25), 16, Point::new(0.0, 6.0))
                            .on_tap(|| {})
                            .transition(Transition::Scale),
                    ))
                },
            ),
        ))
    }
}

/// Anchored popover overlay (`Popover`).
pub struct Popover<A: View, P: View> {
    is_presented: Binding<bool>,
    anchor: A,
    popover_builder: Rc<dyn Fn() -> P>,
}

impl<A: View, P: View> std::fmt::Debug for Popover<A, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Popover").finish_non_exhaustive()
    }
}

impl<A: View, P: View> Popover<A, P> {
    pub fn new(is_presented: Binding<bool>, anchor: A, popover: impl Fn() -> P + 'static) -> Self {
        Self {
            is_presented,
            anchor,
            popover_builder: Rc::new(popover),
        }
    }
}

impl<A: View, P: View> View for Popover<A, P> {
    fn body(self, _cx: &mut Cx) -> impl View {
        let presented = self.is_presented;
        let builder = self.popover_builder;
        ZStack::new((
            self.anchor,
            If::new(
                move || presented.get(),
                move || {
                    builder()
                        .padding(12)
                        .background(Color::surface_elevated())
                        .border(1, Color::border())
                        .corner_radius(8)
                        .shadow(Color::rgba(0.0, 0.0, 0.0, 0.2), 10, Point::new(0.0, 4.0))
                        .offset(0, 38)
                        .accessibility_role(AccessibilityRole::Popover)
                },
            ),
        ))
        .alignment(Alignment::TOP_LEADING)
    }
}

/// Menu action item inside [`Menu`].
pub struct MenuItem {
    pub label: String,
    pub action: Rc<dyn Fn()>,
}

impl std::fmt::Debug for MenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuItem")
            .field("label", &self.label)
            .finish()
    }
}

impl MenuItem {
    pub fn new(label: impl Into<String>, action: impl Fn() + 'static) -> Self {
        Self {
            label: label.into(),
            action: Rc::new(action),
        }
    }
}

/// Expandable dropdown menu widget (`Menu`).
#[derive(Debug)]
pub struct Menu {
    label: String,
    items: Vec<MenuItem>,
}

impl Menu {
    pub fn new(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self {
            label: label.into(),
            items,
        }
    }
}

impl View for Menu {
    fn body(self, cx: &mut Cx) -> impl View {
        let open = cx.state(false);
        let theme = cx.env::<Theme>().unwrap_or_default();
        let items = Rc::new(self.items);

        Popover::new(
            open.binding(),
            Button::new(format!("{} ▾", self.label), move || {
                open.update(|o| *o = !*o)
            })
            .style(ButtonStyle::secondary(&theme))
            .accessibility_role(AccessibilityRole::Menu),
            move || {
                let btns: Vec<AnyView> = items
                    .iter()
                    .map(|item| {
                        let act = item.action.clone();
                        let close = open;
                        AnyView::new(
                            Button::new(item.label.clone(), move || {
                                act();
                                close.set(false);
                            })
                            .style(ButtonStyle::plain(&theme))
                            .accessibility_role(AccessibilityRole::MenuItem),
                        )
                    })
                    .collect();
                VStack::new(4, btns).alignment(HorizontalAlignment::Leading)
            },
        )
    }
}

/// Window toolbar container (`Toolbar`).
#[derive(Debug)]
pub struct Toolbar<C: rustui_core::ViewSequence> {
    children: C,
}

impl<C: rustui_core::ViewSequence> Toolbar<C> {
    pub fn new(children: C) -> Self {
        Self { children }
    }
}

impl<C: rustui_core::ViewSequence> Primitive for Toolbar<C> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let standard = theme.uses_default_palette();
        let bg = if standard {
            Color::surface()
        } else {
            theme.colors.surface
        };
        let border = if standard {
            Color::border()
        } else {
            theme.colors.border
        };
        let bar = cx.alloc_node(NodeKind::Container(ContainerKind::HStack {
            spacing: 10.0,
            alignment: VerticalAlignment::Center,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(bar) {
                n.layout_props.padding = Edges::symmetric(8.0, 12.0);
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .min_height(48.0)
                        .alignment(Alignment::LEADING),
                );
                n.visual_props.background = Some(bg);
                n.visual_props.border = Some((1.0, border));
                n.accessibility.role = AccessibilityRole::Toolbar;
                n.accessibility.label = Some("Toolbar".into());
            }
        });
        self.children.build_all(cx, bar);
        bar
    }
}

impl<C: rustui_core::ViewSequence> View for Toolbar<C> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}
