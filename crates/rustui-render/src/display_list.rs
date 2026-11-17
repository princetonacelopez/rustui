//! Narrow display-list interface separating layout/widgets from the GPU backend.

use std::collections::HashMap;
use std::sync::Arc;

use rustui_core::{Color, CornerRadii, Edges, Point, Rect};
use rustui_layout::snap_to_pixel;

/// Identifier for a loaded or retained image resource.
pub type ImageId = u64;

/// Layer blend mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
}

/// Gradient color stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}

/// Gradient definition.
#[derive(Debug, Clone, PartialEq)]
pub enum GradientKind {
    Linear {
        start: Point,
        end: Point,
        stops: Vec<GradientStop>,
    },
    Radial {
        center: Point,
        radius: f32,
        stops: Vec<GradientStop>,
    },
}

/// Vector path segment commands executed via Vello.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathOp {
    MoveTo(Point),
    LineTo(Point),
    QuadTo { ctrl: Point, to: Point },
    CubicTo { ctrl1: Point, ctrl2: Point, to: Point },
    Close,
}

/// Horizontal subpixel positioning bin (quarter-pixel precision).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum SubpixelBin {
    #[default]
    Zero = 0,
    Quarter = 1,
    Half = 2,
    ThreeQuarters = 3,
}

impl SubpixelBin {
    pub fn from_offset(x: f32) -> (i32, Self) {
        let floored = x.floor();
        let frac = x - floored;
        let q = ((frac * 4.0).round() as i32).rem_euclid(4);
        let base = if (frac * 4.0).round() as i32 == 4 {
            floored as i32 + 1
        } else {
            floored as i32
        };
        let bin = match q {
            0 => Self::Zero,
            1 => Self::Quarter,
            2 => Self::Half,
            _ => Self::ThreeQuarters,
        };
        (base, bin)
    }

    pub fn offset(self) -> f32 {
        match self {
            Self::Zero => 0.0,
            Self::Quarter => 0.25,
            Self::Half => 0.5,
            Self::ThreeQuarters => 0.75,
        }
    }
}

/// Raw font blob registered with a display list for glyph rasterization.
#[derive(Debug, Clone)]
pub struct FontBlob {
    pub id: u64,
    pub bytes: Arc<Vec<u8>>,
    pub index: u32,
}

/// Cache key for a rasterized glyph in the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub font_id: u64,
    pub glyph_id: u16,
    /// Font size in 1/16th px fixed point.
    pub size_q4: u32,
    pub subpixel_x: SubpixelBin,
}

/// Single positioned glyph in a shaped glyph run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphRunItem {
    pub key: GlyphKey,
    pub position: Point,
    pub color: Color,
}

/// Narrow display-list command emitted by UI nodes and [`Canvas`] callbacks.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayCommand {
    FillRoundedRect {
        rect: Rect,
        radii: CornerRadii,
        color: Color,
    },
    StrokeRoundedRect {
        rect: Rect,
        radii: CornerRadii,
        width: f32,
        color: Color,
    },
    DrawPath {
        ops: Vec<PathOp>,
        fill: Option<Color>,
        stroke: Option<(f32, Color)>,
    },
    DrawImage {
        rect: Rect,
        image_id: ImageId,
        uv: Rect,
        tint: Color,
    },
    DrawNineSlice {
        rect: Rect,
        image_id: ImageId,
        slice: Edges,
        tint: Color,
    },
    DrawGlyphRun {
        glyphs: Vec<GlyphRunItem>,
        color: Color,
    },
    DrawBoxShadow {
        rect: Rect,
        corner_radius: f32,
        offset: Point,
        blur_radius: f32,
        color: Color,
    },
    DrawBlurRect {
        rect: Rect,
        corner_radius: f32,
        blur_radius: f32,
        tint: Color,
    },
    DrawGradient {
        rect: Rect,
        radii: CornerRadii,
        gradient: GradientKind,
    },
    PushClipRect(Rect),
    PushClipRoundedRect {
        rect: Rect,
        radii: CornerRadii,
    },
    PopClip,
    PushLayer {
        bounds: Rect,
        opacity: f32,
        blend_mode: BlendMode,
    },
    PopLayer,
    DrawCustomShader {
        rect: Rect,
        shader_id: u64,
        wgsl: String,
        uniforms: Vec<u8>,
    },
}

/// Pipeline + texture batching key used to minimize GPU draw calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BatchKey {
    SdfQuad,
    GlyphAtlas,
    Image(ImageId),
    VectorPath,
    CustomShader(u64),
    StateChange,
}

/// Retained or per-frame display list consumed by the renderer backend.
#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    pub commands: Vec<DisplayCommand>,
    pub fonts: HashMap<u64, FontBlob>,
    pub scale_factor: f32,
}

impl DisplayList {
    pub fn new(scale_factor: f32) -> Self {
        Self {
            commands: Vec::new(),
            fonts: HashMap::new(),
            scale_factor: if scale_factor > 0.0 {
                scale_factor
            } else {
                1.0
            },
        }
    }

    pub fn clear(&mut self) {
        self.commands.clear();
    }

    pub fn register_font(&mut self, blob: FontBlob) {
        self.fonts.entry(blob.id).or_insert(blob);
    }

    #[inline]
    fn snap(&self, rect: Rect) -> Rect {
        snap_to_pixel(rect, self.scale_factor)
    }

    pub fn fill_rounded_rect(&mut self, rect: Rect, radii: CornerRadii, color: Color) {
        if color.a <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        self.commands.push(DisplayCommand::FillRoundedRect {
            rect: self.snap(rect),
            radii,
            color,
        });
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.fill_rounded_rect(rect, CornerRadii::ZERO, color);
    }

    pub fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        radii: CornerRadii,
        width: f32,
        color: Color,
    ) {
        if color.a <= 0.0 || width <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        self.commands.push(DisplayCommand::StrokeRoundedRect {
            rect: self.snap(rect),
            radii,
            width,
            color,
        });
    }

    pub fn draw_path(
        &mut self,
        ops: Vec<PathOp>,
        fill: Option<Color>,
        stroke: Option<(f32, Color)>,
    ) {
        if ops.is_empty() {
            return;
        }
        self.commands
            .push(DisplayCommand::DrawPath { ops, fill, stroke });
    }

    pub fn draw_image(&mut self, rect: Rect, image_id: ImageId, uv: Rect, tint: Color) {
        self.commands.push(DisplayCommand::DrawImage {
            rect: self.snap(rect),
            image_id,
            uv,
            tint,
        });
    }

    pub fn draw_nine_slice(
        &mut self,
        rect: Rect,
        image_id: ImageId,
        slice: Edges,
        tint: Color,
    ) {
        self.commands.push(DisplayCommand::DrawNineSlice {
            rect: self.snap(rect),
            image_id,
            slice,
            tint,
        });
    }

    pub fn draw_glyph_run(&mut self, glyphs: Vec<GlyphRunItem>, color: Color) {
        if glyphs.is_empty() {
            return;
        }
        self.commands
            .push(DisplayCommand::DrawGlyphRun { glyphs, color });
    }

    pub fn draw_box_shadow(
        &mut self,
        rect: Rect,
        corner_radius: f32,
        offset: Point,
        blur_radius: f32,
        color: Color,
    ) {
        if color.a <= 0.0 {
            return;
        }
        self.commands.push(DisplayCommand::DrawBoxShadow {
            rect: self.snap(rect),
            corner_radius,
            offset,
            blur_radius,
            color,
        });
    }

    pub fn draw_blur_rect(
        &mut self,
        rect: Rect,
        corner_radius: f32,
        blur_radius: f32,
        tint: Color,
    ) {
        self.commands.push(DisplayCommand::DrawBlurRect {
            rect: self.snap(rect),
            corner_radius,
            blur_radius,
            tint,
        });
    }

    pub fn draw_gradient(&mut self, rect: Rect, radii: CornerRadii, gradient: GradientKind) {
        self.commands.push(DisplayCommand::DrawGradient {
            rect: self.snap(rect),
            radii,
            gradient,
        });
    }

    pub fn push_clip_rect(&mut self, rect: Rect) {
        self.commands.push(DisplayCommand::PushClipRect(self.snap(rect)));
    }

    pub fn push_clip_rounded_rect(&mut self, rect: Rect, radii: CornerRadii) {
        self.commands.push(DisplayCommand::PushClipRoundedRect {
            rect: self.snap(rect),
            radii,
        });
    }

    pub fn pop_clip(&mut self) {
        self.commands.push(DisplayCommand::PopClip);
    }

    pub fn push_layer(&mut self, bounds: Rect, opacity: f32, blend_mode: BlendMode) {
        self.commands.push(DisplayCommand::PushLayer {
            bounds: self.snap(bounds),
            opacity,
            blend_mode,
        });
    }

    pub fn pop_layer(&mut self) {
        self.commands.push(DisplayCommand::PopLayer);
    }

    pub fn draw_custom_shader(
        &mut self,
        rect: Rect,
        shader_id: u64,
        wgsl: String,
        uniforms: Vec<u8>,
    ) {
        self.commands.push(DisplayCommand::DrawCustomShader {
            rect: self.snap(rect),
            shader_id,
            wgsl,
            uniforms,
        });
    }

    /// Computes how many coalesced draw batches these commands compile into.
    pub fn batch_count(&self) -> usize {
        let mut batches = 0usize;
        let mut last: Option<BatchKey> = None;
        for cmd in &self.commands {
            let key = match cmd {
                DisplayCommand::FillRoundedRect { .. }
                | DisplayCommand::StrokeRoundedRect { .. }
                | DisplayCommand::DrawBoxShadow { .. }
                | DisplayCommand::DrawBlurRect { .. }
                | DisplayCommand::DrawGradient { .. } => BatchKey::SdfQuad,
                DisplayCommand::DrawGlyphRun { .. } => BatchKey::GlyphAtlas,
                DisplayCommand::DrawImage { image_id, .. }
                | DisplayCommand::DrawNineSlice { image_id, .. } => BatchKey::Image(*image_id),
                DisplayCommand::DrawPath { .. } => BatchKey::VectorPath,
                DisplayCommand::DrawCustomShader { shader_id, .. } => {
                    BatchKey::CustomShader(*shader_id)
                }
                DisplayCommand::PushClipRect(_)
                | DisplayCommand::PushClipRoundedRect { .. }
                | DisplayCommand::PopClip
                | DisplayCommand::PushLayer { .. }
                | DisplayCommand::PopLayer => BatchKey::StateChange,
            };
            if key != BatchKey::StateChange && last != Some(key) {
                batches += 1;
                last = Some(key);
            } else if key == BatchKey::StateChange {
                last = None;
            }
        }
        batches
    }
}
