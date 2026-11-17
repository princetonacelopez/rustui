//! Shelf-bin-packed glyph texture atlas with subpixel positioning, color emoji, and optional LCD AA.

use std::collections::HashMap;

use rustui_core::Rect;
use swash::FontRef;
use swash::scale::image::Content as GlyphContent;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Vector};

use crate::display_list::{FontBlob, GlyphKey};

/// Placement entry inside the [`GlyphAtlas`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasGlyphEntry {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub bearing_x: i32,
    pub bearing_y: i32,
    pub is_color: bool,
    pub uv: Rect,
}

#[derive(Debug, Clone, Copy)]
struct Shelf {
    y: u32,
    height: u32,
    cursor_x: u32,
}

/// CPU + GPU-uploadable RGBA8 glyph atlas supporting subpixel bins and color emoji.
pub struct GlyphAtlas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    shelves: Vec<Shelf>,
    entries: HashMap<GlyphKey, AtlasGlyphEntry>,
    scale_cx: ScaleContext,
    pub lcd_aa: bool,
    pub dirty: bool,
}

impl std::fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphAtlas")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("entries", &self.entries.len())
            .field("lcd_aa", &self.lcd_aa)
            .finish()
    }
}

impl Default for GlyphAtlas {
    fn default() -> Self {
        Self::new(1024, 1024)
    }
}

impl GlyphAtlas {
    pub fn new(width: u32, height: u32) -> Self {
        let w = width.max(64);
        let h = height.max(64);
        Self {
            width: w,
            height: h,
            pixels: vec![0u8; (w * h * 4) as usize],
            shelves: Vec::new(),
            entries: HashMap::new(),
            scale_cx: ScaleContext::new(),
            lcd_aa: false,
            dirty: true,
        }
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, key: &GlyphKey) -> Option<&AtlasGlyphEntry> {
        self.entries.get(key)
    }

    /// Rasterizes (if missing) and returns the [`AtlasGlyphEntry`] for `key`.
    pub fn get_or_rasterize(
        &mut self,
        key: GlyphKey,
        font_blob: Option<&FontBlob>,
    ) -> Option<AtlasGlyphEntry> {
        if let Some(entry) = self.entries.get(&key) {
            return Some(*entry);
        }
        let blob = font_blob?;
        let font = FontRef::from_index(&blob.bytes, blob.index as usize)?;
        let size_px = (key.size_q4 as f32) / 16.0;
        let subpixel_dx = key.subpixel_x.offset();

        let mut scaler = self
            .scale_cx
            .builder(font)
            .size(size_px.max(1.0))
            .hint(true)
            .build();

        let format = if self.lcd_aa {
            Format::Subpixel
        } else {
            Format::Alpha
        };

        let image = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ])
        .format(format)
        .offset(Vector::new(subpixel_dx, 0.0))
        .render(&mut scaler, key.glyph_id);

        let Some(img) = image else {
            let empty = AtlasGlyphEntry {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                bearing_x: 0,
                bearing_y: 0,
                is_color: false,
                uv: Rect::ZERO,
            };
            self.entries.insert(key, empty);
            return Some(empty);
        };

        let gw = img.placement.width;
        let gh = img.placement.height;
        if gw == 0 || gh == 0 {
            let empty = AtlasGlyphEntry {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                bearing_x: img.placement.left,
                bearing_y: img.placement.top,
                is_color: false,
                uv: Rect::ZERO,
            };
            self.entries.insert(key, empty);
            return Some(empty);
        }

        let (ax, ay) = self.allocate_rect(gw, gh)?;
        let is_color = matches!(img.content, GlyphContent::Color);

        for row in 0..gh {
            for col in 0..gw {
                let dst = (((ay + row) * self.width + (ax + col)) * 4) as usize;
                match img.content {
                    GlyphContent::Mask => {
                        let a = img.data[(row * gw + col) as usize];
                        self.pixels[dst] = 255;
                        self.pixels[dst + 1] = 255;
                        self.pixels[dst + 2] = 255;
                        self.pixels[dst + 3] = a;
                    }
                    GlyphContent::SubpixelMask => {
                        let src = ((row * gw + col) * 4) as usize;
                        self.pixels[dst] = img.data[src];
                        self.pixels[dst + 1] = img.data[src + 1];
                        self.pixels[dst + 2] = img.data[src + 2];
                        let avg = ((u16::from(img.data[src])
                            + u16::from(img.data[src + 1])
                            + u16::from(img.data[src + 2]))
                            / 3) as u8;
                        self.pixels[dst + 3] = avg;
                    }
                    GlyphContent::Color => {
                        let src = ((row * gw + col) * 4) as usize;
                        self.pixels[dst..dst + 4].copy_from_slice(&img.data[src..src + 4]);
                    }
                }
            }
        }

        let entry = AtlasGlyphEntry {
            x: ax,
            y: ay,
            width: gw,
            height: gh,
            bearing_x: img.placement.left,
            bearing_y: img.placement.top,
            is_color,
            uv: Rect::new(
                ax as f32 / self.width as f32,
                ay as f32 / self.height as f32,
                gw as f32 / self.width as f32,
                gh as f32 / self.height as f32,
            ),
        };
        self.entries.insert(key, entry);
        self.dirty = true;
        Some(entry)
    }

    /// Inserts a pre-rasterized RGBA glyph bitmap directly (useful in tests and custom glyph sources).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_raw(
        &mut self,
        key: GlyphKey,
        width: u32,
        height: u32,
        bearing_x: i32,
        bearing_y: i32,
        is_color: bool,
        rgba: &[u8],
    ) -> Option<AtlasGlyphEntry> {
        if rgba.len() < (width * height * 4) as usize {
            return None;
        }
        let (ax, ay) = self.allocate_rect(width, height)?;
        for row in 0..height {
            let src_start = (row * width * 4) as usize;
            let dst_start = (((ay + row) * self.width + ax) * 4) as usize;
            self.pixels[dst_start..dst_start + (width * 4) as usize]
                .copy_from_slice(&rgba[src_start..src_start + (width * 4) as usize]);
        }
        let entry = AtlasGlyphEntry {
            x: ax,
            y: ay,
            width,
            height,
            bearing_x,
            bearing_y,
            is_color,
            uv: Rect::new(
                ax as f32 / self.width as f32,
                ay as f32 / self.height as f32,
                width as f32 / self.width as f32,
                height as f32 / self.height as f32,
            ),
        };
        self.entries.insert(key, entry);
        self.dirty = true;
        Some(entry)
    }

    fn allocate_rect(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let padded_w = w + 1;
        let padded_h = h + 1;

        if let Some((x, y)) = self.try_find_shelf(padded_w, padded_h) {
            return Some((x, y));
        }

        // Grow atlas height by 2x (up to 4096) if full.
        if self.height < 4096 {
            let new_h = (self.height * 2).min(4096);
            self.pixels.resize((self.width * new_h * 4) as usize, 0);
            self.height = new_h;
            for entry in self.entries.values_mut() {
                entry.uv.y = entry.y as f32 / self.height as f32;
                entry.uv.height = entry.height as f32 / self.height as f32;
            }
            return self.try_find_shelf(padded_w, padded_h);
        }
        None
    }

    fn try_find_shelf(&mut self, padded_w: u32, padded_h: u32) -> Option<(u32, u32)> {
        for shelf in &mut self.shelves {
            if padded_h <= shelf.height && shelf.cursor_x + padded_w <= self.width {
                let x = shelf.cursor_x;
                let y = shelf.y;
                shelf.cursor_x += padded_w;
                return Some((x, y));
            }
        }
        let next_y = self.shelves.last().map_or(0, |s| s.y + s.height);
        if next_y + padded_h <= self.height && padded_w <= self.width {
            self.shelves.push(Shelf {
                y: next_y,
                height: padded_h,
                cursor_x: padded_w,
            });
            return Some((0, next_y));
        }
        None
    }
}
