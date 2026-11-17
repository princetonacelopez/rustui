//! Core geometry, color, input, accessibility, and animation value types.

use std::ops::{Add, Sub};

/// A 2D point in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn distance_to(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

impl Add for Point {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Point {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

/// A 2D size generic over component type (typically `f32` or `Option<f32>`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size<T = f32> {
    pub width: T,
    pub height: T,
}

impl<T> Size<T> {
    #[inline]
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

impl Size<f32> {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
    };
    pub const INFINITY: Self = Self {
        width: f32::INFINITY,
        height: f32::INFINITY,
    };

    #[inline]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self {
            width: self.width.clamp(min.width, max.width),
            height: self.height.clamp(min.height, max.height),
        }
    }
}

impl Size<Option<f32>> {
    pub const UNSPECIFIED: Self = Self {
        width: None,
        height: None,
    };
    pub const ZERO_PROPOSED: Self = Self {
        width: Some(0.0),
        height: Some(0.0),
    };
    pub const INFINITY: Self = Self {
        width: Some(f32::INFINITY),
        height: Some(f32::INFINITY),
    };

    #[inline]
    pub fn replacing_unspecified_with(self, fallback: Size<f32>) -> Size<f32> {
        Size {
            width: self.width.unwrap_or(fallback.width),
            height: self.height.unwrap_or(fallback.height),
        }
    }
}

/// An axis-aligned rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };

    #[inline]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[inline]
    pub const fn from_origin_size(origin: Point, size: Size<f32>) -> Self {
        Self {
            x: origin.x,
            y: origin.y,
            width: size.width,
            height: size.height,
        }
    }

    #[inline]
    pub const fn origin(&self) -> Point {
        Point::new(self.x, self.y)
    }

    #[inline]
    pub const fn size(&self) -> Size<f32> {
        Size::new(self.width, self.height)
    }

    #[inline]
    pub fn min_x(&self) -> f32 {
        self.x
    }

    #[inline]
    pub fn min_y(&self) -> f32 {
        self.y
    }

    #[inline]
    pub fn max_x(&self) -> f32 {
        self.x + self.width
    }

    #[inline]
    pub fn max_y(&self) -> f32 {
        self.y + self.height
    }

    #[inline]
    pub fn mid_x(&self) -> f32 {
        self.x + self.width * 0.5
    }

    #[inline]
    pub fn mid_y(&self) -> f32 {
        self.y + self.height * 0.5
    }

    #[inline]
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.max_x() && p.y >= self.y && p.y <= self.max_y()
    }

    #[inline]
    pub fn intersects(&self, other: &Self) -> bool {
        self.x < other.max_x()
            && self.max_x() > other.x
            && self.y < other.max_y()
            && self.max_y() > other.y
    }

    #[inline]
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.max_x().min(other.max_x());
        let y1 = self.max_y().min(other.max_y());
        if x1 > x0 && y1 > y0 {
            Some(Self::new(x0, y0, x1 - x0, y1 - y0))
        } else {
            None
        }
    }

    #[inline]
    pub fn union(&self, other: &Self) -> Self {
        if self.width <= 0.0 || self.height <= 0.0 {
            return *other;
        }
        if other.width <= 0.0 || other.height <= 0.0 {
            return *self;
        }
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = self.max_x().max(other.max_x());
        let y1 = self.max_y().max(other.max_y());
        Self::new(x0, y0, x1 - x0, y1 - y0)
    }

    #[inline]
    pub fn inset(&self, edges: Edges) -> Self {
        Self {
            x: self.x + edges.left,
            y: self.y + edges.top,
            width: (self.width - edges.horizontal()).max(0.0),
            height: (self.height - edges.vertical()).max(0.0),
        }
    }

    #[inline]
    pub fn expand(&self, amount: f32) -> Self {
        Self {
            x: self.x - amount,
            y: self.y - amount,
            width: (self.width + amount * 2.0).max(0.0),
            height: (self.height + amount * 2.0).max(0.0),
        }
    }

    #[inline]
    pub fn translate(&self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.width, self.height)
    }
}

/// Edge insets in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub const ZERO: Self = Self {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };

    #[inline]
    pub const fn all(v: f32) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    #[inline]
    pub const fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    #[inline]
    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    #[inline]
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    #[inline]
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

/// Per-corner rounding radii in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CornerRadii {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl CornerRadii {
    pub const ZERO: Self = Self::all(0.0);

    #[inline]
    pub const fn all(r: f32) -> Self {
        Self {
            top_left: r,
            top_right: r,
            bottom_right: r,
            bottom_left: r,
        }
    }

    #[inline]
    pub fn max_radius(&self) -> f32 {
        self.top_left
            .max(self.top_right)
            .max(self.bottom_right)
            .max(self.bottom_left)
    }
}

impl From<f32> for CornerRadii {
    fn from(value: f32) -> Self {
        Self::all(value)
    }
}

/// Trait allowing ergonomic `12` or `12.0` logical pixel arguments.
pub trait IntoPx {
    fn into_px(self) -> f32;
}

impl IntoPx for f32 {
    #[inline]
    fn into_px(self) -> f32 {
        self
    }
}

impl IntoPx for i32 {
    #[inline]
    fn into_px(self) -> f32 {
        self as f32
    }
}

/// Trait allowing `.padding(16)`, `.padding(16.0)`, or `.padding(Edges::symmetric(...))`.
pub trait IntoEdges {
    fn into_edges(self) -> Edges;
}

impl IntoEdges for Edges {
    #[inline]
    fn into_edges(self) -> Edges {
        self
    }
}

impl IntoEdges for f32 {
    #[inline]
    fn into_edges(self) -> Edges {
        Edges::all(self)
    }
}

impl IntoEdges for i32 {
    #[inline]
    fn into_edges(self) -> Edges {
        Edges::all(self as f32)
    }
}

/// Linear RGBA color with optional semantic token resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    pub semantic: Option<SemanticColor>,
}

/// Semantic color tokens resolved via the active `Theme` environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticColor {
    Primary,
    Secondary,
    Surface,
    SurfaceElevated,
    Background,
    Accent,
    Border,
    Destructive,
    Success,
    Warning,
}

/// Resolved color palette stored in node environments for dynamic semantic color lookup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemePalette {
    pub is_dark: bool,
    pub primary: Color,
    pub secondary: Color,
    pub background: Color,
    pub surface: Color,
    pub surface_elevated: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub border: Color,
    pub destructive: Color,
    pub success: Color,
    pub warning: Color,
}

impl Default for ThemePalette {
    fn default() -> Self {
        Self::light()
    }
}

impl ThemePalette {
    pub fn light() -> Self {
        Self {
            is_dark: false,
            primary: Color::from_hex(0x0f172a),
            secondary: Color::from_hex(0x475569),
            background: Color::from_hex(0xf8fafc),
            surface: Color::from_hex(0xf1f5f9),
            surface_elevated: Color::WHITE,
            accent: Color::from_hex(0x2563eb),
            on_accent: Color::WHITE,
            border: Color::from_hex(0xcbd5e1),
            destructive: Color::from_hex(0xdc2626),
            success: Color::from_hex(0x16a34a),
            warning: Color::from_hex(0xd97706),
        }
    }

    pub fn dark() -> Self {
        Self {
            is_dark: true,
            primary: Color::from_hex(0xf8fafc),
            secondary: Color::from_hex(0x94a3b8),
            background: Color::from_hex(0x0f172a),
            surface: Color::from_hex(0x1e293b),
            surface_elevated: Color::from_hex(0x334155),
            accent: Color::from_hex(0x3b82f6),
            on_accent: Color::WHITE,
            border: Color::from_hex(0x475569),
            destructive: Color::from_hex(0xef4444),
            success: Color::from_hex(0x22c55e),
            warning: Color::from_hex(0xf59e0b),
        }
    }

    /// Resolves a semantic [`Color`] using this palette while preserving its alpha channel.
    #[inline]
    pub fn resolve(&self, color: Color) -> Color {
        match color.semantic {
            None => color,
            Some(SemanticColor::Primary) => self.primary.with_alpha(color.a),
            Some(SemanticColor::Secondary) => self.secondary.with_alpha(color.a),
            Some(SemanticColor::Background) => self.background.with_alpha(color.a),
            Some(SemanticColor::Surface) => self.surface.with_alpha(color.a),
            Some(SemanticColor::SurfaceElevated) => self.surface_elevated.with_alpha(color.a),
            Some(SemanticColor::Accent) => self.accent.with_alpha(color.a),
            Some(SemanticColor::Border) => self.border.with_alpha(color.a),
            Some(SemanticColor::Destructive) => self.destructive.with_alpha(color.a),
            Some(SemanticColor::Success) => self.success.with_alpha(color.a),
            Some(SemanticColor::Warning) => self.warning.with_alpha(color.a),
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::TRANSPARENT
    }
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);
    pub const RED: Self = Self::rgb(0.92, 0.26, 0.24);
    pub const GREEN: Self = Self::rgb(0.20, 0.78, 0.35);
    pub const BLUE: Self = Self::rgb(0.04, 0.52, 1.0);
    pub const YELLOW: Self = Self::rgb(1.0, 0.80, 0.0);
    pub const ORANGE: Self = Self::rgb(1.0, 0.58, 0.0);
    pub const PURPLE: Self = Self::rgb(0.69, 0.32, 0.87);
    pub const GRAY: Self = Self::rgb(0.56, 0.56, 0.58);

    #[inline]
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            r,
            g,
            b,
            a,
            semantic: None,
        }
    }

    #[inline]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::rgba(r, g, b, 1.0)
    }

    #[inline]
    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::rgba(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            f32::from(a) / 255.0,
        )
    }

    #[inline]
    pub fn from_hex(hex: u32) -> Self {
        let r = ((hex >> 16) & 0xff) as u8;
        let g = ((hex >> 8) & 0xff) as u8;
        let b = (hex & 0xff) as u8;
        Self::from_rgba8(r, g, b, 255)
    }

    #[inline]
    pub const fn with_alpha(mut self, a: f32) -> Self {
        self.a = a;
        self
    }

    #[inline]
    pub fn to_rgba8(self) -> [u8; 4] {
        [
            (self.r.clamp(0.0, 1.0) * 255.0).round() as u8,
            (self.g.clamp(0.0, 1.0) * 255.0).round() as u8,
            (self.b.clamp(0.0, 1.0) * 255.0).round() as u8,
            (self.a.clamp(0.0, 1.0) * 255.0).round() as u8,
        ]
    }

    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self::rgba(
            self.r + (other.r - self.r) * t,
            self.g + (other.g - self.g) * t,
            self.b + (other.b - self.b) * t,
            self.a + (other.a - self.a) * t,
        )
    }

    #[inline]
    pub const fn primary() -> Self {
        Self {
            r: 0.08,
            g: 0.09,
            b: 0.11,
            a: 1.0,
            semantic: Some(SemanticColor::Primary),
        }
    }

    #[inline]
    pub const fn secondary() -> Self {
        Self {
            r: 0.42,
            g: 0.45,
            b: 0.50,
            a: 1.0,
            semantic: Some(SemanticColor::Secondary),
        }
    }

    #[inline]
    pub const fn surface() -> Self {
        Self {
            r: 0.96,
            g: 0.97,
            b: 0.98,
            a: 1.0,
            semantic: Some(SemanticColor::Surface),
        }
    }

    #[inline]
    pub const fn surface_elevated() -> Self {
        Self {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
            semantic: Some(SemanticColor::SurfaceElevated),
        }
    }

    #[inline]
    pub const fn background() -> Self {
        Self {
            r: 0.98,
            g: 0.98,
            b: 0.99,
            a: 1.0,
            semantic: Some(SemanticColor::Background),
        }
    }

    #[inline]
    pub const fn accent() -> Self {
        Self {
            r: 0.14,
            g: 0.48,
            b: 0.98,
            a: 1.0,
            semantic: Some(SemanticColor::Accent),
        }
    }

    #[inline]
    pub const fn border() -> Self {
        Self {
            r: 0.84,
            g: 0.86,
            b: 0.90,
            a: 1.0,
            semantic: Some(SemanticColor::Border),
        }
    }

    #[inline]
    pub const fn destructive() -> Self {
        Self {
            r: 0.88,
            g: 0.22,
            b: 0.24,
            a: 1.0,
            semantic: Some(SemanticColor::Destructive),
        }
    }
}

/// Text/layout reading direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDirection {
    #[default]
    Ltr,
    Rtl,
}

/// Horizontal alignment respecting text direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HorizontalAlignment {
    Leading,
    #[default]
    Center,
    Trailing,
}

impl HorizontalAlignment {
    /// Converts alignment to normalized `[0.0, 1.0]` factor accounting for RTL.
    #[inline]
    pub fn factor(self, dir: TextDirection) -> f32 {
        match (self, dir) {
            (Self::Leading, TextDirection::Ltr) | (Self::Trailing, TextDirection::Rtl) => 0.0,
            (Self::Center, _) => 0.5,
            (Self::Trailing, TextDirection::Ltr) | (Self::Leading, TextDirection::Rtl) => 1.0,
        }
    }
}

/// Vertical alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlignment {
    Top,
    #[default]
    Center,
    Bottom,
}

impl VerticalAlignment {
    #[inline]
    pub fn factor(self) -> f32 {
        match self {
            Self::Top => 0.0,
            Self::Center => 0.5,
            Self::Bottom => 1.0,
        }
    }
}

/// 2D alignment within a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Alignment {
    pub horizontal: HorizontalAlignment,
    pub vertical: VerticalAlignment,
}

impl Alignment {
    pub const TOP_LEADING: Self = Self::new(HorizontalAlignment::Leading, VerticalAlignment::Top);
    pub const TOP: Self = Self::new(HorizontalAlignment::Center, VerticalAlignment::Top);
    pub const TOP_TRAILING: Self = Self::new(HorizontalAlignment::Trailing, VerticalAlignment::Top);
    pub const LEADING: Self = Self::new(HorizontalAlignment::Leading, VerticalAlignment::Center);
    pub const CENTER: Self = Self::new(HorizontalAlignment::Center, VerticalAlignment::Center);
    pub const TRAILING: Self = Self::new(HorizontalAlignment::Trailing, VerticalAlignment::Center);
    pub const BOTTOM_LEADING: Self =
        Self::new(HorizontalAlignment::Leading, VerticalAlignment::Bottom);
    pub const BOTTOM: Self = Self::new(HorizontalAlignment::Center, VerticalAlignment::Bottom);
    pub const BOTTOM_TRAILING: Self =
        Self::new(HorizontalAlignment::Trailing, VerticalAlignment::Bottom);

    #[inline]
    pub const fn new(horizontal: HorizontalAlignment, vertical: VerticalAlignment) -> Self {
        Self {
            horizontal,
            vertical,
        }
    }
}

/// Content scaling behavior for `aspect_ratio` and `Image`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContentMode {
    #[default]
    Fit,
    Fill,
}

/// Frame constraints attached via `.frame(...)`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Frame {
    pub min_width: Option<f32>,
    pub ideal_width: Option<f32>,
    pub max_width: Option<f32>,
    pub min_height: Option<f32>,
    pub ideal_height: Option<f32>,
    pub max_height: Option<f32>,
    pub alignment: Alignment,
}

impl Frame {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fixed(width: f32, height: f32) -> Self {
        Self {
            min_width: Some(width),
            ideal_width: Some(width),
            max_width: Some(width),
            min_height: Some(height),
            ideal_height: Some(height),
            max_height: Some(height),
            alignment: Alignment::CENTER,
        }
    }

    pub fn width(mut self, w: impl IntoPx) -> Self {
        let w = w.into_px();
        self.min_width = Some(w);
        self.ideal_width = Some(w);
        self.max_width = Some(w);
        self
    }

    pub fn height(mut self, h: impl IntoPx) -> Self {
        let h = h.into_px();
        self.min_height = Some(h);
        self.ideal_height = Some(h);
        self.max_height = Some(h);
        self
    }

    pub fn min_width(mut self, w: impl IntoPx) -> Self {
        self.min_width = Some(w.into_px());
        self
    }

    pub fn ideal_width(mut self, w: impl IntoPx) -> Self {
        self.ideal_width = Some(w.into_px());
        self
    }

    pub fn max_width(mut self, w: impl IntoPx) -> Self {
        self.max_width = Some(w.into_px());
        self
    }

    pub fn min_height(mut self, h: impl IntoPx) -> Self {
        self.min_height = Some(h.into_px());
        self
    }

    pub fn ideal_height(mut self, h: impl IntoPx) -> Self {
        self.ideal_height = Some(h.into_px());
        self
    }

    pub fn max_height(mut self, h: impl IntoPx) -> Self {
        self.max_height = Some(h.into_px());
        self
    }

    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    pub fn is_fixed_width(&self) -> bool {
        matches!(
            (self.min_width, self.max_width),
            (Some(min), Some(max)) if (max - min).abs() < f32::EPSILON && max.is_finite()
        )
    }

    pub fn is_fixed_height(&self) -> bool {
        matches!(
            (self.min_height, self.max_height),
            (Some(min), Some(max)) if (max - min).abs() < f32::EPSILON && max.is_finite()
        )
    }
}

/// Shadow parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    pub color: Color,
    pub offset: Point,
    pub blur_radius: f32,
}

/// Accessibility roles mapped cleanly to AccessKit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AccessibilityRole {
    #[default]
    GenericContainer,
    Window,
    Group,
    Heading,
    Label,
    Button,
    CheckBox,
    Switch,
    Slider,
    ComboBox,
    TextInput,
    MultilineTextInput,
    PasswordInput,
    Image,
    Link,
    List,
    ListItem,
    Table,
    TableRow,
    TableColumnHeader,
    TableCell,
    Tree,
    TreeItem,
    TabList,
    Tab,
    Toolbar,
    Menu,
    MenuBar,
    MenuItem,
    Popover,
    ScrollView,
    Splitter,
    Navigation,
    ProgressIndicator,
}

/// Keyboard modifier flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    };
}

/// Logical/physical key abstraction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Character(String),
    Enter,
    Tab,
    Space,
    Backspace,
    Delete,
    Escape,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
    Named(String),
}

/// Keyboard event passed to `on_key` handlers and focused controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: KeyCode,
    pub physical_code: String,
    pub text: Option<String>,
    pub modifiers: Modifiers,
    pub is_repeat: bool,
    pub is_down: bool,
}

/// Pointer button identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PointerButton {
    #[default]
    Primary,
    Secondary,
    Middle,
}

/// Pointer interaction phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
}

/// Pointer event delivered to hit-tested nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEvent {
    pub position: Point,
    pub button: PointerButton,
    pub phase: PointerPhase,
    pub click_count: u32,
    pub modifiers: Modifiers,
}

/// Drag event delivered to `on_drag` handlers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragEvent {
    pub start: Point,
    pub current: Point,
    pub delta: Point,
    pub phase: PointerPhase,
}

/// Scroll wheel / trackpad event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollEvent {
    pub position: Point,
    pub delta_x: f32,
    pub delta_y: f32,
    pub is_momentum: bool,
}

/// Trackpad pinch gesture event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinchEvent {
    pub position: Point,
    pub scale_delta: f32,
    pub phase: PointerPhase,
}

/// Animation curve definitions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Animation {
    Linear {
        duration_secs: f32,
    },
    EaseIn {
        duration_secs: f32,
    },
    EaseOut {
        duration_secs: f32,
    },
    EaseInOut {
        duration_secs: f32,
    },
    Spring {
        stiffness: f32,
        damping: f32,
        mass: f32,
    },
}

impl Animation {
    pub fn linear(duration_secs: f32) -> Self {
        Self::Linear { duration_secs }
    }

    pub fn ease_in(duration_secs: f32) -> Self {
        Self::EaseIn { duration_secs }
    }

    pub fn ease_out(duration_secs: f32) -> Self {
        Self::EaseOut { duration_secs }
    }

    pub fn ease_in_out(duration_secs: f32) -> Self {
        Self::EaseInOut { duration_secs }
    }

    pub fn spring() -> Self {
        Self::Spring {
            stiffness: 180.0,
            damping: 20.0,
            mass: 1.0,
        }
    }

    pub fn spring_with(stiffness: f32, damping: f32, mass: f32) -> Self {
        Self::Spring {
            stiffness,
            damping,
            mass,
        }
    }

    /// Evaluates progress `[0.0, 1.0]` at `elapsed_secs`. Returns `(progress, is_finished)`.
    pub fn evaluate(&self, elapsed_secs: f32) -> (f32, bool) {
        match *self {
            Self::Linear { duration_secs } => {
                if duration_secs <= 0.0 || elapsed_secs >= duration_secs {
                    (1.0, true)
                } else {
                    ((elapsed_secs / duration_secs).clamp(0.0, 1.0), false)
                }
            }
            Self::EaseIn { duration_secs } => {
                if duration_secs <= 0.0 || elapsed_secs >= duration_secs {
                    (1.0, true)
                } else {
                    let t = (elapsed_secs / duration_secs).clamp(0.0, 1.0);
                    (t * t, false)
                }
            }
            Self::EaseOut { duration_secs } => {
                if duration_secs <= 0.0 || elapsed_secs >= duration_secs {
                    (1.0, true)
                } else {
                    let t = (elapsed_secs / duration_secs).clamp(0.0, 1.0);
                    (1.0 - (1.0 - t) * (1.0 - t), false)
                }
            }
            Self::EaseInOut { duration_secs } => {
                if duration_secs <= 0.0 || elapsed_secs >= duration_secs {
                    (1.0, true)
                } else {
                    let t = (elapsed_secs / duration_secs).clamp(0.0, 1.0);
                    let eased = if t < 0.5 {
                        2.0 * t * t
                    } else {
                        1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                    };
                    (eased, false)
                }
            }
            Self::Spring {
                stiffness,
                damping,
                mass,
            } => {
                let m = mass.max(0.001);
                let omega0 = (stiffness / m).sqrt();
                let zeta = damping / (2.0 * (stiffness * m).sqrt());
                let t = elapsed_secs.max(0.0);
                let val = if zeta < 1.0 {
                    let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
                    let decay = (-zeta * omega0 * t).exp();
                    1.0 - decay
                        * ((zeta * omega0 / omega_d) * (omega_d * t).sin() + (omega_d * t).cos())
                } else {
                    let decay = (-omega0 * t).exp();
                    1.0 - decay * (1.0 + omega0 * t)
                };
                let finished = t > 2.5 || ((1.0 - val).abs() < 0.002 && t > 0.35);
                if finished { (1.0, true) } else { (val, false) }
            }
        }
    }
}

/// Edge for slide/move insertion and removal transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edge {
    Top,
    Bottom,
    Leading,
    Trailing,
}

/// View insertion and removal transitions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transition {
    Opacity,
    Slide,
    Scale,
    Move(Edge),
}
