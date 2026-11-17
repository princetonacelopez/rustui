//! Parley text shaping, font fallback/discovery, rich text spans, line breaking, and display-list emission.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use parley::{
    Affinity, Alignment as ParleyAlign, AlignmentOptions, Cursor, FontContext, FontFamily,
    FontFamilyName, FontStyle as ParleyFontStyle, FontWeight as ParleyFontWeight, GenericFamily,
    Layout as ParleyLayout, LayoutContext, PositionedLayoutItem, Selection as ParleySelection,
    StyleProperty,
};
use rustui_core::{
    Color, FontSpec, FontWeight, HorizontalAlignment, Point, Rect, Size, TextNodeData,
    TruncationMode,
};
use rustui_layout::TextMeasurer;
use rustui_render::{DisplayList, FontBlob, GlyphKey, GlyphRunItem, SubpixelBin, TextPainter};
use unicode_bidi::BidiInfo;

/// Per-line metrics exposed to layout and editors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedLineMetrics {
    pub baseline: f32,
    pub line_height: f32,
    pub advance: f32,
    pub offset: f32,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Rich text span for programmatic attributed text construction.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    pub text: String,
    pub font: Option<FontSpec>,
    pub color: Option<Color>,
    pub underline: bool,
    pub strikethrough: bool,
}

impl TextSpan {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            font: None,
            color: None,
            underline: false,
            strikethrough: false,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn bold(mut self) -> Self {
        let mut f = self.font.unwrap_or_default();
        f.weight = FontWeight::Bold;
        self.font = Some(f);
        self
    }

    pub fn italic(mut self) -> Self {
        let mut f = self.font.unwrap_or_default();
        f.italic = true;
        self.font = Some(f);
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        let mut f = self.font.unwrap_or_default();
        f.size = size;
        self.font = Some(f);
        self
    }

    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }
}

/// Builder for rich attributed text paragraphs.
#[derive(Debug, Clone, Default)]
pub struct RichText {
    pub spans: Vec<TextSpan>,
}

impl RichText {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(mut self, span: TextSpan) -> Self {
        self.spans.push(span);
        self
    }

    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        for s in &self.spans {
            out.push_str(&s.text);
        }
        out
    }
}

/// Result of shaping and line-breaking a paragraph via [`TextSystem`].
pub struct ShapedText {
    pub source_text: String,
    pub layout: ParleyLayout<[u8; 4]>,
    pub size: Size<f32>,
    pub lines: Vec<ShapedLineMetrics>,
    pub has_rtl: bool,
    pub truncated: bool,
}

impl std::fmt::Debug for ShapedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShapedText")
            .field("size", &self.size)
            .field("line_count", &self.lines.len())
            .field("has_rtl", &self.has_rtl)
            .field("truncated", &self.truncated)
            .finish()
    }
}

impl ShapedText {
    /// Returns the byte offset inside `source_text` closest to local point `(x, y)`.
    pub fn hit_test_point(&self, x: f32, y: f32) -> usize {
        if self.source_text.is_empty() || y < 0.0 {
            return 0;
        }
        Cursor::from_point(&self.layout, x, y)
            .index()
            .min(self.source_text.len())
    }

    /// Computes a local caret rectangle for `byte_idx`.
    pub fn caret_rect_for_offset(&self, byte_idx: usize, fallback_height: f32) -> Rect {
        let h_min = fallback_height.max(16.0);
        if self.source_text.is_empty() {
            return Rect::new(0.0, 0.0, 2.0, h_min);
        }
        let clamped = byte_idx.min(self.source_text.len());
        let cursor = Cursor::from_byte_index(&self.layout, clamped, Affinity::Downstream);
        let bb = cursor.geometry(&self.layout, 2.0);
        let w = ((bb.x1 - bb.x0) as f32).max(2.0);
        let h = ((bb.y1 - bb.y0) as f32).max(h_min);
        Rect::new(bb.x0 as f32, bb.y0 as f32, w, h)
    }

    /// Computes visual highlight rectangles covering the byte range `[start_byte, end_byte)`.
    pub fn selection_rects_for_range(
        &self,
        start_byte: usize,
        end_byte: usize,
        fallback_height: f32,
    ) -> Vec<Rect> {
        let s = start_byte.min(end_byte).min(self.source_text.len());
        let e = start_byte.max(end_byte).min(self.source_text.len());
        if s == e || self.source_text.is_empty() {
            return Vec::new();
        }
        let sel = ParleySelection::new(
            Cursor::from_byte_index(&self.layout, s, Affinity::Downstream),
            Cursor::from_byte_index(&self.layout, e, Affinity::Downstream),
        );
        sel.geometry(&self.layout)
            .into_iter()
            .map(|(bb, _)| {
                Rect::new(
                    bb.x0 as f32,
                    bb.y0 as f32,
                    ((bb.x1 - bb.x0) as f32).max(4.0),
                    ((bb.y1 - bb.y0) as f32).max(fallback_height),
                )
            })
            .collect()
    }

    /// Emits shaped glyph runs, underlines, strikethroughs, and inline preedit underline into `list`.
    pub fn emit_to_display_list(
        &self,
        origin: Point,
        default_color: Color,
        underline: bool,
        strikethrough: bool,
        preedit_byte_range: Option<(usize, usize)>,
        list: &mut DisplayList,
    ) {
        let mut glyphs = Vec::new();
        for line in self.layout.lines() {
            let lm = line.metrics();
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    let font = run.run().font();
                    let font_id = font.data.id();
                    let bytes = font.data.as_ref();
                    if !list.fonts.contains_key(&font_id) && !bytes.is_empty() {
                        list.register_font(FontBlob {
                            id: font_id,
                            bytes: Arc::new(bytes.to_vec()),
                            index: font.index,
                        });
                    }
                    let brush = run.style().brush;
                    let run_color = Color::from_rgba8(brush[0], brush[1], brush[2], brush[3]);
                    let effective_color = if run_color.a > 0.0 {
                        run_color
                    } else {
                        default_color
                    };
                    let scale = list.scale_factor.max(1.0);
                    let size_q4 = ((run.run().font_size() * scale * 16.0).round() as u32).max(16);

                    for pg in run.positioned_glyphs() {
                        let gx = origin.x + pg.x;
                        let gy = origin.y + pg.y;
                        let (base_phys_x, subpixel_x) = SubpixelBin::from_offset(gx * scale);
                        glyphs.push(GlyphRunItem {
                            key: GlyphKey {
                                font_id,
                                glyph_id: pg.id as u16,
                                size_q4,
                                subpixel_x,
                            },
                            position: Point::new(base_phys_x as f32 / scale, gy),
                            color: effective_color,
                        });
                    }

                    if underline || run.style().underline.is_some() {
                        list.fill_rect(
                            Rect::new(
                                origin.x + run.offset(),
                                origin.y + run.baseline() + 2.0,
                                run.advance().max(1.0),
                                1.25,
                            ),
                            effective_color,
                        );
                    }
                    if strikethrough || run.style().strikethrough.is_some() {
                        list.fill_rect(
                            Rect::new(
                                origin.x + run.offset(),
                                origin.y + run.baseline() - run.run().font_size() * 0.3,
                                run.advance().max(1.0),
                                1.25,
                            ),
                            effective_color,
                        );
                    }
                }
            }
            if let Some((pre_start, pre_end)) = preedit_byte_range
                && pre_end > pre_start
            {
                let start_r = self.caret_rect_for_offset(pre_start, lm.line_height);
                let end_r = self.caret_rect_for_offset(pre_end, lm.line_height);
                let x0 = start_r.x.min(end_r.x);
                let x1 = start_r.x.max(end_r.x).max(x0 + 4.0);
                list.fill_rect(
                    Rect::new(origin.x + x0, origin.y + lm.baseline + 2.0, x1 - x0, 1.5),
                    default_color,
                );
            }
        }

        if !glyphs.is_empty() {
            list.draw_glyph_run(glyphs, default_color);
        }
    }
}

/// Shared Parley shaping + fontique system font discovery engine.
pub struct TextSystem {
    pub font_cx: FontContext,
    pub layout_cx: LayoutContext<[u8; 4]>,
    cache: HashMap<u64, Arc<ShapedText>>,
}

impl std::fmt::Debug for TextSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextSystem")
            .field("cached_layouts", &self.cache.len())
            .finish()
    }
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSystem {
    pub fn new() -> Self {
        Self {
            font_cx: FontContext::new(),
            layout_cx: LayoutContext::new(),
            cache: HashMap::new(),
        }
    }

    /// Number of system font families discovered by fontique (fontconfig/CoreText/DirectWrite).
    pub fn discovered_font_families(&mut self) -> usize {
        self.font_cx.collection.family_names().count()
    }

    /// Shapes and line-breaks `data` up to `max_width` at `scale_factor`.
    pub fn shape_node_data(
        &mut self,
        data: &TextNodeData,
        max_width: Option<f32>,
        scale_factor: f32,
    ) -> Arc<ShapedText> {
        let key = compute_shape_key(data, max_width, scale_factor);
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }

        let shaped = Arc::new(self.shape_uncached(data, max_width, scale_factor));
        if self.cache.len() > 2048 {
            self.cache.clear();
        }
        self.cache.insert(key, shaped.clone());
        shaped
    }

    fn shape_uncached(
        &mut self,
        data: &TextNodeData,
        max_width: Option<f32>,
        scale_factor: f32,
    ) -> ShapedText {
        let (full_text, preedit_range) = if let Some((pre_text, _, _)) = &data.preedit {
            let caret = data.caret.unwrap_or(data.text.len()).min(data.text.len());
            let mut combined = String::with_capacity(data.text.len() + pre_text.len());
            combined.push_str(&data.text[..caret]);
            let start = combined.len();
            combined.push_str(pre_text);
            let end = combined.len();
            combined.push_str(&data.text[caret..]);
            (combined, Some((start, end)))
        } else if !data.spans.is_empty() && data.text.is_empty() {
            let mut combined = String::new();
            for s in &data.spans {
                combined.push_str(&s.text);
            }
            (combined, None)
        } else {
            (data.text.clone(), None)
        };
        let _ = preedit_range;

        let display_text = apply_truncation_hint(
            &full_text,
            data.line_limit,
            data.truncation,
            max_width,
            data.font.size,
        );
        let was_truncated = display_text.len() != full_text.len();

        let bidi = BidiInfo::new(&display_text, None);
        let has_rtl = bidi.has_rtl();

        let default_rgba = data.color.unwrap_or(Color::BLACK).to_rgba8();

        let _ = scale_factor;
        let mut builder =
            self.layout_cx
                .ranged_builder(&mut self.font_cx, &display_text, 1.0, true);

        builder.push_default(StyleProperty::FontSize(data.font.size.max(1.0)));
        builder.push_default(StyleProperty::FontWeight(ParleyFontWeight::new(
            data.font.weight.numeric(),
        )));
        builder.push_default(StyleProperty::FontStyle(if data.font.italic {
            ParleyFontStyle::Italic
        } else {
            ParleyFontStyle::Normal
        }));
        builder.push_default(StyleProperty::Brush(default_rgba));
        builder.push_default(StyleProperty::Underline(data.underline));
        builder.push_default(StyleProperty::Strikethrough(data.strikethrough));

        if let Some(fam) = &data.font.family {
            builder.push_default(StyleProperty::FontFamily(FontFamily::List(
                std::borrow::Cow::Owned(vec![
                    FontFamilyName::Named(std::borrow::Cow::Borrowed(fam.as_str())),
                    FontFamilyName::Generic(GenericFamily::SansSerif),
                    FontFamilyName::Named(std::borrow::Cow::Borrowed("Noto Emoji")),
                    FontFamilyName::Named(std::borrow::Cow::Borrowed("Symbola")),
                    FontFamilyName::Generic(GenericFamily::Emoji),
                ]),
            )));
        } else if data.font.monospace {
            builder.push_default(StyleProperty::FontFamily(FontFamily::List(
                std::borrow::Cow::Borrowed(&[
                    FontFamilyName::Generic(GenericFamily::Monospace),
                    FontFamilyName::Named(std::borrow::Cow::Borrowed("Noto Emoji")),
                    FontFamilyName::Named(std::borrow::Cow::Borrowed("Symbola")),
                    FontFamilyName::Generic(GenericFamily::Emoji),
                ]),
            )));
        } else {
            builder.push_default(StyleProperty::FontFamily(FontFamily::List(
                std::borrow::Cow::Borrowed(&[
                    FontFamilyName::Generic(GenericFamily::SansSerif),
                    FontFamilyName::Named(std::borrow::Cow::Borrowed("Noto Emoji")),
                    FontFamilyName::Named(std::borrow::Cow::Borrowed("Symbola")),
                    FontFamilyName::Generic(GenericFamily::Emoji),
                ]),
            )));
        }

        if !data.spans.is_empty() && data.preedit.is_none() {
            let mut offset = 0usize;
            for span in &data.spans {
                let end = (offset + span.text.len()).min(display_text.len());
                if end > offset {
                    let range = offset..end;
                    if let Some(c) = span.color {
                        builder.push(StyleProperty::Brush(c.to_rgba8()), range.clone());
                    }
                    if let Some(f) = &span.font {
                        builder.push(StyleProperty::FontSize(f.size.max(1.0)), range.clone());
                        builder.push(
                            StyleProperty::FontWeight(ParleyFontWeight::new(f.weight.numeric())),
                            range.clone(),
                        );
                        builder.push(
                            StyleProperty::FontStyle(if f.italic {
                                ParleyFontStyle::Italic
                            } else {
                                ParleyFontStyle::Normal
                            }),
                            range.clone(),
                        );
                    }
                    if span.underline {
                        builder.push(StyleProperty::Underline(true), range.clone());
                    }
                    if span.strikethrough {
                        builder.push(StyleProperty::Strikethrough(true), range);
                    }
                }
                offset = end;
            }
        }

        let mut layout = builder.build(&display_text);
        let parley_align = match data.alignment {
            HorizontalAlignment::Leading => ParleyAlign::Start,
            HorizontalAlignment::Center => ParleyAlign::Center,
            HorizontalAlignment::Trailing => ParleyAlign::End,
        };
        layout.set_alignment(parley_align, AlignmentOptions::default());
        let break_w = max_width.filter(|w| w.is_finite() && *w > 0.0);
        layout.break_all_lines(break_w);

        let mut lines = Vec::new();
        let mut total_h = 0.0f32;
        let mut max_w = 0.0f32;
        let limit = data.line_limit.unwrap_or(usize::MAX);

        for (idx, line) in layout.lines().enumerate() {
            if idx >= limit {
                break;
            }
            let m = line.metrics();
            let tr = line.text_range();
            max_w = max_w.max(m.advance);
            total_h += m.line_height;
            lines.push(ShapedLineMetrics {
                baseline: m.baseline,
                line_height: m.line_height,
                advance: m.advance,
                offset: m.offset,
                start_byte: tr.start,
                end_byte: tr.end,
            });
        }

        if lines.is_empty() {
            total_h = (data.font.size * 1.35).ceil();
        }
        let width = match break_w {
            Some(bw) => layout.width().max(max_w).min(bw),
            None => layout.width().max(max_w),
        };
        let height = if data.line_limit.is_some() {
            total_h
        } else {
            layout.height().max(total_h)
        };

        ShapedText {
            source_text: display_text,
            layout,
            size: Size::new(width.ceil(), height.ceil()),
            lines,
            has_rtl,
            truncated: was_truncated,
        }
    }
}

fn apply_truncation_hint(
    text: &str,
    line_limit: Option<usize>,
    mode: TruncationMode,
    max_width: Option<f32>,
    font_size: f32,
) -> String {
    let Some(limit) = line_limit else {
        return text.to_owned();
    };
    if limit == 1
        && let Some(mw) = max_width.filter(|w| w.is_finite() && *w > 0.0)
    {
        let approx_chars = ((mw / (font_size * 0.55)).floor() as usize).max(1);
        let chars: Vec<char> = text.chars().collect();
        if chars.len() > approx_chars {
            return match mode {
                TruncationMode::Tail => {
                    let keep = approx_chars.saturating_sub(1);
                    let mut s: String = chars[..keep].iter().collect();
                    s.push('…');
                    s
                }
                TruncationMode::Head => {
                    let keep = approx_chars.saturating_sub(1);
                    let mut s = String::from("…");
                    s.extend(&chars[chars.len() - keep..]);
                    s
                }
                TruncationMode::Middle => {
                    let half = approx_chars.saturating_sub(1) / 2;
                    let mut s: String = chars[..half].iter().collect();
                    s.push('…');
                    s.extend(&chars[chars.len() - half..]);
                    s
                }
                TruncationMode::Clip => chars[..approx_chars].iter().collect(),
            };
        }
    }
    text.to_owned()
}

fn compute_shape_key(data: &TextNodeData, max_width: Option<f32>, scale_factor: f32) -> u64 {
    let mut h = DefaultHasher::new();
    data.text.hash(&mut h);
    data.font.family.hash(&mut h);
    ((data.font.size * 16.0) as u32).hash(&mut h);
    data.font.weight.hash(&mut h);
    data.font.italic.hash(&mut h);
    data.font.monospace.hash(&mut h);
    data.line_limit.hash(&mut h);
    (data.truncation as u8).hash(&mut h);
    (data.alignment as u8).hash(&mut h);
    data.preedit.hash(&mut h);
    data.spans.len().hash(&mut h);
    for s in &data.spans {
        s.text.hash(&mut h);
    }
    max_width.map(|w| (w * 16.0) as i32).hash(&mut h);
    ((scale_factor * 16.0) as u32).hash(&mut h);
    h.finish()
}

/// Connects [`TextSystem`] to `rustui-layout`'s [`TextMeasurer`] protocol.
#[derive(Debug)]
pub struct ParleyTextMeasurer<'a> {
    pub system: &'a mut TextSystem,
    pub scale_factor: f32,
}

impl TextMeasurer for ParleyTextMeasurer<'_> {
    fn measure_text(&mut self, data: &TextNodeData, max_width: Option<f32>) -> Size<f32> {
        self.system
            .shape_node_data(data, max_width, self.scale_factor)
            .size
    }
}

/// Connects [`TextSystem`] to `rustui-render`'s [`TextPainter`] display-list emitter.
#[derive(Debug)]
pub struct ParleyTextPainter<'a> {
    pub system: &'a mut TextSystem,
    pub scale_factor: f32,
}

impl TextPainter for ParleyTextPainter<'_> {
    fn paint_text(&mut self, data: &TextNodeData, bounds: Rect, list: &mut DisplayList) {
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return;
        }
        let shaped = self
            .system
            .shape_node_data(data, Some(bounds.width + 1.0), self.scale_factor);
        let default_color = data.color.unwrap_or(Color::BLACK);

        // Draw selection background highlight if present.
        if let Some((sel_s, sel_e)) = data.selection
            && sel_e > sel_s
        {
            for r in shaped.selection_rects_for_range(sel_s, sel_e, data.font.size * 1.3) {
                list.fill_rect(
                    Rect::new(bounds.x + r.x, bounds.y + r.y, r.width, r.height),
                    Color::rgba(0.15, 0.39, 0.92, 0.26),
                );
            }
        }

        let preedit_range = data.preedit.as_ref().map(|(pre_text, _, _)| {
            let caret = data.caret.unwrap_or(data.text.len()).min(data.text.len());
            (caret, caret + pre_text.len())
        });

        shaped.emit_to_display_list(
            bounds.origin(),
            default_color,
            data.underline,
            data.strikethrough,
            preedit_range,
            list,
        );

        // Draw high-contrast I-beam caret with top/bottom serif caps when focused.
        if let Some(caret) = data.caret {
            let caret_rect = shaped.caret_rect_for_offset(caret, data.font.size * 1.25);
            let cx = bounds.x + caret_rect.x;
            let cy = bounds.y + caret_rect.y;
            let ch = caret_rect.height.min(bounds.height.max(16.0));
            let caret_color = Color::from_hex(0x2563eb);
            list.fill_rect(Rect::new(cx, cy, 2.0, ch), caret_color);
            list.fill_rect(Rect::new(cx - 2.0, cy, 6.0, 1.5), caret_color);
            list.fill_rect(Rect::new(cx - 2.0, cy + ch - 1.5, 6.0, 1.5), caret_color);
        }
    }
}
