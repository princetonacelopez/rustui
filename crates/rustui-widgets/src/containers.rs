//! Tier 4 container widgets (`NavigationStack`, `TabView`, `SplitView`, `Sheet`, `Popover`, `Menu`, `Toolbar`).

use std::rc::Rc;

use rustui_core::{
    AccessibilityRole, Alignment, AnyView, Binding, BuildCx, Color, ContainerKind, CornerRadii, Cx,
    DirtyFlags, Edge, Edges, EmptyView, Frame, HorizontalAlignment, If, NodeId, NodeKind, Point,
    Primitive, Transition, VerticalAlignment, View, ViewExt,
};

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
        .background(theme.colors.surface);

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
        let root = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
            spacing: 0.0,
            alignment: HorizontalAlignment::Leading,
        }));

        let tab_buttons: Vec<AnyView> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| {
                let sel = self.selected.clone();
                let is_active = sel.get() == idx;
                let style = if is_active {
                    ButtonStyle::primary(&theme)
                } else {
                    ButtonStyle::plain(&theme)
                };
                AnyView::new(
                    Button::new(tab.title.clone(), move || sel.set(idx))
                        .style(style)
                        .accessibility_role(AccessibilityRole::Tab),
                )
            })
            .collect();

        let tab_bar = HStack::new(6, tab_buttons)
            .padding(Edges::symmetric(8.0, 10.0))
            .background(theme.colors.surface)
            .accessibility_role(AccessibilityRole::TabList);
        cx.build_child(root, tab_bar);
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
        let root = cx.alloc_node(NodeKind::Container(ContainerKind::HStack {
            spacing: 0.0,
            alignment: VerticalAlignment::Top,
        }));
        let left_node = cx.build_child(root, self.left);
        let splitter = cx.alloc_node(NodeKind::Modifier);
        let right_node = cx.build_child(root, self.right);

        let rt = cx.runtime.clone();
        let drag_ratio = self.ratio.clone();
        cx.runtime.with_inner_mut(|inner| {
            if let Some(sp) = inner.nodes.get_mut(splitter) {
                sp.layout_props.frame = Some(Frame::new().width(6.0).max_height(f32::INFINITY));
                sp.visual_props.background = Some(theme.colors.border);
                sp.accessibility.role = AccessibilityRole::Splitter;
                sp.accessibility.label = Some("Split divider".into());
                sp.events.on_drag = Some(Rc::new(move |ev| {
                    let parent_w = rt
                        .with_inner(|i| i.nodes.get(root).map(|n| n.bounds().width))
                        .unwrap_or(800.0)
                        .max(100.0);
                    let r = ((ev.current.x) / parent_w).clamp(0.15, 0.85);
                    drag_ratio.set(r);
                }));
            }
        });

        let r_left = self.ratio;
        let rt_left = cx.runtime.clone();
        cx.runtime.register_node_updater(
            left_node,
            move || {
                let total_w = rt_left
                    .with_inner(|i| i.nodes.get(root).map(|p| p.bounds().width))
                    .unwrap_or(800.0)
                    .max(200.0);
                (r_left.get(), total_w)
            },
            move |n, (ratio, total_w)| {
                n.layout_props.frame =
                    Some(Frame::new().width((total_w - 6.0) * ratio.clamp(0.15, 0.85)));
                DirtyFlags::LAYOUT
            },
        );
        let _ = right_node;

        cx.runtime.attach_child(root, splitter);
        // Ensure splitter sits between left and right in children list.
        cx.runtime.with_inner_mut(|inner| {
            if let Some(p) = inner.nodes.get_mut(root) {
                p.children = vec![left_node, splitter, right_node];
            }
        });
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
                            .corner_radius(12)
                            .shadow(Color::rgba(0.0, 0.0, 0.0, 0.25), 16, Point::new(0.0, 6.0))
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
                VStack::new(4, btns)
                    .alignment(HorizontalAlignment::Leading)
                    .background(theme.colors.surface_elevated)
                    .corner_radius(CornerRadii::all(6.0).top_left)
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
        let bar = cx.alloc_node(NodeKind::Container(ContainerKind::HStack {
            spacing: 10.0,
            alignment: VerticalAlignment::Center,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(bar) {
                n.layout_props.padding = Edges::symmetric(8.0, 12.0);
                n.visual_props.background = Some(theme.colors.surface);
                n.visual_props.border = Some((1.0, theme.colors.border));
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
