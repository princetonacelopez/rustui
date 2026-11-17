//! Text shaping, rich text, rope-backed editing model, and IME state for RustUI.

pub mod editor;
pub mod ime;
pub mod shaping;

pub use editor::{EditableText, Selection, TextBuffer};
pub use ime::ImeState;
pub use shaping::{
    ParleyTextMeasurer, ParleyTextPainter, RichText, ShapedLineMetrics, ShapedText, TextSpan,
    TextSystem,
};

#[cfg(test)]
mod tests {
    use rustui_core::{Color, FontSpec, FontWeight, Rect, TextNodeData};

    use super::*;

    #[test]
    fn parley_shapes_mixed_rich_text_emoji_cjk_and_bidi() {
        let mut ts = TextSystem::new();
        let data = TextNodeData {
            text: "Hello 日本語 שלום 🚀 world!".into(),
            font: FontSpec {
                size: 16.0,
                ..Default::default()
            },
            color: Some(Color::BLACK),
            ..Default::default()
        };

        let unwrapped = ts.shape_node_data(&data, None, 1.0);
        let wrapped = ts.shape_node_data(&data, Some(90.0), 1.0);

        assert!(unwrapped.size.width > 0.0);
        assert!(unwrapped.size.height > 0.0);
        assert!(unwrapped.has_rtl);
        assert!(
            wrapped.lines.len() >= unwrapped.lines.len(),
            "wrapping at narrow width should produce at least as many lines"
        );

        let mut list = rustui_render::DisplayList::new(1.0);
        let mut painter = ParleyTextPainter {
            system: &mut ts,
            scale_factor: 1.0,
        };
        rustui_render::TextPainter::paint_text(
            &mut painter,
            &data,
            Rect::new(0.0, 0.0, 200.0, 80.0),
            &mut list,
        );
        // Should emit glyph runs without panicking even on mixed scripts + emoji
        assert!(!list.commands.is_empty() || unwrapped.size.width > 0.0);
        let mut atlas = rustui_render::GlyphAtlas::default();
        let mut glyph_count = 0usize;
        for cmd in &list.commands {
            if let rustui_render::DisplayCommand::DrawGlyphRun { glyphs, .. } = cmd {
                for g in glyphs {
                    glyph_count += 1;
                    let blob = list.fonts.get(&g.key.font_id);
                    atlas.get_or_rasterize(g.key, blob);
                }
            }
        }
        assert!(glyph_count > 0);
    }

    #[test]
    fn rich_text_spans_and_hit_testing() {
        let rich = RichText::new()
            .push(TextSpan::new("Rust").bold().color(Color::ORANGE))
            .push(TextSpan::new("UI").italic().underline());

        let data = TextNodeData {
            text: rich.to_plain_text(),
            spans: rich
                .spans
                .into_iter()
                .map(|s| rustui_core::TextSpanData {
                    text: s.text,
                    font: s.font,
                    color: s.color,
                    underline: s.underline,
                    strikethrough: s.strikethrough,
                })
                .collect(),
            font: FontSpec {
                size: 18.0,
                weight: FontWeight::Regular,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut ts = TextSystem::new();
        let shaped = ts.shape_node_data(&data, Some(300.0), 1.0);
        assert_eq!(shaped.source_text, "RustUI");
        let idx_start = shaped.hit_test_point(0.0, 4.0);
        assert_eq!(idx_start, 0);
        let caret = shaped.caret_rect_for_offset(4, 18.0);
        assert!(caret.height > 0.0);
    }

    #[test]
    fn rope_text_buffer_graphemes_selection_and_undo_redo() {
        let mut buf = TextBuffer::new("");
        buf.insert("H");
        buf.insert("i");
        assert_eq!(buf.text(), "Hi");

        // Coalesced typing undo removes the word "Hi" together.
        assert!(buf.undo());
        assert_eq!(buf.text(), "");
        assert!(buf.redo());
        assert_eq!(buf.text(), "Hi");

        // Grapheme-aware movement with combining mark & emoji.
        buf.insert(" 👨‍👩‍👧‍👦 café");
        let end = buf.len_bytes();
        buf.set_selection(Selection::collapsed(end));
        buf.move_left(false);
        buf.delete_forward();
        assert!(buf.text().ends_with("caf"));

        buf.set_text("hello world\nsecond line");
        buf.select_word_at(2);
        assert_eq!(buf.selected_text(), "hello");
        buf.select_line_at(14);
        assert_eq!(buf.selected_text(), "second line");
    }

    #[test]
    fn ime_preedit_and_commit_for_japanese_chinese_and_emoji() {
        let mut buf = TextBuffer::new("Start: ");
        let mut ime = ImeState::new();
        let caret = Rect::new(42.0, 12.0, 1.5, 18.0);

        // Japanese pre-edit & commit
        ime.set_preedit("にほんご".into(), Some((0, 12)), caret);
        assert!(ime.is_composing());
        assert_eq!(ime.candidate_window_rect, caret);
        assert_eq!(
            ime.compose_display_string(&buf.text(), buf.selection().active),
            "Start: にほんご"
        );
        ime.commit("日本語", &mut buf);
        assert!(!ime.is_composing());
        assert_eq!(buf.text(), "Start: 日本語");

        // Chinese & Emoji commit
        ime.set_preedit("zhongwen".into(), Some((8, 8)), caret);
        ime.commit("中文🎉", &mut buf);
        assert_eq!(buf.text(), "Start: 日本語中文🎉");
    }
}
