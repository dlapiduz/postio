//! Fonts without fontconfig (research R3): the bundled faces, the generic
//! families, and per-script fallbacks from what `fontdb` finds installed.

use std::collections::HashSet;
use std::sync::OnceLock;

use parley::layout::PositionedLayoutItem;
use parley::{FontContext, LayoutContext, StyleProperty};
use postio_model::test_corpus;
use postio_render::fonts::{Bundled, FontSet};
use postio_ui::reader::document::FACES;
use skrifa::MetadataProvider as _;

/// Built once: discovery reads every installed face's tables.
fn fonts() -> &'static FontSet {
    static FONTS: OnceLock<FontSet> = OnceLock::new();
    FONTS.get_or_init(|| {
        FontSet::new(Bundled {
            faces: FACES.iter().map(|face| face.bytes).collect(),
            sans: "Barlow",
            mono: "IBM Plex Mono",
        })
    })
}

#[test]
fn the_bundled_faces_resolve_by_family_name() {
    for family in ["Barlow", "Barlow Condensed", "IBM Plex Mono"] {
        assert!(fonts().has_family(family), "{family} is not registered");
    }
}

#[test]
fn every_generic_family_resolves() {
    for generic in ["serif", "sans-serif", "monospace"] {
        let family = fonts().generic(generic);
        assert!(family.is_some(), "{generic} resolves to nothing");
    }
    assert_eq!(fonts().generic("sans-serif").as_deref(), Some("Barlow"));
    assert_eq!(
        fonts().generic("monospace").as_deref(),
        Some("IBM Plex Mono")
    );
}

/// The visible text of a fixture's HTML: tags dropped, entities left, which
/// is enough to shape every script in it.
fn text_of(fixture: &str) -> String {
    let parsed = postio_model::mime::parse(test_corpus::load(fixture).bytes());
    let html = parsed.body.html.expect("an HTML fixture");
    let body = html.split_once("<body").map_or(html.as_str(), |(_, b)| b);
    let mut text = String::new();
    let mut in_tag = false;
    for c in body.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    text
}

/// Characters some installed face can draw, decided from the faces' own
/// character maps rather than from FontSet: tofu is only a defect where
/// the machine had a glyph to give.
fn coverable(text: &str) -> HashSet<char> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let wanted: HashSet<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let mut covered = HashSet::new();
    for face in database.faces() {
        database.with_face_data(face.id, |data, index| {
            if let Ok(font) = skrifa::FontRef::from_index(data, index) {
                let map = font.charmap();
                covered.extend(wanted.iter().filter(|c| map.map(**c).is_some()));
            }
        });
    }
    for face in FACES {
        if let Ok(font) = skrifa::FontRef::new(face.bytes) {
            let map = font.charmap();
            covered.extend(wanted.iter().filter(|c| map.map(**c).is_some()));
        }
    }
    covered
}

/// Every character that shaped to glyph 0 -- the missing-glyph box.
fn tofu(text: &str) -> HashSet<char> {
    let mut font_cx: FontContext = fonts().context();
    let mut layout_cx: LayoutContext<()> = LayoutContext::new();
    let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(
        parley::style::FontFamily::Source("Arial, sans-serif".into()),
    ));
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(800.0));
    let mut missing = HashSet::new();
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            for cluster in run.run().clusters() {
                if cluster.glyphs().any(|glyph| glyph.id == 0) {
                    missing.extend(
                        text[cluster.text_range()]
                            .chars()
                            .filter(|c| !c.is_whitespace() && !c.is_control()),
                    );
                }
            }
        }
    }
    missing
}

#[test]
fn cjk_emoji_and_rtl_text_shapes_without_tofu_where_a_face_covers_it() {
    for fixture in ["html-cjk-emoji", "html-rtl-mixed"] {
        let text = text_of(fixture);
        let coverable = coverable(&text);
        let avoidable: Vec<char> = tofu(&text).intersection(&coverable).copied().collect();
        assert!(
            avoidable.is_empty(),
            "{fixture}: drawn as tofu although an installed face covers them: {avoidable:?}"
        );
    }
}

/// Construction registers only the faces discovery found and the bundled
/// bytes it was handed: no other file, and nothing fetched.
#[test]
fn the_font_set_holds_only_discovered_files() {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let discovered: HashSet<std::path::PathBuf> = database
        .faces()
        .filter_map(|face| match &face.source {
            fontdb::Source::File(path) => Some(path.clone()),
            _ => None,
        })
        .collect();
    let files = fonts().files();
    assert!(!files.is_empty(), "no installed face was registered");
    for file in files {
        assert!(
            discovered.contains(file),
            "{} was not discovered",
            file.display()
        );
    }
}
