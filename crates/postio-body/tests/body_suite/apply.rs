use postio_body::{
    Block, BlockRange, DocPosition, Document, EditIntent, Edited, HeadingLevel, Href, Inline, Join,
    ListItem, ListKind, Selection, Unit, apply, quote_of,
};

pub(crate) fn t(s: &str) -> Inline {
    Inline::Text(s.to_owned())
}

pub(crate) fn p(s: &str) -> Block {
    if s.is_empty() {
        Block::Paragraph(Vec::new())
    } else {
        Block::Paragraph(vec![t(s)])
    }
}

pub(crate) fn h(level: HeadingLevel, s: &str) -> Block {
    Block::Heading {
        level,
        inlines: if s.is_empty() { Vec::new() } else { vec![t(s)] },
    }
}

pub(crate) fn li(blocks: Vec<Block>) -> ListItem {
    ListItem::new(blocks)
}

pub(crate) fn ul(items: Vec<ListItem>) -> Block {
    Block::List {
        kind: ListKind::Bullet,
        items,
    }
}

pub(crate) fn ol(items: Vec<ListItem>) -> Block {
    Block::List {
        kind: ListKind::NUMBERED,
        items,
    }
}

pub(crate) fn bullets(texts: &[&str]) -> Block {
    ul(texts.iter().map(|s| li(vec![p(s)])).collect())
}

pub(crate) fn doc(blocks: Vec<Block>) -> Document {
    Document { blocks }
}

pub(crate) fn caret(path: &[u32], offset: u32) -> Selection {
    Selection::caret(DocPosition::new(path, offset))
}

pub(crate) fn span(from: (&[u32], u32), to: (&[u32], u32)) -> Selection {
    Selection::range(
        DocPosition::new(from.0, from.1),
        DocPosition::new(to.0, to.1),
    )
}

pub(crate) fn quoted() -> Block {
    Block::Quoted(quote_of(None, "Earlier words from Ada", "quote"))
}

fn expect(edited: &Edited, blocks: Vec<Block>, at: Selection) {
    assert_eq!(edited.document, doc(blocks), "the document");
    assert_eq!(edited.selection, at, "the caret");
}

#[test]
fn typing_over_a_range_replaces_it_and_starts_a_step() {
    let edited = apply(
        &doc(vec![p("hello world")]),
        &span((&[0], 0), (&[0], 5)),
        EditIntent::Insert("bye".into()),
    );
    expect(&edited, vec![p("bye world")], caret(&[0], 3));
    assert_eq!(edited.join, Join::New);
}

#[test]
fn typing_continues_the_marks_on_its_left_and_is_a_run() {
    let edited = apply(
        &doc(vec![Block::Paragraph(vec![
            Inline::Strong(vec![t("ab")]),
            t(" c"),
        ])]),
        &caret(&[0], 2),
        EditIntent::Insert("x".into()),
    );
    assert_eq!(
        edited.document,
        doc(vec![Block::Paragraph(vec![
            Inline::Strong(vec![t("abx")]),
            t(" c")
        ])])
    );
    assert_eq!(edited.join, Join::Run);
    assert_eq!(
        edited.changed,
        BlockRange {
            start: 0,
            old_end: 1,
            new_end: 1
        }
    );
}

#[test]
fn typing_at_the_end_of_a_link_does_not_lengthen_it() {
    let href = Href::parse("https://example.com/").unwrap();
    let link = Inline::Link {
        href,
        inlines: vec![t("site")],
    };
    let edited = apply(
        &doc(vec![Block::Paragraph(vec![link.clone()])]),
        &caret(&[0], 4),
        EditIntent::Insert("!".into()),
    );
    assert_eq!(
        edited.document,
        doc(vec![Block::Paragraph(vec![link, t("!")])])
    );
}

#[test]
fn enter_splits_a_paragraph_with_the_caret_at_the_second() {
    let edited = apply(&doc(vec![p("ab")]), &caret(&[0], 1), EditIntent::SplitBlock);
    expect(&edited, vec![p("a"), p("b")], caret(&[1], 0));
    assert_eq!(
        edited.changed,
        BlockRange {
            start: 0,
            old_end: 1,
            new_end: 2
        },
        "the two blocks a split makes"
    );
}

#[test]
fn enter_inside_a_heading_keeps_both_halves_headings_and_after_its_end_opens_a_paragraph() {
    let two = HeadingLevel::Two;
    let inside = apply(
        &doc(vec![h(two, "ab")]),
        &caret(&[0], 1),
        EditIntent::SplitBlock,
    );
    expect(&inside, vec![h(two, "a"), h(two, "b")], caret(&[1], 0));
    let after = apply(
        &doc(vec![h(two, "ab")]),
        &caret(&[0], 2),
        EditIntent::SplitBlock,
    );
    expect(&after, vec![h(two, "ab"), p("")], caret(&[1], 0));
}

#[test]
fn enter_in_a_list_item_splits_the_item() {
    let edited = apply(
        &doc(vec![bullets(&["ab"])]),
        &caret(&[0, 0, 0], 1),
        EditIntent::SplitBlock,
    );
    expect(&edited, vec![bullets(&["a", "b"])], caret(&[0, 1, 0], 0));
}

#[test]
fn enter_in_an_empty_last_item_leaves_the_list() {
    let edited = apply(
        &doc(vec![bullets(&["a", ""])]),
        &caret(&[0, 1, 0], 0),
        EditIntent::SplitBlock,
    );
    expect(&edited, vec![bullets(&["a"]), p("")], caret(&[1], 0));
}

#[test]
fn enter_in_an_empty_item_mid_list_splits_the_list_around_a_paragraph() {
    let edited = apply(
        &doc(vec![bullets(&["a", "", "c"])]),
        &caret(&[0, 1, 0], 0),
        EditIntent::SplitBlock,
    );
    expect(
        &edited,
        vec![bullets(&["a"]), p(""), bullets(&["c"])],
        caret(&[1], 0),
    );
}

#[test]
fn enter_in_an_empty_nested_item_outdents_it() {
    let edited = apply(
        &doc(vec![ul(vec![li(vec![p("a"), bullets(&[""])])])]),
        &caret(&[0, 0, 1, 0, 0], 0),
        EditIntent::SplitBlock,
    );
    expect(&edited, vec![bullets(&["a", ""])], caret(&[0, 1, 0], 0));
}

#[test]
fn enter_in_an_empty_quote_child_leaves_the_quote() {
    let edited = apply(
        &doc(vec![Block::Quote(vec![p("said"), p("")])]),
        &caret(&[0, 1], 0),
        EditIntent::SplitBlock,
    );
    expect(
        &edited,
        vec![Block::Quote(vec![p("said")]), p("")],
        caret(&[1], 0),
    );
}

#[test]
fn enter_in_a_code_block_is_a_newline_in_it() {
    let code = |text: &str| Block::Code {
        language: None,
        text: text.to_owned(),
    };
    let edited = apply(
        &doc(vec![code("ab")]),
        &caret(&[0], 1),
        EditIntent::SplitBlock,
    );
    expect(&edited, vec![code("a\nb")], caret(&[0], 2));
}

#[test]
fn backspace_takes_a_character_a_word_or_the_line() {
    let start = doc(vec![p("one two  three")]);
    let char = apply(
        &start,
        &caret(&[0], 14),
        EditIntent::DeleteBackward(Unit::Char),
    );
    expect(&char, vec![p("one two  thre")], caret(&[0], 13));
    assert_eq!(char.join, Join::Run);
    let word = apply(
        &start,
        &caret(&[0], 9),
        EditIntent::DeleteBackward(Unit::Word),
    );
    expect(&word, vec![p("one three")], caret(&[0], 4));
    let line = apply(
        &doc(vec![Block::Paragraph(vec![
            t("one"),
            Inline::Break,
            t("two"),
        ])]),
        &caret(&[0], 7),
        EditIntent::DeleteBackward(Unit::Line),
    );
    expect(
        &line,
        vec![Block::Paragraph(vec![t("one"), Inline::Break])],
        caret(&[0], 4),
    );
}

#[test]
fn backspace_at_the_start_of_a_block_merges_it_into_the_one_before() {
    let edited = apply(
        &doc(vec![p("a"), p("b")]),
        &caret(&[1], 0),
        EditIntent::DeleteBackward(Unit::Char),
    );
    expect(&edited, vec![p("ab")], caret(&[0], 1));
    assert_eq!(
        edited.changed,
        BlockRange {
            start: 0,
            old_end: 2,
            new_end: 1
        }
    );
}

#[test]
fn backspace_after_a_list_joins_its_last_item() {
    let edited = apply(
        &doc(vec![bullets(&["a"]), p("b")]),
        &caret(&[1], 0),
        EditIntent::DeleteBackward(Unit::Char),
    );
    expect(&edited, vec![bullets(&["ab"])], caret(&[0, 0, 0], 1));
}

#[test]
fn backspace_at_the_first_item_takes_it_out_of_the_list() {
    let edited = apply(
        &doc(vec![bullets(&["a", "b"])]),
        &caret(&[0, 0, 0], 0),
        EditIntent::DeleteBackward(Unit::Char),
    );
    expect(&edited, vec![p("a"), bullets(&["b"])], caret(&[0], 0));
}

#[test]
fn backspace_at_a_later_item_joins_the_item_before() {
    let edited = apply(
        &doc(vec![bullets(&["a", "b"])]),
        &caret(&[0, 1, 0], 0),
        EditIntent::DeleteBackward(Unit::Char),
    );
    expect(&edited, vec![bullets(&["ab"])], caret(&[0, 0, 0], 1));
}

#[test]
fn backspace_at_a_quotes_first_child_takes_it_out_of_the_quote() {
    let edited = apply(
        &doc(vec![Block::Quote(vec![p("a"), p("b")])]),
        &caret(&[0, 0], 0),
        EditIntent::DeleteBackward(Unit::Char),
    );
    expect(
        &edited,
        vec![p("a"), Block::Quote(vec![p("b")])],
        caret(&[0], 0),
    );
}

#[test]
fn backspace_after_the_reply_quote_removes_it_whole_and_never_merges_into_it() {
    let edited = apply(
        &doc(vec![p("a"), quoted(), p("b")]),
        &caret(&[2], 0),
        EditIntent::DeleteBackward(Unit::Char),
    );
    expect(&edited, vec![p("a"), p("b")], caret(&[1], 0));
}

#[test]
fn backspace_at_the_very_start_changes_nothing() {
    let start = doc(vec![p("a")]);
    let edited = apply(
        &start,
        &caret(&[0], 0),
        EditIntent::DeleteBackward(Unit::Char),
    );
    assert_eq!(edited.document, start);
    assert!(edited.changed.is_empty());
}

#[test]
fn a_range_over_the_reply_quote_takes_it_whole() {
    let edited = apply(
        &doc(vec![p("ab"), quoted(), p("cd")]),
        &span((&[0], 1), (&[2], 1)),
        EditIntent::DeleteBackward(Unit::Char),
    );
    expect(&edited, vec![p("ad")], caret(&[0], 1));
}

#[test]
fn a_range_from_a_list_item_into_a_paragraph_joins_the_ends() {
    let edited = apply(
        &doc(vec![bullets(&["one", "two"]), p("three"), p("four")]),
        &span((&[0, 0, 0], 2), (&[2], 2)),
        EditIntent::DeleteForward(Unit::Char),
    );
    expect(&edited, vec![bullets(&["onur"])], caret(&[0, 0, 0], 2));
}

#[test]
fn delete_takes_a_character_or_a_word_and_at_the_end_joins_the_next_block() {
    let start = doc(vec![p("one two"), p("three")]);
    let char = apply(
        &start,
        &caret(&[0], 0),
        EditIntent::DeleteForward(Unit::Char),
    );
    expect(&char, vec![p("ne two"), p("three")], caret(&[0], 0));
    let word = apply(
        &start,
        &caret(&[0], 3),
        EditIntent::DeleteForward(Unit::Word),
    );
    expect(&word, vec![p("one"), p("three")], caret(&[0], 3));
    let join = apply(
        &start,
        &caret(&[0], 7),
        EditIntent::DeleteForward(Unit::Char),
    );
    expect(&join, vec![p("one twothree")], caret(&[0], 7));
}

#[test]
fn delete_at_the_end_of_an_item_joins_the_next_item() {
    let edited = apply(
        &doc(vec![bullets(&["a", "b"])]),
        &caret(&[0, 0, 0], 1),
        EditIntent::DeleteForward(Unit::Char),
    );
    expect(&edited, vec![bullets(&["ab"])], caret(&[0, 0, 0], 1));
}

#[test]
fn delete_before_a_rule_removes_the_rule() {
    let edited = apply(
        &doc(vec![p("a"), Block::Rule, p("b")]),
        &caret(&[0], 1),
        EditIntent::DeleteForward(Unit::Char),
    );
    expect(&edited, vec![p("a"), p("b")], caret(&[0], 1));
}

#[test]
fn shift_enter_is_a_line_break_in_the_paragraph() {
    let edited = apply(
        &doc(vec![p("ab")]),
        &caret(&[0], 1),
        EditIntent::InsertBreak,
    );
    expect(
        &edited,
        vec![Block::Paragraph(vec![t("a"), Inline::Break, t("b")])],
        caret(&[0], 2),
    );
}

#[test]
fn an_invalid_selection_changes_nothing() {
    let start = doc(vec![p("a"), quoted()]);
    let edited = apply(&start, &caret(&[1], 0), EditIntent::Insert("x".into()));
    assert_eq!(
        edited.document, start,
        "typing into the reply quote is refused"
    );
    assert!(edited.changed.is_empty());
}
