//! Theme, typography scale (`Font`), semantic color resolution, and widget style protocols.

use std::rc::Rc;

use rustui_core::{Color, CornerRadii, Edges, FontSpec, FontWeight, SemanticColor};

/// Typography helpers matching SwiftUI-style font presets (`Font::title()`).
#[derive(Debug, Clone, PartialEq)]
pub struct Font;

impl Font {
    pub fn large_title() -> FontSpec {
        FontSpec {
            size: 28.0,
            weight: FontWeight::Bold,
            ..Default::default()
        }
    }

    pub fn title() -> FontSpec {
        FontSpec {
            size: 22.0,
            weight: FontWeight::Bold,
            ..Default::default()
        }
    }

    pub fn headline() -> FontSpec {
        FontSpec {
            size: 16.0,
            weight: FontWeight::SemiBold,
            ..Default::default()
        }
    }

    pub fn body() -> FontSpec {
        FontSpec {
            size: 14.0,
            weight: FontWeight::Regular,
            ..Default::default()
        }
    }

    pub fn caption() -> FontSpec {
        FontSpec {
            size: 12.0,
            weight: FontWeight::Regular,
            ..Default::default()
        }
    }

    pub fn monospace() -> FontSpec {
        FontSpec {
            size: 13.0,
            weight: FontWeight::Regular,
            monospace: true,
            ..Default::default()
        }
    }
}

/// Complete color palette for light or dark themes.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeColors {
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

/// Environment theme value governing semantic colors, spacing, and corner radii.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub is_dark: bool,
    pub colors: ThemeColors,
    pub spacing_sm: f32,
    pub spacing_md: f32,
    pub spacing_lg: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self::light()
    }
}

impl Theme {
    pub fn light() -> Self {
        Self {
            is_dark: false,
            colors: ThemeColors {
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
            },
            spacing_sm: 6.0,
            spacing_md: 12.0,
            spacing_lg: 20.0,
            radius_sm: 4.0,
            radius_md: 8.0,
            radius_lg: 12.0,
        }
    }

    pub fn dark() -> Self {
        Self {
            is_dark: true,
            colors: ThemeColors {
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
            },
            spacing_sm: 6.0,
            spacing_md: 12.0,
            spacing_lg: 20.0,
            radius_sm: 4.0,
            radius_md: 8.0,
            radius_lg: 12.0,
        }
    }

    pub fn resolve(&self, color: Color) -> Color {
        match color.semantic {
            None => color,
            Some(SemanticColor::Primary) => self.colors.primary.with_alpha(color.a),
            Some(SemanticColor::Secondary) => self.colors.secondary.with_alpha(color.a),
            Some(SemanticColor::Background) => self.colors.background.with_alpha(color.a),
            Some(SemanticColor::Surface) => self.colors.surface.with_alpha(color.a),
            Some(SemanticColor::SurfaceElevated) => {
                self.colors.surface_elevated.with_alpha(color.a)
            }
            Some(SemanticColor::Accent) => self.colors.accent.with_alpha(color.a),
            Some(SemanticColor::Border) => self.colors.border.with_alpha(color.a),
            Some(SemanticColor::Destructive) => self.colors.destructive.with_alpha(color.a),
            Some(SemanticColor::Success) => self.colors.success.with_alpha(color.a),
            Some(SemanticColor::Warning) => self.colors.warning.with_alpha(color.a),
        }
    }
}

/// Visual styling descriptor for [`Button`] subtrees (`ButtonStyle` protocol).
#[derive(Clone)]
pub struct ButtonStyle {
    pub background: Color,
    pub hover_background: Color,
    pub pressed_background: Color,
    pub foreground: Color,
    pub border: Option<(f32, Color)>,
    pub corner_radius: f32,
    pub padding: Edges,
    pub custom_decorator: Option<Rc<dyn Fn(bool, bool, &Theme) -> (Color, Color)>>,
}

impl std::fmt::Debug for ButtonStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ButtonStyle")
            .field("background", &self.background)
            .field("foreground", &self.foreground)
            .field("corner_radius", &self.corner_radius)
            .finish()
    }
}

impl ButtonStyle {
    pub fn primary(theme: &Theme) -> Self {
        Self {
            background: theme.colors.accent,
            hover_background: theme.colors.accent.lerp(Color::WHITE, 0.12),
            pressed_background: theme.colors.accent.lerp(Color::BLACK, 0.15),
            foreground: theme.colors.on_accent,
            border: None,
            corner_radius: theme.radius_md,
            padding: Edges::symmetric(8.0, 14.0),
            custom_decorator: None,
        }
    }

    pub fn secondary(theme: &Theme) -> Self {
        Self {
            background: theme.colors.surface,
            hover_background: theme.colors.surface_elevated,
            pressed_background: theme.colors.border,
            foreground: theme.colors.primary,
            border: Some((1.0, theme.colors.border)),
            corner_radius: theme.radius_md,
            padding: Edges::symmetric(8.0, 14.0),
            custom_decorator: None,
        }
    }

    pub fn plain(theme: &Theme) -> Self {
        Self {
            background: Color::TRANSPARENT,
            hover_background: theme.colors.surface,
            pressed_background: theme.colors.border,
            foreground: theme.colors.accent,
            border: None,
            corner_radius: theme.radius_sm,
            padding: Edges::symmetric(6.0, 10.0),
            custom_decorator: None,
        }
    }
}

/// Visual styling descriptor for [`Toggle`] subtrees (`ToggleStyle` protocol).
#[derive(Debug, Clone, PartialEq)]
pub struct ToggleStyle {
    pub on_track: Color,
    pub off_track: Color,
    pub knob_color: Color,
    pub label_color: Color,
}

impl ToggleStyle {
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            on_track: theme.colors.accent,
            off_track: theme.colors.border,
            knob_color: Color::WHITE,
            label_color: theme.colors.primary,
        }
    }
}

/// Visual styling descriptor for [`List`] subtrees (`ListStyle` protocol).
#[derive(Debug, Clone, PartialEq)]
pub struct ListStyle {
    pub row_background: Color,
    pub alternate_row_background: Option<Color>,
    pub selected_background: Color,
    pub corner_radii: CornerRadii,
    pub row_height: f32,
}

impl ListStyle {
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            row_background: theme.colors.surface_elevated,
            alternate_row_background: Some(theme.colors.surface),
            selected_background: theme.colors.accent.with_alpha(0.18),
            corner_radii: CornerRadii::all(theme.radius_sm),
            row_height: 36.0,
        }
    }
}
