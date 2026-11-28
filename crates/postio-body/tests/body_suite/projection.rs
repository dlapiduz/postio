use postio_body::projection::{AtomicKind, LINE_BREAK, OBJECT, PrefixKind};
use postio_body::{
    Block, ContentId, DocPosition, Document, EditHistory, EditIntent, EditStep, HeadingLevel,
    Inline, LineKind, ListItem, ListKind, Preedit, Run, Selection, apply, project, quote_of,
};

fn text(s: &str) -> Inline {
    Inline::Text(s.to_owned())
}

fn para(s: &str) -> Block {
    Block::Paragraph(vec![text(s)])
}

fn item(s: &str) -> ListItem {
    ListItem::new(vec![para(s)])
}

pub(crate) fn sample() -> Document {
    Document {
        blocks: vec![
            Block::Heading {
                level: HeadingLevel::Two,
                inlines: vec![text("Plan")],
            },
            Block::List {
                kind: ListKind::Bullet,
                items: vec![
                    item("milk"),
                    ListItem::new(vec![
                        para("eggs"),
                        Block::List {
                            kind: ListKind::Ordered { start: 3 },
                            items: vec![item("brown"), item("white")],
                        },
                    ]),
                ],
            },
            Block::Quote(vec![para("said"), para("twice")]),
            Block::Paragraph(vec![
                Inline::Strong(vec![text("日本")]),
                Inline::Break,
                Inline::Image {
                    content_id: ContentId::parse("cat@example.com").unwrap(),
                    alt: "a cat".to_owned(),
                },
                text("🙂!"),
            ]),
            Block::Rule,
            Block::Quoted(quote_of(None, "Earlier words\nfrom Ada", "quote")),
            para("end"),
        ],
    }
}

fn chars(inlines: &[Inline]) -> u32 {
    inlines
        .iter()
        .map(|inline| match inline {
            Inline::Text(t) | Inline::Code(t) => t.chars().count() as u32,
            Inline::Strong(inner) | Inline::Emphasis(inner) | Inline::Strikethrough(inner) => {
                chars(inner)
            }
            Inline::Link { inlines, .. } => chars(inlines),
            _ => 1,
        })
        .sum()
}

/// Every position the document holds, read from the document itself, not from the projection.
fn every_position(document: &Document) -> Vec<DocPosition> {
    fn walk(blocks: &[Block], path: &mut Vec<u32>, out: &mut Vec<DocPosition>) {
        for (index, block) in blocks.iter().enumerate() {
            path.push(index as u32);
            match block {
                Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                    for offset in 0..=chars(inlines) {
                        out.push(DocPosition::new(path.clone(), offset));
                    }
                }
                Block::List { items, .. } => {
                    for (i, item) in items.iter().enumerate() {
                        path.push(i as u32);
                        walk(&item.blocks, path, out);
                        path.pop();
                    }
                }
                Block::Quote(children) => walk(children, path, out),
                _ => {}
            }
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(&document.blocks, &mut Vec::new(), &mut out);
    out
}

#[test]
fn the_buffer_reads_as_the_document_with_its_prefixes_drawn() {
    let projection = project(&sample(), None);
    assert_eq!(
        projection.text(),
        format!(
            "Plan\n\u{2022} milk\n\u{2022} eggs\n3. brown\n4. white\nsaid\ntwice\n\
             日本{LINE_BREAK}{OBJECT}🙂!\n{OBJECT}\nEarlier words\nfrom Ada\nend"
        )
    );
    let prefixes: Vec<_> = projection
        .runs()
        .filter_map(|(_, run)| match run {
            Run::Prefix { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    assert_eq!(
        prefixes,
        [
            PrefixKind::Bullet { depth: 1 },
            PrefixKind::Bullet { depth: 1 },
            PrefixKind::Number(3),
            PrefixKind::Number(4),
        ]
    );
    let atomics: Vec<_> = projection
        .runs()
        .filter_map(|(_, run)| match run {
            Run::Atomic { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    assert_eq!(atomics, [AtomicKind::Rule, AtomicKind::Quoted]);
}

#[test]
fn each_line_knows_its_kind_and_depths() {
    let styles: Vec<_> = project(&sample(), None)
        .lines()
        .map(|line| {
            (
                line.path,
                line.style.kind,
                line.style.list_depth,
                line.style.quote_depth,
            )
        })
        .collect();
    assert_eq!(
        styles,
        [
            (vec![0], LineKind::Heading(HeadingLevel::Two), 0, 0),
            (vec![1, 0, 0], LineKind::Paragraph, 1, 0),
            (vec![1, 1, 0], LineKind::Paragraph, 1, 0),
            (vec![1, 1, 1, 0, 0], LineKind::Paragraph, 2, 0),
            (vec![1, 1, 1, 1, 0], LineKind::Paragraph, 2, 0),
            (vec![2, 0], LineKind::Paragraph, 0, 1),
            (vec![2, 1], LineKind::Paragraph, 0, 1),
            (vec![3], LineKind::Paragraph, 0, 0),
            (vec![6], LineKind::Paragraph, 0, 0),
        ]
    );
}

#[test]
fn every_position_survives_the_trip_through_the_buffer() {
    let document = sample();
    let projection = project(&document, None);
    let positions = every_position(&document);
    assert_eq!(
        positions.len(),
        5 + 5 + 5 + 6 + 6 + 5 + 6 + 7 + 4,
        "the walk saw every line"
    );
    for position in positions {
        let offset = projection.offset_of(&position);
        assert_eq!(
            projection.position_of(offset),
            position,
            "through offset {offset}"
        );
    }
}

#[test]
fn every_buffer_offset_lands_on_the_nearest_text() {
    let projection = project(&sample(), None);
    let text: Vec<char> = projection.text().chars().collect();
    let lines: Vec<_> = projection.lines().collect();
    for offset in 0..=text.len() as u32 {
        let landed = projection.offset_of(&projection.position_of(offset));
        let inside = lines
            .iter()
            .any(|line| (line.text_start..=line.text_start + line.len).contains(&offset));
        if inside {
            assert_eq!(landed, offset, "offset {offset} is text and stays put");
        } else {
            let next = lines.iter().find(|line| line.text_start > offset);
            let expected = match next {
                Some(line) => line.text_start,
                None => lines.last().map(|l| l.text_start + l.len).unwrap(),
            };
            assert_eq!(
                landed,
                expected,
                "offset {offset} ({:?}) moves to text",
                text.get(offset as usize)
            );
        }
    }
}

#[test]
fn a_bullet_has_no_position_of_its_own() {
    let projection = project(&sample(), None);
    let line = projection.lines().nth(1).unwrap();
    assert_eq!(line.text_start - line.start, 2, "the bullet and its space");
    for offset in line.start..line.text_start {
        assert_eq!(
            projection.position_of(offset),
            DocPosition::new([1, 0, 0], 0)
        );
    }
}

#[test]
fn the_reply_quote_sends_the_caret_past_it() {
    let projection = project(&sample(), None);
    let text = projection.text();
    let inside = text.find("Earlier").unwrap();
    let offset = text[..inside].chars().count() as u32 + 3;
    assert_eq!(projection.position_of(offset), DocPosition::new([6], 0));
}

#[test]
fn utf16_counts_what_appkit_counts() {
    let projection = project(
        &Document {
            blocks: vec![para("日本"), para("🙂a")],
        },
        None,
    );
    assert_eq!(projection.utf16_of(2), 2, "a CJK character is one unit");
    assert_eq!(
        projection.utf16_of(3),
        3,
        "and the newline after the line is one"
    );
    assert_eq!(projection.utf16_of(4), 5, "an emoji outside the BMP is two");
    assert_eq!(projection.utf16_of(5), 6);
}

#[test]
fn an_empty_document_has_one_place_to_be() {
    let projection = project(&Document::new(), None);
    assert_eq!(projection.text(), "");
    assert_eq!(projection.position_of(0), DocPosition::default());
    assert_eq!(projection.offset_of(&DocPosition::new([0], 0)), 0);
}

fn list(text: &str) -> Document {
    Document {
        blocks: vec![
            para("before"),
            Block::List {
                kind: ListKind::Bullet,
                items: vec![item(text)],
            },
        ],
    }
}

#[test]
fn a_preedit_is_a_hole_in_the_buffer_with_no_place_in_the_document() {
    let document = list("ab");
    let at = DocPosition::new([1, 0, 0], 1);
    let preedit = Preedit {
        at: at.clone(),
        text: "にほ".to_owned(),
    };
    let projection = project(&document, Some(&preedit));
    assert_eq!(projection.text(), "before\n\u{2022} aにほb");
    assert!(
        projection
            .runs()
            .any(|(_, run)| *run == Run::Preedit { len: 2 })
    );
    let hole = projection.offset_of(&at);
    for offset in hole..=hole + 2 {
        assert_eq!(
            projection.position_of(offset),
            at,
            "offset {offset} is the hole's anchor"
        );
    }
    assert_eq!(
        projection.position_of(hole + 3),
        DocPosition::new([1, 0, 0], 2)
    );
    for position in every_position(&document) {
        let offset = projection.offset_of(&position);
        assert_eq!(projection.position_of(offset), position, "around the hole");
    }
}

#[test]
fn while_composing_nothing_but_the_commit_or_the_cancel_edits() {
    let document = list("a");
    let composing = Selection {
        preedit: Some("にほ".to_owned()),
        ..Selection::caret(DocPosition::new([1, 0, 0], 1))
    };
    for intent in [
        EditIntent::Insert("x".into()),
        EditIntent::SplitBlock,
        EditIntent::DeleteBackward(postio_body::Unit::Char),
        EditIntent::ToggleMark(postio_body::Mark::Strong),
        EditIntent::Indent,
        EditIntent::Paste(Document::from_text("y")),
    ] {
        let edited = apply(&document, &composing, intent.clone());
        assert_eq!(edited.document, document, "{intent:?} during a preedit");
        assert_eq!(edited.selection, composing, "and the preedit is kept");
    }
    let cancelled = apply(&document, &composing, EditIntent::CancelPreedit);
    assert_eq!(cancelled.document, document);
    assert_eq!(cancelled.selection.preedit, None);
    let committed = apply(
        &document,
        &composing,
        EditIntent::CommitPreedit("日本".into()),
    );
    assert_eq!(committed.document, list("a日本"));
    assert_eq!(
        committed.selection,
        Selection::caret(DocPosition::new([1, 0, 0], 3))
    );
    assert_eq!(
        committed.join,
        postio_body::Join::New,
        "a commit is its own undo step"
    );
}
