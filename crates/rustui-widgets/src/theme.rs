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
    pub fn to_palette(&self) -> rustui_core::ThemePalette {
        rustui_core::ThemePalette {
            is_dark: self.is_dark,
            primary: self.colors.primary,
            secondary: self.colors.secondary,
            background: self.colors.background,
            surface: self.colors.surface,
            surface_elevated: self.colors.surface_elevated,
            accent: self.colors.accent,
            on_accent: self.colors.on_accent,
            border: self.colors.border,
            destructive: self.colors.destructive,
            success: self.colors.success,
            warning: self.colors.warning,
        }
    }

    pub(crate) fn uses_default_palette(&self) -> bool {
        self.colors == Self::light().colors || self.colors == Self::dark().colors
    }

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

    pub fn high_contrast() -> Self {
        Self {
            is_dark: true,
            colors: ThemeColors {
                primary: Color::WHITE,
                secondary: Color::from_hex(0xe2e8f0),
                background: Color::BLACK,
                surface: Color::from_hex(0x090d16),
                surface_elevated: Color::from_hex(0x161f30),
                accent: Color::from_hex(0x60a5fa),
                on_accent: Color::BLACK,
                border: Color::WHITE,
                destructive: Color::from_hex(0xff6b6b),
                success: Color::from_hex(0x4ade80),
                warning: Color::from_hex(0xfacc15),
            },
            spacing_sm: 6.0,
            spacing_md: 12.0,
            spacing_lg: 20.0,
            radius_sm: 4.0,
            radius_md: 8.0,
            radius_lg: 12.0,
        }
    }

    /// Serializes this [`Theme`] as a TOML theme document.
    pub fn to_toml_string(&self) -> String {
        let hex = |c: Color| -> String {
            let [r, g, b, _] = c.to_rgba8();
            format!("#{r:02x}{g:02x}{b:02x}")
        };
        format!(
            "is_dark = {}\nspacing_sm = {:.1}\nspacing_md = {:.1}\nspacing_lg = {:.1}\nradius_sm = {:.1}\nradius_md = {:.1}\nradius_lg = {:.1}\n\n[colors]\nprimary = \"{}\"\nsecondary = \"{}\"\nbackground = \"{}\"\nsurface = \"{}\"\nsurface_elevated = \"{}\"\naccent = \"{}\"\non_accent = \"{}\"\nborder = \"{}\"\ndestructive = \"{}\"\nsuccess = \"{}\"\nwarning = \"{}\"\n",
            self.is_dark,
            self.spacing_sm,
            self.spacing_md,
            self.spacing_lg,
            self.radius_sm,
            self.radius_md,
            self.radius_lg,
            hex(self.colors.primary),
            hex(self.colors.secondary),
            hex(self.colors.background),
            hex(self.colors.surface),
            hex(self.colors.surface_elevated),
            hex(self.colors.accent),
            hex(self.colors.on_accent),
            hex(self.colors.border),
            hex(self.colors.destructive),
            hex(self.colors.success),
            hex(self.colors.warning),
        )
    }

    /// Parses a [`Theme`] from a TOML string (`[colors]` with `#rrggbb` values).
    pub fn from_toml_str(src: &str) -> Result<Self, String> {
        let mut theme = Self::light();
        let parse_hex = |s: &str| -> Result<Color, String> {
            let trimmed = s.trim().trim_matches('"').trim_start_matches('#');
            if trimmed.len() != 6 {
                return Err(format!("expected #rrggbb hex color, got '{s}'"));
            }
            let val =
                u32::from_str_radix(trimmed, 16).map_err(|_| format!("invalid hex color '{s}'"))?;
            Ok(Color::from_hex(val))
        };

        for raw in src.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                let v = v.trim();
                match k {
                    "is_dark" => theme.is_dark = v == "true",
                    "spacing_sm" => {
                        theme.spacing_sm = v.parse::<f32>().map_err(|e| e.to_string())?;
                    }
                    "spacing_md" => {
                        theme.spacing_md = v.parse::<f32>().map_err(|e| e.to_string())?;
                    }
                    "spacing_lg" => {
                        theme.spacing_lg = v.parse::<f32>().map_err(|e| e.to_string())?;
                    }
                    "radius_sm" => {
                        theme.radius_sm = v.parse::<f32>().map_err(|e| e.to_string())?;
                    }
                    "radius_md" => {
                        theme.radius_md = v.parse::<f32>().map_err(|e| e.to_string())?;
                    }
                    "radius_lg" => {
                        theme.radius_lg = v.parse::<f32>().map_err(|e| e.to_string())?;
                    }
                    "primary" => theme.colors.primary = parse_hex(v)?,
                    "secondary" => theme.colors.secondary = parse_hex(v)?,
                    "background" => theme.colors.background = parse_hex(v)?,
                    "surface" => theme.colors.surface = parse_hex(v)?,
                    "surface_elevated" => theme.colors.surface_elevated = parse_hex(v)?,
                    "accent" => theme.colors.accent = parse_hex(v)?,
                    "on_accent" => theme.colors.on_accent = parse_hex(v)?,
                    "border" => theme.colors.border = parse_hex(v)?,
                    "destructive" => theme.colors.destructive = parse_hex(v)?,
                    "success" => theme.colors.success = parse_hex(v)?,
                    "warning" => theme.colors.warning = parse_hex(v)?,
                    _ => {}
                }
            }
        }
        Ok(theme)
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

/// Custom button decorator closure `(is_hovered, is_pressed, &Theme) -> (background, foreground)`.
pub type ButtonDecoratorFn = Rc<dyn Fn(bool, bool, &Theme) -> (Color, Color)>;

/// Visual styling descriptor for [`crate::Button`] subtrees (`ButtonStyle` protocol).
#[derive(Clone)]
pub struct ButtonStyle {
    pub background: Color,
    pub hover_background: Color,
    pub pressed_background: Color,
    pub foreground: Color,
    pub border: Option<(f32, Color)>,
    pub corner_radius: f32,
    pub padding: Edges,
    pub custom_decorator: Option<ButtonDecoratorFn>,
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
        let standard = theme.uses_default_palette();
        Self {
            background: if standard {
                Color::accent()
            } else {
                theme.colors.accent
            },
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
        let standard = theme.uses_default_palette();
        Self {
            background: if standard {
                Color::surface_elevated()
            } else {
                theme.colors.surface_elevated
            },
            hover_background: theme.colors.surface,
            pressed_background: theme.colors.border,
            foreground: if standard {
                Color::primary()
            } else {
                theme.colors.primary
            },
            border: Some((
                1.0,
                if standard {
                    Color::border()
                } else {
                    theme.colors.border
                },
            )),
            corner_radius: theme.radius_md,
            padding: Edges::symmetric(8.0, 14.0),
            custom_decorator: None,
        }
    }

    pub fn destructive(theme: &Theme) -> Self {
        let standard = theme.uses_default_palette();
        Self {
            background: if standard {
                Color::destructive()
            } else {
                theme.colors.destructive
            },
            hover_background: theme.colors.destructive.lerp(Color::WHITE, 0.12),
            pressed_background: theme.colors.destructive.lerp(Color::BLACK, 0.15),
            foreground: theme.colors.on_accent,
            border: None,
            corner_radius: theme.radius_md,
            padding: Edges::symmetric(8.0, 14.0),
            custom_decorator: None,
        }
    }

    pub fn plain(theme: &Theme) -> Self {
        let standard = theme.uses_default_palette();
        Self {
            background: Color::TRANSPARENT,
            hover_background: theme.colors.surface,
            pressed_background: theme.colors.border,
            foreground: if standard {
                Color::accent()
            } else {
                theme.colors.accent
            },
            border: None,
            corner_radius: theme.radius_sm,
            padding: Edges::symmetric(6.0, 10.0),
            custom_decorator: None,
        }
    }
}

/// Visual styling descriptor for [`crate::Toggle`] subtrees (`ToggleStyle` protocol).
#[derive(Debug, Clone, PartialEq)]
pub struct ToggleStyle {
    pub on_track: Color,
    pub off_track: Color,
    pub knob_color: Color,
    pub label_color: Color,
}

impl ToggleStyle {
    pub fn from_theme(theme: &Theme) -> Self {
        let standard = theme.uses_default_palette();
        Self {
            on_track: if standard {
                Color::accent()
            } else {
                theme.colors.accent
            },
            off_track: if standard {
                Color::border()
            } else {
                theme.colors.border
            },
            knob_color: Color::WHITE,
            label_color: if standard {
                Color::primary()
            } else {
                theme.colors.primary
            },
        }
    }
}

/// Visual styling descriptor for [`crate::List`] subtrees (`ListStyle` protocol).
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
        let standard = theme.uses_default_palette();
        Self {
            row_background: if standard {
                Color::surface_elevated()
            } else {
                theme.colors.surface_elevated
            },
            alternate_row_background: Some(if standard {
                Color::surface()
            } else {
                theme.colors.surface
            }),
            selected_background: if standard {
                Color::accent().with_alpha(0.18)
            } else {
                theme.colors.accent.with_alpha(0.18)
            },
            corner_radii: CornerRadii::all(theme.radius_sm),
            row_height: 36.0,
        }
    }
}
