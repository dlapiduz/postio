//! T203: the open message's reading leftovers, measured on what the
//! renderer laid out and what the dialog put on screen: the paragraph gap
//! of a plain-text body, the measure, find keeping the reading position,
//! and the column's ground.

use std::collections::BTreeMap;

use adw::prelude::*;
use gtk::gdk;

use crate::open_reading::{opened, scroller_of};
use crate::support::{self, Fixture};

/// Prose that wraps: a paragraph of ordinary words, several hundred
/// characters long.
const PROSE: &str = "Uploaded the third draft of the harbour schedule with the \
    changes we talked about on Monday. The main differences are in the \
    second section, where the crossings are listed by the hour rather than \
    by the boat, and in the notes at the end, which now say who to call when \
    a crossing is cancelled. Please leave comments by Wednesday; I would \
    like to send it to the printers on Thursday so the new timetable can go \
    up on the boards before the weekend. The rendered copy and the plain \
    text are attached, and the old one is in the shared folder if you want \
    to compare the two side by side.";

/// Each laid-out line of the body: its top, and the characters it holds
/// (from its first glyph to its last, the spaces between included).
fn lines(document: &postio_render::RenderedDocument) -> Vec<(f64, f64, usize)> {
    let mut by_line: BTreeMap<u32, (f64, f64, usize, usize)> = BTreeMap::new();
    for cluster in &document.text.clusters {
        let entry = by_line
            .entry(cluster.line)
            .or_insert((f64::MAX, f64::MAX, usize::MAX, 0));
        entry.0 = entry.0.min(cluster.rect.y0);
        entry.1 = entry.1.min(cluster.rect.x0);
        entry.2 = entry.2.min(cluster.range.start);
        entry.3 = entry.3.max(cluster.range.end);
    }
    by_line
        .into_values()
        .map(|(top, left, start, end)| (top, left, end - start))
        .collect()
}

/// A plain-text paragraph break is a gap of about two thirds of a line --
/// the reference's 12px at the 15px reading size -- not a whole blank line
/// in the body's line-height.
pub fn a_plain_paragraph_break_is_a_short_gap_not_a_blank_line() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Paragraphs", "x", 10)
            .await;
        fixture
            .write_body(
                message,
                "Hi all,\nThe first paragraph has a second line.\n\nThe second paragraph.",
            )
            .await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let document = reading.reader().view().document().expect("drawn");
        let lines = lines(&document);
        assert_eq!(lines.len(), 3, "three lines laid out: {lines:?}");
        let pitch = lines[1].0 - lines[0].0;
        let gap = lines[2].0 - lines[1].0 - pitch;
        assert!(
            (10.0..=14.0).contains(&gap),
            "a paragraph break adds {gap}px to the {pitch}px line pitch; \
             the reference's gap is 12px, and a blank line would be {pitch}px"
        );
    });
}

/// The measure is the column's: 480px, 32em of the 15px reading size,
/// about 70 characters (the message dialog handoff, T207, which replaces
/// T203's 32em at the left of a full-width column). The text keeps the
/// column's left edge -- the subject's, the sender block's -- and the
/// column is centred in the dialog.
pub fn the_measure_is_near_seventy_characters_in_a_centred_column() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Measure", "x", 10)
            .await;
        fixture.write_body(message, PROSE).await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let view = reading.reader().view().clone();
        let document = view.document().expect("drawn");
        let lines = lines(&document);
        assert!(lines.len() >= 4, "the prose did not wrap: {lines:?}");
        // The last line of a paragraph is as short as it happens to be.
        let longest = lines[..lines.len() - 1]
            .iter()
            .map(|(_, _, chars)| *chars)
            .max()
            .unwrap_or(0);
        assert!(
            (64..=78).contains(&longest),
            "the longest line holds {longest} characters in a {}px column: \
             the measure is not near 70",
            view.width()
        );
        let content = reading.dialog().child().expect("the dialog's content");
        let bounds = view.compute_bounds(&content).expect("in the dialog");
        let (left, right) = (
            bounds.x(),
            content.width() as f32 - bounds.x() - bounds.width(),
        );
        assert!(
            view.width() == 480 && (left - right).abs() <= 1.0,
            "the body is {}px, {left}px from the dialog's left and {right}px from its \
             right: not the centred 480px column",
            view.width()
        );
        let left = lines
            .iter()
            .map(|(_, left, _)| *left)
            .fold(f64::MAX, f64::min);
        assert!(
            left < 1.0,
            "the text is inset {left}px: it does not keep the column's edge"
        );
    });
}

/// Opening find (`mod+f`) keeps the reading position: the bar is above
/// the column, not at its top, so neither showing it nor its entry taking
/// the keyboard scrolls the message; a query's first match is the first
/// one from where the person is reading; and Escape closes find, not the
/// message.
pub fn opening_find_keeps_the_reading_position() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Long", "x", 10)
            .await;
        let body = (1..=300)
            .map(|n| format!("Line {n} of a long message about the harbour."))
            .collect::<Vec<_>>()
            .join("\n");
        fixture.write_body(message, &body).await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let view = reading.reader().view().clone();
        let column = scroller_of(&view).vadjustment();
        column.set_value(2000.0);
        crate::settle();
        let at = column.value();
        assert!(at > 1000.0, "the column did not scroll: {at}");

        support::press(&window, "f", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || reading.reader().finding()).await,
            "mod+f did not open find in the open message"
        );
        crate::settle();
        assert!(
            (column.value() - at).abs() < 1.0,
            "opening find moved the column from {at} to {}",
            column.value()
        );
        let bar = reading.reader().find_bar().widget().clone();
        assert!(bar.is_mapped(), "the find bar is not on screen");
        assert_ne!(
            bar.ancestor(gtk::ScrolledWindow::static_type()),
            Some(scroller_of(&view).upcast()),
            "the find bar scrolls with the column, so it is only on screen at the top"
        );

        reading.reader().find_bar().entry().set_text("harbour");
        crate::settle_until(async || view.current_match().is_some()).await;
        crate::settle();
        assert!(
            column.value() >= at - 1.0,
            "the first match was taken from the top: the column went from {at} to {}",
            column.value()
        );

        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || !reading.reader().finding()).await,
            "Escape did not close find"
        );
        assert!(reading.is_open(), "Escape in find closed the message");
    });
}

/// One pixel of `texture`, as RGB bytes.
fn pixel(texture: &gdk::Texture, x: i32, y: i32) -> [u8; 3] {
    let (width, height) = (texture.width(), texture.height());
    assert!(
        (0..width).contains(&x) && (0..height).contains(&y),
        "({x}, {y}) is outside the {width}x{height} picture"
    );
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let at = y as usize * stride + x as usize * 4;
    [bytes[at], bytes[at + 1], bytes[at + 2]]
}

/// The column's ground is the dialog's own surface token, not a colour
/// written into the reader: in light and in dark, the body's ground is the
/// colour the dialog paints around it, to the pixel.
pub fn the_columns_ground_is_the_dialogs_own_in_light_and_dark() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let manager = adw::StyleManager::default();
        manager.set_color_scheme(adw::ColorScheme::ForceLight);
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Ground", "x", 10)
            .await;
        fixture.write_body(message, "A short line.").await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        let view = reading.reader().view().clone();
        let mut seen = Vec::new();
        for scheme in [adw::ColorScheme::ForceLight, adw::ColorScheme::ForceDark] {
            manager.set_color_scheme(scheme);
            crate::settle();
            assert!(
                crate::settle_until(async || view.tiles_settled()).await,
                "the body was not drawn again"
            );
            crate::settle_for(std::time::Duration::from_millis(300)).await;
            let picture = postio_widgets::capture::texture(&dialog).expect("the dialog was drawn");
            let body = view
                .compute_bounds(&dialog)
                .expect("the body is in the dialog");
            // Beside the body, in the column's gutter; and inside the
            // body, right of its one short line.
            let y = (body.y() + 6.0) as i32;
            let around = pixel(&picture.texture, (body.x() - 6.0) as i32, y);
            let inside = pixel(&picture.texture, (body.x() + body.width() - 6.0) as i32, y);
            let close = around.iter().zip(inside).all(|(a, b)| a.abs_diff(b) <= 1);
            assert!(
                close,
                "{scheme:?}: the body's ground {inside:?} is not the dialog's {around:?}"
            );
            seen.push(around);
        }
        manager.set_color_scheme(adw::ColorScheme::Default);
        assert_ne!(seen[0], seen[1], "the dialog did not go dark");
    });
}

/// Where `word`'s glyphs are drawn: the first place it stands.
fn glyphs(document: &postio_render::RenderedDocument, word: &str) -> postio_render::Rect {
    let found = document.text.find(word);
    let range = found
        .first()
        .unwrap_or_else(|| panic!("{word:?} is not drawn"))
        .clone();
    let rects = document.text.rects(range);
    rects
        .iter()
        .skip(1)
        .fold(rects[0], |all, rect| all.union(*rect))
}

/// The line box a run of glyphs sits in, `line_height` tall: the glyphs
/// are centred in it, half the leading above and half below.
fn line_box(glyphs: postio_render::Rect, line_height: f64) -> (f64, f64) {
    let leading = (line_height - glyphs.height()) / 2.0;
    (glyphs.y0 - leading, glyphs.y1 + leading)
}

/// T208: a plain-text list is drawn as a list -- its items under a 20px
/// indent, 4px apart on the 24px line -- not as lines that start with a
/// dash; the paragraphs around it keep the column's edge.
pub fn a_plain_list_draws_as_a_list_with_the_handoffs_rhythm() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "List", "x", 10)
            .await;
        fixture
            .write_body(
                message,
                "The differences:\n\n- Cursor pagination\n- Rate limits\n- Exports moved\n\nThanks",
            )
            .await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let document = reading.reader().view().document().expect("drawn");
        assert!(
            document.text.find("- Cursor").is_empty(),
            "the dash is drawn as text, not as the list's bullet"
        );
        let intro = glyphs(&document, "The differences");
        let items = ["Cursor", "Rate", "Exports"].map(|word| glyphs(&document, word));
        assert!(intro.x0 < 1.0, "the paragraph is inset {}px", intro.x0);
        for item in &items {
            assert!(
                (item.x0 - 20.0).abs() <= 1.0,
                "an item's words start {}px in, not under the 20px indent",
                item.x0
            );
        }
        for pair in items.windows(2) {
            let pitch = pair[1].y0 - pair[0].y0;
            assert!(
                (pitch - 28.0).abs() <= 1.0,
                "items are {pitch}px apart: not the 24px line and 4px between"
            );
        }
    });
}

/// T208: the line that introduces a quote sits 20px under the sign-off,
/// smaller and muted, and the quote's 28px toggle 4px under it.
pub fn the_attribution_sits_between_the_sign_off_and_the_toggle() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Lena Park", "lena@example.com"), "Reply", "x", 10)
            .await;
        fixture
            .write_body(
                message,
                "Sounds right.\n\nThanks,\nLena\n\nOn Monday, Ana Ruiz wrote:\n\n> Can we move it?\n> To Thursday?\n",
            )
            .await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let view = reading.reader().view().clone();
        let document = view.document().expect("drawn");
        let sign_off = glyphs(&document, "Lena");
        let attribution = glyphs(&document, "On Monday");
        let fold = document.folds.first().expect("the quote folds");
        let (_, sign_off_bottom) = line_box(sign_off, 24.0);
        let (attribution_top, attribution_bottom) = line_box(attribution, 20.0);
        let gap = attribution_top - sign_off_bottom;
        assert!(
            (gap - 20.0).abs() <= 1.5,
            "the attribution is {gap}px under the sign-off, not 20"
        );
        let gap = fold.summary_rect.y0 - attribution_bottom;
        assert!(
            (gap - 4.0).abs() <= 1.5,
            "the toggle is {gap}px under the attribution, not 4"
        );
        assert!(
            (fold.summary_rect.height() - 28.0).abs() <= 1.0,
            "the toggle is {}px tall",
            fold.summary_rect.height()
        );
        assert!(
            attribution.height() < sign_off.height(),
            "the attribution is not smaller than the body: {} vs {}",
            attribution.height(),
            sign_off.height()
        );
        // Muted: its strongest ink stands less far from the ground than
        // the sign-off's, in whichever scheme the suite runs.
        let picture = postio_widgets::capture::texture(&view).expect("drawn");
        let mut downloader = gdk::TextureDownloader::new(&picture.texture);
        downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        let scale = f64::from(picture.texture.width()) / f64::from(view.width());
        let at = |x: usize, y: usize| -> [i32; 3] {
            let at = y * stride + x * 4;
            [bytes[at], bytes[at + 1], bytes[at + 2]].map(i32::from)
        };
        // The ground: the column's right edge, beside the first line.
        let ground = at(
            picture.texture.width() as usize - 2,
            (sign_off.y0 * scale) as usize,
        );
        let strongest = |rect: postio_render::Rect| -> i32 {
            let mut most = 0;
            for y in (rect.y0 * scale) as usize..(rect.y1 * scale) as usize {
                for x in (rect.x0 * scale) as usize..(rect.x1 * scale) as usize {
                    let pixel = at(x, y);
                    most = most.max((0..3).map(|i| (pixel[i] - ground[i]).abs()).sum());
                }
            }
            most
        };
        let (ink, muted) = (strongest(sign_off), strongest(attribution));
        assert!(
            muted + 60 < ink,
            "the attribution's ink stands {muted} from the ground, the sign-off's {ink}: \
             it is not muted"
        );
    });
}

/// T208: the body's last block keeps no gap under it, so the body ends at
/// its last line and what follows measures its own distance from there.
pub fn the_body_ends_at_its_last_line() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Ends", "x", 10)
            .await;
        fixture
            .write_body(message, "First paragraph.\n\nThe last line.")
            .await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let view = reading.reader().view().clone();
        let document = view.document().expect("drawn");
        let (_, last) = line_box(glyphs(&document, "The last line"), 24.0);
        assert!(
            (f64::from(view.height()) - last).abs() <= 1.5,
            "the body is {}px tall and its last line ends at {last}",
            view.height()
        );
    });
}
