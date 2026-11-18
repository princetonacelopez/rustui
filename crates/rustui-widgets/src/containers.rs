//! Tier 4 container widgets (`NavigationStack`, `TabView`, `SplitView`, `Sheet`, `Popover`, `Menu`, `Toolbar`).

use std::cell::Cell;
use std::rc::Rc;

use rustui_core::{
    AccessibilityRole, Alignment, AnyView, Binding, BuildCx, Color, ContainerKind, Cx, DirtyFlags,
    Edges, EmptyView, Frame, HorizontalAlignment, If, NodeId, NodeKind, Point, Primitive, Rect,
    Size, Transition, VerticalAlignment, View, ViewExt, ViewSequence,
};
use rustui_layout::{Children, CustomLayoutBox, Layout};
use rustui_platform::Platform as _;

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
                n.visual_props.clip_to_bounds = true;
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
            sw = sw.case(i, move || {
                if i == 0 {
                    b()
                } else {
                    b().transition(Transition::Opacity).into_any()
                }
            });
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
    vertical: bool,
    collapsed: Rc<Cell<bool>>,
}

impl Layout for SplitViewLayout {
    fn size(&self, proposed: Size<Option<f32>>, children: &mut Children<'_>) -> Size<f32> {
        let w = match proposed.width {
            Some(w) if w.is_finite() => w,
            Some(_) => 10_000.0,
            None => {
                if self.vertical {
                    let top = if !children.is_empty() {
                        children.child(0).size(Size::UNSPECIFIED).width
                    } else {
                        200.0
                    };
                    let bot = if children.len() > 2 {
                        children.child(2).size(Size::UNSPECIFIED).width
                    } else {
                        200.0
                    };
                    top.max(bot)
                } else {
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
            }
        };
        let h = match proposed.height {
            Some(h) if h.is_finite() => h,
            Some(_) => 10_000.0,
            None => {
                if self.vertical {
                    let top = if !children.is_empty() {
                        children.child(0).size(Size::UNSPECIFIED).height
                    } else {
                        160.0
                    };
                    let bot = if children.len() > 2 {
                        children.child(2).size(Size::UNSPECIFIED).height
                    } else {
                        160.0
                    };
                    top + 6.0 + bot
                } else {
                    let mut max_h = 200.0f32;
                    for i in 0..children.len() {
                        max_h = max_h.max(children.child(i).size(Size::UNSPECIFIED).height);
                    }
                    max_h
                }
            }
        };
        Size::new(w, h)
    }

    fn place(&self, bounds: Rect, children: &mut Children<'_>) {
        if children.len() < 3 {
            return;
        }
        let is_collapsed = self.collapsed.get();
        if self.vertical {
            let splitter_h = 6.0f32.min(bounds.height);
            let avail_h = (bounds.height - splitter_h).max(0.0);
            let r = if is_collapsed {
                0.0
            } else {
                self.ratio.get().clamp(0.15, 0.85)
            };
            let top_h = (avail_h * r).round();
            let bot_h = (avail_h - top_h).max(0.0);

            children.child(0).place(
                Point::new(bounds.x, bounds.y),
                Size::new(Some(bounds.width), Some(top_h)),
            );
            children.child(1).place(
                Point::new(bounds.x, bounds.y + top_h),
                Size::new(Some(bounds.width), Some(splitter_h)),
            );
            children.child(2).place(
                Point::new(bounds.x, bounds.y + top_h + splitter_h),
                Size::new(Some(bounds.width), Some(bot_h)),
            );
        } else {
            let splitter_w = 6.0f32.min(bounds.width);
            let avail_w = (bounds.width - splitter_w).max(0.0);
            let r = if is_collapsed {
                0.0
            } else {
                self.ratio.get().clamp(0.15, 0.85)
            };
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
}

/// Resizable and optionally collapsible horizontal/vertical split view (`SplitView`).
#[derive(Debug)]
pub struct SplitView<L: View, R: View> {
    ratio: Binding<f32>,
    left: L,
    right: R,
    vertical: bool,
    collapsible: bool,
    collapsed: Option<Binding<bool>>,
}

impl<L: View, R: View> SplitView<L, R> {
    pub fn horizontal(ratio: Binding<f32>, left: L, right: R) -> Self {
        Self {
            ratio,
            left,
            right,
            vertical: false,
            collapsible: false,
            collapsed: None,
        }
    }

    pub fn vertical(ratio: Binding<f32>, top: L, bottom: R) -> Self {
        Self {
            ratio,
            left: top,
            right: bottom,
            vertical: true,
            collapsible: false,
            collapsed: None,
        }
    }

    pub fn collapsible(mut self, enabled: bool) -> Self {
        self.collapsible = enabled;
        self
    }

    pub fn collapsed(mut self, binding: Binding<bool>) -> Self {
        self.collapsible = true;
        self.collapsed = Some(binding);
        self
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
        let initial_collapsed = self
            .collapsed
            .as_ref()
            .is_some_and(|b| cx.runtime.enter(|| b.get()));
        let cached_collapsed = Rc::new(Cell::new(initial_collapsed));
        let root = cx.alloc_node(NodeKind::Container(ContainerKind::CustomLayout));
        let left_node = cx.build_child(root, self.left);
        let splitter = cx.alloc_node(NodeKind::Modifier);
        let right_node = cx.build_child(root, self.right);

        let rt = cx.runtime.clone();
        let drag_ratio = self.ratio.clone();
        let key_ratio = self.ratio.clone();
        let is_vert = self.vertical;
        let collapsible = self.collapsible;
        let col_cell_for_key = cached_collapsed.clone();
        let col_binding_for_key = self.collapsed.clone();
        cx.runtime.with_inner_mut(|inner| {
            if let Some(p) = inner.nodes.get_mut(root) {
                p.custom_layout = Some(Rc::new(CustomLayoutBox(Rc::new(SplitViewLayout {
                    ratio: cached_ratio.clone(),
                    vertical: is_vert,
                    collapsed: cached_collapsed.clone(),
                }))));
                p.children = vec![left_node, splitter, right_node];
            }
            if let Some(sp) = inner.nodes.get_mut(splitter) {
                sp.parent = Some(root);
                sp.layout_props.frame = Some(if is_vert {
                    Frame::new().height(6.0).max_width(f32::INFINITY)
                } else {
                    Frame::new().width(6.0).max_height(f32::INFINITY)
                });
                sp.visual_props.background = Some(border_col);
                sp.focus.focusable = true;
                sp.accessibility.role = AccessibilityRole::Splitter;
                sp.accessibility.label = Some("Split divider".into());
                sp.accessibility.supports_focus = true;
                sp.events.on_drag = Some(Rc::new(move |ev| {
                    let root_b = rt
                        .with_inner(|i| i.nodes.get(root).map(|n| n.bounds()))
                        .unwrap_or(Rect::new(0.0, 0.0, 800.0, 500.0));
                    let r = if is_vert {
                        ((ev.current.y - root_b.y) / root_b.height.max(100.0)).clamp(0.15, 0.85)
                    } else {
                        ((ev.current.x - root_b.x) / root_b.width.max(100.0)).clamp(0.15, 0.85)
                    };
                    drag_ratio.set(r);
                }));
                sp.events.on_key = Some(Rc::new(move |ev| {
                    if !ev.is_down {
                        return false;
                    }
                    let delta = if is_vert {
                        match ev.key {
                            rustui_core::KeyCode::ArrowUp => Some(-0.05f32),
                            rustui_core::KeyCode::ArrowDown => Some(0.05f32),
                            _ => None,
                        }
                    } else {
                        match ev.key {
                            rustui_core::KeyCode::ArrowLeft => Some(-0.05f32),
                            rustui_core::KeyCode::ArrowRight => Some(0.05f32),
                            _ => None,
                        }
                    };
                    if let Some(d) = delta {
                        let next = (key_ratio.get() + d).clamp(0.15, 0.85);
                        key_ratio.set(next);
                        return true;
                    }
                    if collapsible
                        && matches!(
                            ev.key,
                            rustui_core::KeyCode::Enter | rustui_core::KeyCode::Space
                        )
                    {
                        let next = !col_cell_for_key.get();
                        col_cell_for_key.set(next);
                        if let Some(cb) = &col_binding_for_key {
                            cb.set(next);
                        }
                        return true;
                    }
                    false
                }));
            }
        });

        let r_watch = self.ratio;
        let ratio_cell = cached_ratio;
        let col_watch = self.collapsed;
        let col_watch_getter = col_watch.clone();
        let col_cell_upd = cached_collapsed;
        cx.runtime.register_node_updater(
            root,
            move || {
                let r = r_watch.get();
                let c = col_watch_getter.as_ref().is_some_and(|b| b.get());
                (r, c)
            },
            move |_n, (val, c)| {
                ratio_cell.set(val.clamp(0.15, 0.85));
                if col_watch.is_some() {
                    col_cell_upd.set(c);
                }
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

/// Tab descriptor inside [`TabBar`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabBarItem {
    pub id: String,
    pub title: String,
    pub pinned: bool,
    pub dirty: bool,
}

impl TabBarItem {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            pinned: false,
            dirty: false,
        }
    }

    pub fn pinned(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
    }

    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }
}

/// Closable, reorderable, pinned-aware horizontal tab strip (`TabBar`).
pub struct TabBar {
    tabs: Binding<Vec<TabBarItem>>,
    active: Binding<usize>,
    on_close: Option<Rc<dyn Fn(usize)>>,
    on_reorder: Option<Rc<dyn Fn(usize, usize)>>,
}

impl std::fmt::Debug for TabBar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TabBar").finish_non_exhaustive()
    }
}

impl TabBar {
    pub fn new(tabs: Binding<Vec<TabBarItem>>, active: Binding<usize>) -> Self {
        Self {
            tabs,
            active,
            on_close: None,
            on_reorder: None,
        }
    }

    pub fn on_close(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }

    pub fn on_reorder(mut self, f: impl Fn(usize, usize) + 'static) -> Self {
        self.on_reorder = Some(Rc::new(f));
        self
    }
}

impl View for TabBar {
    fn body(self, cx: &mut Cx) -> impl View {
        let theme = cx.env::<Theme>().unwrap_or_default();
        let tabs_b = self.tabs.clone();
        let active_b = self.active.clone();
        let on_close = self.on_close;
        let on_reorder = self.on_reorder;

        HStack::new(
            4,
            (
                crate::collections::ScrollView::horizontal(HStack::new(
                    4,
                    rustui_core::ForEach::new(
                        move || {
                            tabs_b
                                .get()
                                .into_iter()
                                .enumerate()
                                .collect::<Vec<(usize, TabBarItem)>>()
                        },
                        |(_, item)| item.id.clone(),
                        move |(idx, item)| {
                            let active_click = active_b.clone();
                            let active_style = active_b.clone();
                            let tabs_close = self.tabs.clone();
                            let active_close = active_b.clone();
                            let close_cb = on_close.clone();
                            let reorder_cb = on_reorder.clone();
                            let tabs_move = self.tabs.clone();
                            let active_move = active_b.clone();

                            let title = if item.dirty {
                                format!("{}{} •", if item.pinned { "📌 " } else { "" }, item.title)
                            } else if item.pinned {
                                format!("📌 {}", item.title)
                            } else {
                                item.title.clone()
                            };

                            let is_active = active_style.get() == idx;
                            HStack::new(
                                4,
                                (
                                    Button::new(title, move || active_click.set(idx))
                                        .style(if is_active {
                                            ButtonStyle::primary(&theme)
                                        } else {
                                            ButtonStyle::plain(&theme)
                                        })
                                        .accessibility_role(AccessibilityRole::Tab),
                                    If::new(move || !item.pinned, {
                                        let theme_c = theme.clone();
                                        move || {
                                            let tabs_c = tabs_close.clone();
                                            let act_c = active_close.clone();
                                            let c_cb = close_cb.clone();
                                            Button::new("×", move || {
                                                if let Some(f) = &c_cb {
                                                    f(idx);
                                                } else {
                                                    let cur_act = act_c.get();
                                                    tabs_c.update(|v| {
                                                        if idx < v.len() {
                                                            v.remove(idx);
                                                        }
                                                    });
                                                    if cur_act >= idx && cur_act > 0 {
                                                        act_c.set(cur_act - 1);
                                                    }
                                                }
                                            })
                                            .style(ButtonStyle::plain(&theme_c))
                                            .accessibility_label(format!("Close tab {}", idx + 1))
                                        }
                                    }),
                                    If::new(move || idx > 0 && !item.pinned, {
                                        let theme_m = theme.clone();
                                        move || {
                                            let r_cb = reorder_cb.clone();
                                            let t_m = tabs_move.clone();
                                            let a_m = active_move.clone();
                                            Button::new("‹", move || {
                                                if let Some(f) = &r_cb {
                                                    f(idx, idx - 1);
                                                } else {
                                                    t_m.update(|v| {
                                                        if idx > 0 && idx < v.len() {
                                                            v.swap(idx, idx - 1);
                                                        }
                                                    });
                                                    a_m.set(idx - 1);
                                                }
                                            })
                                            .style(ButtonStyle::plain(&theme_m))
                                            .accessibility_label("Move tab left")
                                        }
                                    }),
                                ),
                            )
                            .padding(Edges::symmetric(2.0, 6.0))
                            .background(if is_active {
                                Color::surface_elevated()
                            } else {
                                Color::surface()
                            })
                            .border(1, Color::border())
                            .corner_radius(6)
                        },
                    ),
                )),
                Spacer::new(),
            ),
        )
        .padding(Edges::symmetric(4.0, 8.0))
        .background(Color::surface())
        .border(1, Color::border())
        .accessibility_role(AccessibilityRole::TabList)
    }
}

/// Window bottom status bar (`StatusBar`).
#[derive(Debug)]
pub struct StatusBar<C: rustui_core::ViewSequence> {
    children: C,
}

impl<C: rustui_core::ViewSequence> StatusBar<C> {
    pub fn new(children: C) -> Self {
        Self { children }
    }
}

impl<C: rustui_core::ViewSequence> Primitive for StatusBar<C> {
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
            spacing: 12.0,
            alignment: VerticalAlignment::Center,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(bar) {
                n.layout_props.padding = Edges::symmetric(5.0, 12.0);
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .min_height(28.0)
                        .alignment(Alignment::LEADING),
                );
                n.visual_props.background = Some(bg);
                n.visual_props.border = Some((1.0, border));
                n.accessibility.role = AccessibilityRole::Status;
                n.accessibility.label = Some("Status Bar".into());
            }
        });
        self.children.build_all(cx, bar);
        bar
    }
}

impl<C: rustui_core::ViewSequence> View for StatusBar<C> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }

    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Named action registered in [`CommandPalette`].
#[derive(Clone)]
pub struct CommandItem {
    pub id: String,
    pub title: String,
    pub shortcut: String,
    pub action: Rc<dyn Fn()>,
}

impl std::fmt::Debug for CommandItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandItem")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("shortcut", &self.shortcut)
            .finish()
    }
}

impl CommandItem {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        shortcut: impl Into<String>,
        action: impl Fn() + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            shortcut: shortcut.into(),
            action: Rc::new(action),
        }
    }
}

fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    let q = needle.trim().to_ascii_lowercase();
    if q.is_empty() {
        return true;
    }
    let h = haystack.to_ascii_lowercase();
    let mut chars = q.chars();
    let mut target = match chars.next() {
        Some(c) => c,
        None => return true,
    };
    for ch in h.chars() {
        if ch == target {
            match chars.next() {
                Some(next) => target = next,
                None => return true,
            }
        }
    }
    false
}

/// Modal fuzzy-searching command palette overlay (`CommandPalette`).
pub struct CommandPalette<C: View> {
    is_open: Binding<bool>,
    commands: Rc<Vec<CommandItem>>,
    content: C,
}

impl<C: View> std::fmt::Debug for CommandPalette<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandPalette")
            .field("commands", &self.commands.len())
            .finish()
    }
}

impl<C: View> CommandPalette<C> {
    pub fn new(is_open: Binding<bool>, commands: Vec<CommandItem>, content: C) -> Self {
        Self {
            is_open,
            commands: Rc::new(commands),
            content,
        }
    }
}

impl<C: View> View for CommandPalette<C> {
    fn body(self, cx: &mut Cx) -> impl View {
        let query = cx.state(String::new());
        let theme = cx.env::<Theme>().unwrap_or_default();
        let open_watch = self.is_open.clone();
        let open_dismiss = self.is_open;
        let commands = self.commands;

        ZStack::new((
            self.content,
            If::new(
                move || open_watch.get(),
                move || {
                    let dismiss_bg = open_dismiss.clone();
                    let dismiss_close = open_dismiss.clone();
                    let dismiss_submit = open_dismiss.clone();
                    let cmds_list = commands.clone();
                    let cmds_submit = commands.clone();
                    let q_bind = query.binding();
                    let q_filter = query;
                    let q_submit = query;
                    let q_reset = query;
                    let theme_list = theme.clone();
                    let theme_close = theme.clone();

                    ZStack::new((
                        EmptyView
                            .frame(
                                Frame::new()
                                    .max_width(f32::INFINITY)
                                    .max_height(f32::INFINITY),
                            )
                            .background(Color::rgba(0.0, 0.0, 0.0, 0.45))
                            .on_tap(move || {
                                q_reset.set(String::new());
                                dismiss_bg.set(false);
                            }),
                        VStack::new(
                            8,
                            (
                                HStack::new(
                                    8,
                                    (
                                        Text::new("Command Palette").font(Font::headline()),
                                        Spacer::new(),
                                        Button::new("Close", move || {
                                            dismiss_close.set(false);
                                        })
                                        .style(ButtonStyle::plain(&theme_close)),
                                    ),
                                ),
                                crate::input_widgets::TextField::new("Type a command...", q_bind)
                                    .on_submit(move |txt| {
                                        let q_now = if txt.is_empty() {
                                            q_submit.get_untracked()
                                        } else {
                                            txt
                                        };
                                        if let Some(first) = cmds_submit
                                            .iter()
                                            .find(|c| fuzzy_match(&c.title, &q_now))
                                        {
                                            (first.action)();
                                            q_submit.set(String::new());
                                            dismiss_submit.set(false);
                                        }
                                    }),
                                crate::collections::ScrollView::vertical(VStack::new(
                                    4,
                                    rustui_core::ForEach::new(
                                        move || {
                                            let q = q_filter.get();
                                            cmds_list
                                                .iter()
                                                .filter(|c| fuzzy_match(&c.title, &q))
                                                .cloned()
                                                .collect::<Vec<_>>()
                                        },
                                        |c| c.id.clone(),
                                        {
                                            let open_row = open_dismiss.clone();
                                            move |item: CommandItem| {
                                                let act = item.action.clone();
                                                let open_c = open_row.clone();
                                                let q_clear = query;
                                                let label = if item.shortcut.is_empty() {
                                                    item.title.clone()
                                                } else {
                                                    format!("{}   [{}]", item.title, item.shortcut)
                                                };
                                                Button::new(label, move || {
                                                    act();
                                                    q_clear.set(String::new());
                                                    open_c.set(false);
                                                })
                                                .style(ButtonStyle::plain(&theme_list))
                                            }
                                        },
                                    ),
                                ))
                                .frame(Frame::new().height(220.0)),
                            ),
                        )
                        .padding(14)
                        .frame(Frame::new().width(480.0))
                        .background(Color::surface_elevated())
                        .border(1, Color::border())
                        .corner_radius(10)
                        .shadow(Color::rgba(0.0, 0.0, 0.0, 0.35), 18, Point::new(0.0, 8.0))
                        .on_tap(|| {})
                        .accessibility_role(AccessibilityRole::AlertDialog),
                    ))
                },
            ),
        ))
    }
}

/// Action button inside [`Alert`] or [`ConfirmationDialog`].
#[derive(Clone)]
pub struct AlertButton {
    pub label: String,
    pub destructive: bool,
    pub action: Rc<dyn Fn()>,
}

impl std::fmt::Debug for AlertButton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlertButton")
            .field("label", &self.label)
            .field("destructive", &self.destructive)
            .finish()
    }
}

impl AlertButton {
    pub fn new(label: impl Into<String>, action: impl Fn() + 'static) -> Self {
        Self {
            label: label.into(),
            destructive: false,
            action: Rc::new(action),
        }
    }

    pub fn destructive(label: impl Into<String>, action: impl Fn() + 'static) -> Self {
        Self {
            label: label.into(),
            destructive: true,
            action: Rc::new(action),
        }
    }
}

/// Modal dialog prompt (`Alert`) anchored over background content.
pub struct Alert<C: View> {
    is_presented: Binding<bool>,
    title: String,
    message: String,
    buttons: Rc<Vec<AlertButton>>,
    content: C,
}

impl<C: View> std::fmt::Debug for Alert<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Alert").field("title", &self.title).finish()
    }
}

impl<C: View> Alert<C> {
    pub fn new(
        is_presented: Binding<bool>,
        title: impl Into<String>,
        message: impl Into<String>,
        buttons: Vec<AlertButton>,
        content: C,
    ) -> Self {
        Self {
            is_presented,
            title: title.into(),
            message: message.into(),
            buttons: Rc::new(buttons),
            content,
        }
    }
}

impl<C: View> View for Alert<C> {
    fn body(self, cx: &mut Cx) -> impl View {
        let theme = cx.env::<Theme>().unwrap_or_default();
        let open = self.is_presented.clone();
        let dismiss = self.is_presented;
        let title = self.title;
        let message = self.message;
        let buttons = self.buttons;

        ZStack::new((
            self.content,
            If::new(
                move || open.get(),
                move || {
                    let btns: Vec<AnyView> = buttons
                        .iter()
                        .map(|b| {
                            let act = b.action.clone();
                            let d = dismiss.clone();
                            let style = if b.destructive {
                                ButtonStyle::destructive(&theme)
                            } else {
                                ButtonStyle::primary(&theme)
                            };
                            AnyView::new(
                                Button::new(b.label.clone(), move || {
                                    act();
                                    d.set(false);
                                })
                                .style(style),
                            )
                        })
                        .collect();
                    let d_bg = dismiss.clone();
                    ZStack::new((
                        EmptyView
                            .frame(
                                Frame::new()
                                    .max_width(f32::INFINITY)
                                    .max_height(f32::INFINITY),
                            )
                            .background(Color::rgba(0.0, 0.0, 0.0, 0.45))
                            .on_tap(move || d_bg.set(false)),
                        VStack::new(
                            12,
                            (
                                Text::new(title.clone()).font(Font::title()),
                                Text::new(message.clone()).color(Color::secondary()),
                                HStack::new(8, btns),
                            ),
                        )
                        .padding(18)
                        .frame(Frame::new().width(380.0))
                        .background(Color::surface_elevated())
                        .border(1, Color::border())
                        .corner_radius(12)
                        .on_tap(|| {})
                        .accessibility_role(AccessibilityRole::AlertDialog),
                    ))
                },
            ),
        ))
    }
}

/// Alias type providing SwiftUI naming parity for action confirmation dialogs.
pub type ConfirmationDialog<C> = Alert<C>;

/// Convenience wrapper invoking OS open/save/folder picker dialogs through [`rustui_platform::DesktopPlatform`].
#[derive(Debug, Clone, Default)]
pub struct FileDialog {
    pub title: String,
    pub directory: bool,
    pub multiple: bool,
    pub filters: Vec<String>,
}

impl FileDialog {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            directory: false,
            multiple: false,
            filters: Vec::new(),
        }
    }

    pub fn directory(mut self, dir: bool) -> Self {
        self.directory = dir;
        self
    }

    pub fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    pub fn filters(mut self, filters: Vec<String>) -> Self {
        self.filters = filters;
        self
    }

    pub fn run(&self) -> Vec<std::path::PathBuf> {
        let filters = self
            .filters
            .iter()
            .map(|ext| (ext.clone(), vec![ext.clone()]))
            .collect();
        let opts = rustui_platform::FileDialogOptions {
            title: self.title.clone(),
            directory: self.directory,
            multiple: self.multiple,
            filters,
        };
        rustui_platform::DesktopPlatform::shared().open_file_dialog(&opts)
    }
}

/// Expandable disclosure section (`DisclosureGroup`).
pub struct DisclosureGroup<B: View> {
    label: String,
    is_expanded: Binding<bool>,
    content: Rc<dyn Fn() -> B>,
}

impl<B: View> std::fmt::Debug for DisclosureGroup<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisclosureGroup")
            .field("label", &self.label)
            .finish()
    }
}

impl<B: View> DisclosureGroup<B> {
    pub fn new(
        label: impl Into<String>,
        is_expanded: Binding<bool>,
        content: impl Fn() -> B + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            is_expanded,
            content: Rc::new(content),
        }
    }
}

impl<B: View> View for DisclosureGroup<B> {
    fn body(self, cx: &mut Cx) -> impl View {
        let theme = cx.env::<Theme>().unwrap_or_default();
        let exp_toggle = self.is_expanded.clone();
        let exp_btn = self.is_expanded.clone();
        let exp_watch = self.is_expanded;
        let lbl = self.label;
        let builder = self.content;

        VStack::new(
            6,
            (
                Button::new(
                    move || {
                        let arrow = if exp_btn.get() { "▾" } else { "▸" };
                        format!("{arrow} {lbl}")
                    },
                    move || exp_toggle.update(|e| *e = !*e),
                )
                .style(ButtonStyle::plain(&theme)),
                If::new(
                    move || exp_watch.get(),
                    move || builder().padding(Edges::new(0.0, 0.0, 0.0, 14.0)),
                ),
            ),
        )
        .alignment(HorizontalAlignment::Leading)
    }
}

/// Hierarchical outline tree view (`OutlineGroup`) backed by [`crate::TreeItemData`].
pub struct OutlineGroup {
    roots: Vec<crate::collections::TreeItemData>,
    on_toggle: Rc<dyn Fn(&str)>,
}

impl std::fmt::Debug for OutlineGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutlineGroup")
            .field("roots", &self.roots.len())
            .finish()
    }
}

impl OutlineGroup {
    pub fn new(
        roots: Vec<crate::collections::TreeItemData>,
        on_toggle: impl Fn(&str) + 'static,
    ) -> Self {
        Self {
            roots,
            on_toggle: Rc::new(on_toggle),
        }
    }
}

impl View for OutlineGroup {
    fn body(self, _cx: &mut Cx) -> impl View {
        let cb = self.on_toggle;
        crate::collections::Tree::new(self.roots, move |id| cb(id))
    }
}

/// macOS/SwiftUI-aligned settings and data-entry form container (`Form`).
#[derive(Debug)]
pub struct Form<C> {
    children: C,
}

impl<C: ViewSequence> Form<C> {
    pub fn new(children: C) -> Self {
        Self { children }
    }
}

impl<C: ViewSequence> Primitive for Form<C> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let node = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
            spacing: 16.0,
            alignment: HorizontalAlignment::Leading,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(node) {
                n.layout_props.padding = Edges::all(16.0);
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .alignment(Alignment::TOP_LEADING),
                );
                n.accessibility.role = AccessibilityRole::Group;
                n.accessibility.label = Some("Form".to_owned());
            }
        });
        self.children.build_all(cx, node);
        node
    }
}

impl<C: ViewSequence> View for Form<C> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Grouped settings/form section container with header and optional footer (`Section`).
#[derive(Debug)]
pub struct Section<C> {
    header: String,
    footer: Option<String>,
    content: C,
}

impl<C: ViewSequence> Section<C> {
    pub fn new(header: impl Into<String>, content: C) -> Self {
        Self {
            header: header.into(),
            footer: None,
            content,
        }
    }

    pub fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }
}

impl<C: ViewSequence> Primitive for Section<C> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let standard = theme.uses_default_palette();
        let surface_col = if standard {
            Color::surface()
        } else {
            theme.colors.surface
        };
        let border_col = if standard {
            Color::border()
        } else {
            theme.colors.border
        };

        let outer = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
            spacing: 6.0,
            alignment: HorizontalAlignment::Leading,
        }));
        let a11y_label = if self.header.is_empty() {
            "Section".to_owned()
        } else {
            self.header.clone()
        };
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(outer) {
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .alignment(Alignment::TOP_LEADING),
                );
                n.accessibility.role = AccessibilityRole::Group;
                n.accessibility.label = Some(a11y_label);
            }
        });

        if !self.header.is_empty() {
            let hdr = Text::new(self.header)
                .font(rustui_core::FontSpec {
                    weight: rustui_core::FontWeight::SemiBold,
                    ..Font::caption()
                })
                .color(Color::secondary())
                .build_node(cx);
            cx.runtime.attach_child(outer, hdr);
        }

        let card = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
            spacing: 10.0,
            alignment: HorizontalAlignment::Leading,
        }));
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(card) {
                n.layout_props.padding = Edges::all(12.0);
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .alignment(Alignment::TOP_LEADING),
                );
                n.visual_props.background = Some(surface_col);
                n.visual_props.border = Some((1.0, border_col));
                n.visual_props.corner_radii = rustui_core::CornerRadii::all(theme.radius_md);
            }
        });
        self.content.build_all(cx, card);
        cx.runtime.attach_child(outer, card);

        if let Some(footer_text) = self.footer {
            let ftr = Text::new(footer_text)
                .font(Font::caption())
                .color(Color::secondary())
                .build_node(cx);
            cx.runtime.attach_child(outer, ftr);
        }

        outer
    }
}

impl<C: ViewSequence> View for Section<C> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// macOS SwiftUI-aligned titled card box container (`GroupBox`).
#[derive(Debug)]
pub struct GroupBox<C> {
    label: String,
    content: C,
}

impl<C: ViewSequence> GroupBox<C> {
    pub fn new(label: impl Into<String>, content: C) -> Self {
        Self {
            label: label.into(),
            content,
        }
    }
}

impl<C: ViewSequence> Primitive for GroupBox<C> {
    fn build(self, cx: &mut BuildCx) -> NodeId {
        let theme = resolve_theme(cx);
        let standard = theme.uses_default_palette();
        let surface_col = if standard {
            Color::surface()
        } else {
            theme.colors.surface
        };
        let border_col = if standard {
            Color::border()
        } else {
            theme.colors.border
        };

        let box_node = cx.alloc_node(NodeKind::Container(ContainerKind::VStack {
            spacing: 8.0,
            alignment: HorizontalAlignment::Leading,
        }));
        let lbl_str = self.label.clone();
        cx.runtime.with_inner_mut(|inner| {
            if let Some(n) = inner.nodes.get_mut(box_node) {
                n.layout_props.padding = Edges::all(12.0);
                n.layout_props.frame = Some(
                    Frame::new()
                        .max_width(f32::INFINITY)
                        .alignment(Alignment::TOP_LEADING),
                );
                n.visual_props.background = Some(surface_col);
                n.visual_props.border = Some((1.0, border_col));
                n.visual_props.corner_radii = rustui_core::CornerRadii::all(theme.radius_md);
                n.accessibility.role = AccessibilityRole::Group;
                n.accessibility.label = Some(lbl_str);
            }
        });
        if !self.label.is_empty() {
            let hdr = Text::new(self.label)
                .font(Font::headline())
                .color(Color::primary())
                .build_node(cx);
            cx.runtime.attach_child(box_node, hdr);
        }
        self.content.build_all(cx, box_node);
        box_node
    }
}

impl<C: ViewSequence> View for GroupBox<C> {
    fn body(self, _cx: &mut Cx) -> impl View {
        EmptyView
    }
    fn build_node(self, cx: &mut BuildCx) -> NodeId {
        Primitive::build(self, cx)
    }
}

/// Horizontal label-and-control form row (`LabeledContent`).
#[derive(Debug)]
pub struct LabeledContent<V> {
    label: String,
    content: V,
}

impl<V: View> LabeledContent<V> {
    pub fn new(label: impl Into<String>, content: V) -> Self {
        Self {
            label: label.into(),
            content,
        }
    }
}

impl<V: View> View for LabeledContent<V> {
    fn body(self, _cx: &mut Cx) -> impl View {
        HStack::new(
            12,
            (
                Text::new(self.label).color(Color::primary()),
                Spacer::new(),
                self.content,
            ),
        )
        .alignment(VerticalAlignment::Center)
    }
}
