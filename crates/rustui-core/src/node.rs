//! Retained UI node tree, dirty flags, environment, layout/visual/accessibility/event data.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::rc::Rc;

use slotmap::new_key_type;

use crate::signal::{Binding, EffectId, MemoId, SignalKey, StateId, Subscriber};
use crate::types::{
    AccessibilityRole, Alignment, Animation, BoxShadow, Color, ContentMode, CornerRadii,
    DragEvent, Edges, Frame, HorizontalAlignment, KeyEvent, PinchEvent, Point, PointerEvent, Rect,
    ScrollEvent, Size, TextDirection, Transition,
};

new_key_type! {
    /// Stable arena key identifying a retained UI node.
    pub struct NodeId;
}

/// Dirty flags tracking what phases a node requires on the next frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DirtyFlags(u8);

impl DirtyFlags {
    pub const NONE: Self = Self(0);
    pub const REBUILD: Self = Self(1 << 0);
    pub const LAYOUT: Self = Self(1 << 1);
    pub const PAINT: Self = Self(1 << 2);
    pub const ACCESSIBILITY: Self = Self(1 << 3);
    pub const ALL: Self = Self(Self::REBUILD.0 | Self::LAYOUT.0 | Self::PAINT.0 | Self::ACCESSIBILITY.0);

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[inline]
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    #[inline]
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    #[inline]
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

impl std::ops::BitOr for DirtyFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for DirtyFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

/// Environment type-map inherited from ancestor nodes (`cx.env::<T>()`).
#[derive(Debug, Clone, Default)]
pub struct Environment {
    values: HashMap<TypeId, Rc<dyn Any>>,
}

impl Environment {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<T: 'static>(&mut self, value: T) {
        self.values.insert(TypeId::of::<T>(), Rc::new(value));
    }

    pub fn get<T: Clone + 'static>(&self) -> Option<T> {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|v| v.downcast_ref::<T>())
            .cloned()
    }

    pub fn contains<T: 'static>(&self) -> bool {
        self.values.contains_key(&TypeId::of::<T>())
    }

    pub fn inherit_from(&mut self, parent: &Self) {
        for (k, v) in &parent.values {
            self.values.entry(*k).or_insert_with(|| v.clone());
        }
    }
}

/// Metadata map attached to children for custom [`Layout`] containers (`LayoutValue`).
#[derive(Debug, Clone, Default)]
pub struct LayoutValueMap {
    values: HashMap<TypeId, Rc<dyn Any>>,
}

impl LayoutValueMap {
    pub fn insert<T: 'static>(&mut self, value: T) {
        self.values.insert(TypeId::of::<T>(), Rc::new(value));
    }

    pub fn get<T: Clone + 'static>(&self) -> Option<T> {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|v| v.downcast_ref::<T>())
            .cloned()
    }
}

/// Text truncation behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TruncationMode {
    #[default]
    Tail,
    Middle,
    Head,
    Clip,
}

/// Font weight descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontWeight {
    Thin,
    Light,
    #[default]
    Regular,
    Medium,
    SemiBold,
    Bold,
    Black,
}

impl FontWeight {
    pub fn numeric(self) -> f32 {
        match self {
            Self::Thin => 200.0,
            Self::Light => 300.0,
            Self::Regular => 400.0,
            Self::Medium => 500.0,
            Self::SemiBold => 600.0,
            Self::Bold => 700.0,
            Self::Black => 900.0,
        }
    }
}

/// Font specification used by text nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct FontSpec {
    pub family: Option<String>,
    pub size: f32,
    pub weight: FontWeight,
    pub italic: bool,
    pub monospace: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: None,
            size: 14.0,
            weight: FontWeight::Regular,
            italic: false,
            monospace: false,
        }
    }
}

/// Data stored on text nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct TextNodeData {
    pub text: String,
    pub spans: Vec<TextSpanData>,
    pub font: FontSpec,
    pub color: Option<Color>,
    pub line_limit: Option<usize>,
    pub truncation: TruncationMode,
    pub alignment: HorizontalAlignment,
    pub underline: bool,
    pub strikethrough: bool,
    /// Optional inline IME preedit range `(text, cursor_start, cursor_end)` rendered with underline.
    pub preedit: Option<(String, usize, usize)>,
    /// Selection highlight range in byte offsets within `text`.
    pub selection: Option<(usize, usize)>,
    /// Caret byte offset for editable fields when focused.
    pub caret: Option<usize>,
}

impl Default for TextNodeData {
    fn default() -> Self {
        Self {
            text: String::new(),
            spans: Vec::new(),
            font: FontSpec::default(),
            color: None,
            line_limit: None,
            truncation: TruncationMode::Tail,
            alignment: HorizontalAlignment::Leading,
            underline: false,
            strikethrough: false,
            preedit: None,
            selection: None,
            caret: None,
        }
    }
}

/// Rich text span metadata attached to a text node.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpanData {
    pub text: String,
    pub font: Option<FontSpec>,
    pub color: Option<Color>,
    pub underline: bool,
    pub strikethrough: bool,
}

/// Image payload for `Image` nodes.
#[derive(Debug, Clone)]
pub enum ImageSourceData {
    EncodedBytes {
        key: u64,
        bytes: Rc<Vec<u8>>,
        is_svg: bool,
    },
    Rgba {
        key: u64,
        width: u32,
        height: u32,
        pixels: Rc<Vec<u8>>,
    },
    PendingPlaceholder,
}

/// Data stored on `Image` nodes.
#[derive(Debug, Clone)]
pub struct ImageNodeData {
    pub source: ImageSourceData,
    pub content_mode: ContentMode,
    pub tint: Option<Color>,
}

/// Scroll axis flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollAxes {
    #[default]
    Vertical,
    Horizontal,
    Both,
}

/// Data stored on `ScrollView` nodes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollNodeData {
    pub axes: ScrollAxes,
    pub offset: Point,
    pub content_size: Size<f32>,
    pub velocity: Point,
}

/// Data stored on virtualized `LazyVStack` / `List` / `Table` nodes.
#[derive(Clone)]
pub struct VirtualListNodeData {
    pub item_count: usize,
    pub estimated_row_height: f32,
    pub scroll_offset: f32,
    pub overscan: usize,
    pub visible_start: usize,
    pub visible_end: usize,
    pub spacing: f32,
}

impl std::fmt::Debug for VirtualListNodeData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualListNodeData")
            .field("item_count", &self.item_count)
            .field("estimated_row_height", &self.estimated_row_height)
            .field("scroll_offset", &self.scroll_offset)
            .field("visible_start", &self.visible_start)
            .field("visible_end", &self.visible_end)
            .finish()
    }
}

/// Grid column sizing rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrack {
    Fixed(f32),
    Flexible { min: f32, max: f32 },
}

/// Built-in layout container descriptors.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerKind {
    VStack {
        spacing: f32,
        alignment: HorizontalAlignment,
    },
    HStack {
        spacing: f32,
        alignment: VerticalAlignmentKind,
    },
    ZStack {
        alignment: Alignment,
    },
    Grid {
        columns: Vec<GridTrack>,
        h_spacing: f32,
        v_spacing: f32,
    },
    Spacer {
        min_length: f32,
    },
    Divider {
        thickness: f32,
        color: Color,
    },
    Fragment,
    CustomLayout,
}

/// Vertical alignment alias for `HStack`.
pub type VerticalAlignmentKind = crate::types::VerticalAlignment;

/// Custom drawing callback type for `Canvas` nodes (downcast to `&mut DisplayList` in `rustui-render`).
pub type CanvasPaintFn = Rc<dyn Fn(Rect, &mut dyn Any)>;

/// Data stored on `Canvas` nodes.
#[derive(Clone)]
pub struct CanvasNodeData {
    pub painter: CanvasPaintFn,
}

impl std::fmt::Debug for CanvasNodeData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanvasNodeData").finish_non_exhaustive()
    }
}

/// Data stored on `Shader` nodes.
#[derive(Debug, Clone)]
pub struct ShaderNodeData {
    pub shader_id: u64,
    pub wgsl: String,
    pub uniform_data: Vec<u8>,
}

/// Control-specific state stored on widget nodes for rendering, focus, and accessibility.
#[derive(Debug, Clone, Default)]
pub struct ControlNodeData {
    pub kind: &'static str,
    pub is_hovered: bool,
    pub is_pressed: bool,
    pub is_disabled: bool,
    pub value_bool: Option<bool>,
    pub value_f32: Option<f32>,
    pub value_text: Option<String>,
}

/// Dynamic control-flow node state (`If`, `Switch`, `ForEach`).
pub enum DynamicNodeData {
    If {
        last_branch: Option<bool>,
        condition: Rc<dyn Fn() -> bool>,
        then_builder: Rc<dyn Fn(&mut crate::view::BuildCx) -> NodeId>,
        else_builder: Option<Rc<dyn Fn(&mut crate::view::BuildCx) -> NodeId>>,
    },
    Switch {
        last_case: Option<u64>,
        selector: Rc<dyn Fn() -> u64>,
        builder: Rc<dyn Fn(u64, &mut crate::view::BuildCx) -> NodeId>,
    },
    ForEach {
        keyed_children: Vec<(u64, NodeId)>,
        rebuilder: Rc<dyn Fn(&mut crate::view::BuildCx, &mut Vec<(u64, NodeId)>) -> Vec<(u64, NodeId)>>,
    },
}

impl std::fmt::Debug for DynamicNodeData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::If { last_branch, .. } => {
                f.debug_struct("If").field("last_branch", last_branch).finish()
            }
            Self::Switch { last_case, .. } => {
                f.debug_struct("Switch").field("last_case", last_case).finish()
            }
            Self::ForEach { keyed_children, .. } => f
                .debug_struct("ForEach")
                .field("count", &keyed_children.len())
                .finish(),
        }
    }
}

/// Node kind variant stored in the retained tree.
#[derive(Debug)]
pub enum NodeKind {
    Composite,
    Modifier,
    Container(ContainerKind),
    Text(TextNodeData),
    Image(ImageNodeData),
    Scroll(ScrollNodeData),
    VirtualList(VirtualListNodeData),
    Canvas(CanvasNodeData),
    Shader(ShaderNodeData),
    Control(ControlNodeData),
    Dynamic(DynamicNodeData),
}

/// Layout-affecting properties on a node.
#[derive(Debug, Clone, Default)]
pub struct LayoutProps {
    pub padding: Edges,
    pub frame: Option<Frame>,
    pub fixed_horizontal: bool,
    pub fixed_vertical: bool,
    pub aspect_ratio: Option<(f32, ContentMode)>,
    pub offset: Point,
    pub layout_priority: f64,
    pub custom_alignment_x: Option<f32>,
    pub custom_alignment_y: Option<f32>,
    pub layout_values: LayoutValueMap,
    pub direction_override: Option<TextDirection>,
}

impl LayoutProps {
    /// Returns true if this node forms a fixed-size boundary that isolates parent layout.
    pub fn is_fixed_size_boundary(&self) -> bool {
        self.frame
            .as_ref()
            .is_some_and(|f| f.is_fixed_width() && f.is_fixed_height())
    }
}

/// Visual rendering properties on a node.
#[derive(Debug, Clone)]
pub struct VisualProps {
    pub background: Option<Color>,
    pub border: Option<(f32, Color)>,
    pub corner_radii: CornerRadii,
    pub opacity: f32,
    pub shadow: Option<BoxShadow>,
    pub clip_to_bounds: bool,
    pub hidden: bool,
}

impl Default for VisualProps {
    fn default() -> Self {
        Self {
            background: None,
            border: None,
            corner_radii: CornerRadii::ZERO,
            opacity: 1.0,
            shadow: None,
            clip_to_bounds: false,
            hidden: false,
        }
    }
}

/// Accessibility metadata attached to every retained node from creation.
#[derive(Debug, Clone, Default)]
pub struct AccessibilityNodeData {
    pub role: AccessibilityRole,
    pub label: Option<String>,
    pub value: Option<String>,
    pub hidden: bool,
    pub supports_click: bool,
    pub supports_focus: bool,
    pub supports_set_value: bool,
    pub supports_increment: bool,
    pub supports_decrement: bool,
}

/// Focus metadata on a node.
#[derive(Clone, Default)]
pub struct FocusNodeData {
    pub focusable: bool,
    pub focused: bool,
    pub tab_index: i32,
    pub focused_binding: Option<Binding<bool>>,
    pub accepts_text_input: bool,
}

impl std::fmt::Debug for FocusNodeData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FocusNodeData")
            .field("focusable", &self.focusable)
            .field("focused", &self.focused)
            .field("tab_index", &self.tab_index)
            .field("accepts_text_input", &self.accepts_text_input)
            .finish()
    }
}

/// Event callbacks attached to a node.
#[derive(Clone, Default)]
pub struct EventHandlers {
    pub on_tap: Option<Rc<dyn Fn()>>,
    pub on_double_tap: Option<Rc<dyn Fn()>>,
    pub on_long_press: Option<Rc<dyn Fn()>>,
    pub on_hover: Option<Rc<dyn Fn(bool)>>,
    pub on_pointer: Option<Rc<dyn Fn(PointerEvent)>>,
    pub on_drag: Option<Rc<dyn Fn(DragEvent)>>,
    pub on_scroll: Option<Rc<dyn Fn(ScrollEvent) -> bool>>,
    pub on_pinch: Option<Rc<dyn Fn(PinchEvent)>>,
    pub on_key: Option<Rc<dyn Fn(KeyEvent) -> bool>>,
    pub on_ime_preedit: Option<Rc<dyn Fn(String, Option<(usize, usize)>)>>,
    pub on_ime_commit: Option<Rc<dyn Fn(String)>>,
    pub on_accessibility_set_value: Option<Rc<dyn Fn(String)>>,
    pub on_accessibility_increment: Option<Rc<dyn Fn()>>,
    pub on_accessibility_decrement: Option<Rc<dyn Fn()>>,
}

impl std::fmt::Debug for EventHandlers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventHandlers")
            .field("has_on_tap", &self.on_tap.is_some())
            .field("has_on_hover", &self.on_hover.is_some())
            .field("has_on_drag", &self.on_drag.is_some())
            .field("has_on_key", &self.on_key.is_some())
            .finish()
    }
}

/// Cached layout measurement result for incremental invalidation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CachedLayout {
    pub last_proposal: Option<Size<Option<f32>>>,
    pub measured_size: Size<f32>,
    pub bounds: Rect,
}

/// Hook slot metadata recorded for `cx.state(...)` validation.
#[derive(Debug, Clone)]
pub struct HookSlot {
    pub state_id: StateId,
    pub type_id: TypeId,
    pub location: &'static std::panic::Location<'static>,
}

/// Reactive updater closure bound to a specific node.
#[derive(Clone)]
pub struct ReactiveUpdater {
    pub id: u64,
    pub run: Rc<dyn Fn(&mut Node)>,
}

impl std::fmt::Debug for ReactiveUpdater {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReactiveUpdater").field("id", &self.id).finish()
    }
}

/// Animation attachment on a node.
#[derive(Debug, Clone, Copy)]
pub struct NodeAnimation {
    pub animation: Animation,
    pub transition: Option<Transition>,
}

/// Retained node in the UI tree.
#[derive(Debug)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub explicit_key: Option<u64>,
    pub kind: NodeKind,
    pub dirty: DirtyFlags,
    pub env: Environment,
    pub layout_props: LayoutProps,
    pub visual_props: VisualProps,
    pub accessibility: AccessibilityNodeData,
    pub focus: FocusNodeData,
    pub events: EventHandlers,
    pub layout_cache: CachedLayout,
    pub animation: Option<NodeAnimation>,
    pub custom_layout: Option<Rc<dyn Any>>,
    pub hook_slots: Vec<HookSlot>,
    pub owned_states: Vec<StateId>,
    pub owned_memos: Vec<MemoId>,
    pub owned_effects: Vec<EffectId>,
    pub subscriptions: Vec<(SignalKey, Subscriber)>,
    pub updaters: Vec<ReactiveUpdater>,
    pub paint_dirty_generation: u64,
}

impl Node {
    pub fn new(id: NodeId, kind: NodeKind) -> Self {
        Self {
            id,
            parent: None,
            children: Vec::new(),
            explicit_key: None,
            kind,
            dirty: DirtyFlags::ALL,
            env: Environment::new(),
            layout_props: LayoutProps::default(),
            visual_props: VisualProps::default(),
            accessibility: AccessibilityNodeData::default(),
            focus: FocusNodeData::default(),
            events: EventHandlers::default(),
            layout_cache: CachedLayout::default(),
            animation: None,
            custom_layout: None,
            hook_slots: Vec::new(),
            owned_states: Vec::new(),
            owned_memos: Vec::new(),
            owned_effects: Vec::new(),
            subscriptions: Vec::new(),
            updaters: Vec::new(),
            paint_dirty_generation: 0,
        }
    }

    #[inline]
    pub fn bounds(&self) -> Rect {
        self.layout_cache.bounds
    }
}
