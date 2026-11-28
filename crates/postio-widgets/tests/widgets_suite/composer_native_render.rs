use gtk::prelude::*;
use postio_body::{Block, Document, HeadingLevel, Href, Inline, ListItem, ListKind};
use postio_widgets::composer::native::NativeSurface;
use postio_widgets::composer::surface::ComposerSurface;

fn t(s: &str) -> Inline {
    Inline::Text(s.to_owned())
}

pub(crate) fn surface(document: Document) -> NativeSurface {
    let surface = NativeSurface::new(std::rc::Rc::new(|_: &str| None));
    surface.load(document);
    surface
}

pub(crate) fn tags_at(surface: &NativeSurface, needle: &str) -> Vec<String> {
    let buffer = surface.view().buffer();
    let text = buffer.slice(&buffer.start_iter(), &buffer.end_iter(), true);
    let at = text
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} is not in {text:?}"));
    let iter = buffer.iter_at_offset(text[..at].chars().count() as i32);
    let mut names: Vec<String> = iter
        .tags()
        .iter()
        .filter_map(|tag| tag.name().map(|n| n.to_string()))
        .collect();
    names.sort();
    names
}

pub fn each_block_and_mark_is_drawn_by_its_tag() {
    let href = Href::parse("https://example.com/").unwrap();
    let surface = surface(Document {
        blocks: vec![
            Block::Heading {
                level: HeadingLevel::One,
                inlines: vec![t("Plan")],
            },
            Block::List {
                kind: ListKind::Bullet,
                items: vec![ListItem::new(vec![
                    Block::Paragraph(vec![t("milk")]),
                    Block::List {
                        kind: ListKind::NUMBERED,
                        items: vec![ListItem::new(vec![Block::Paragraph(vec![t("oat")])])],
                    },
                ])],
            },
            Block::Quote(vec![Block::Paragraph(vec![t("said")])]),
            Block::Paragraph(vec![
                Inline::Strong(vec![t("bold")]),
                t(" "),
                Inline::Link {
                    href,
                    inlines: vec![t("site")],
                },
                t(" "),
                Inline::Code("f()".into()),
            ]),
        ],
    });
    assert_eq!(tags_at(&surface, "Plan"), ["heading-1"]);
    assert_eq!(
        tags_at(&surface, "\u{2022}"),
        ["list-1", "prefix"],
        "the bullet is drawn, not text"
    );
    assert_eq!(tags_at(&surface, "milk"), ["list-1"]);
    assert_eq!(
        tags_at(&surface, "1. "),
        ["list-2", "prefix"],
        "a nested list sits deeper"
    );
    assert_eq!(tags_at(&surface, "oat"), ["list-2"]);
    assert_eq!(tags_at(&surface, "said"), ["quote-1"]);
    assert_eq!(tags_at(&surface, "bold"), ["strong"]);
    assert_eq!(tags_at(&surface, "site"), ["link"]);
    assert_eq!(tags_at(&surface, "f()"), ["code"]);
}

pub fn the_reply_quote_is_one_unit_no_caret_enters() {
    let surface = surface(Document {
        blocks: vec![
            Block::Paragraph(vec![t("Thanks")]),
            Block::Quoted(postio_body::quote_of(
                None,
                "Earlier words from Ada",
                "quote",
            )),
        ],
    });
    assert_eq!(tags_at(&surface, "Earlier"), ["quoted"]);
    let buffer = surface.view().buffer();
    assert!(
        !buffer.iter_at_offset(10).can_insert(true),
        "the quote is not editable"
    );
}
