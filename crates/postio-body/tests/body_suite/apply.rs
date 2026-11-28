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

fn strong(s: &str) -> Inline {
    Inline::Strong(vec![t(s)])
}

fn href() -> Href {
    Href::parse("https://example.com/").unwrap()
}

#[test]
fn bold_over_part_of_a_block_and_again_gives_the_block_back() {
    use postio_body::Mark;
    let start = doc(vec![p("hello world")]);
    let range = span((&[0], 0), (&[0], 5));
    let once = apply(&start, &range, EditIntent::ToggleMark(Mark::Strong));
    assert_eq!(
        once.document,
        doc(vec![Block::Paragraph(vec![strong("hello"), t(" world")])])
    );
    assert_eq!(
        once.selection, range,
        "the selection stays on what was made bold"
    );
    assert_eq!(once.join, Join::New);
    let twice = apply(&once.document, &range, EditIntent::ToggleMark(Mark::Strong));
    assert_eq!(twice.document, start);
}

#[test]
fn a_mark_over_a_partly_marked_range_covers_all_of_it() {
    use postio_body::Mark;
    let edited = apply(
        &doc(vec![Block::Paragraph(vec![strong("ab"), t("cd")])]),
        &span((&[0], 1), (&[0], 3)),
        EditIntent::ToggleMark(Mark::Strong),
    );
    assert_eq!(
        edited.document,
        doc(vec![Block::Paragraph(vec![strong("abc"), t("d")])])
    );
}

#[test]
fn a_mark_across_blocks_marks_each_blocks_covered_part() {
    use postio_body::Mark;
    let edited = apply(
        &doc(vec![bullets(&["ab"]), p("cd")]),
        &span((&[0, 0, 0], 1), (&[1], 1)),
        EditIntent::ToggleMark(Mark::Emphasis),
    );
    let em = |s: &str| Inline::Emphasis(vec![t(s)]);
    assert_eq!(
        edited.document,
        doc(vec![
            ul(vec![li(vec![Block::Paragraph(vec![t("a"), em("b")])])]),
            Block::Paragraph(vec![em("c"), t("d")]),
        ])
    );
}

#[test]
fn inline_code_over_a_range_is_one_code_run() {
    use postio_body::Mark;
    let edited = apply(
        &doc(vec![p("call f() now")]),
        &span((&[0], 5), (&[0], 8)),
        EditIntent::ToggleMark(Mark::Code),
    );
    assert_eq!(
        edited.document,
        doc(vec![Block::Paragraph(vec![
            t("call "),
            Inline::Code("f()".into()),
            t(" now")
        ])])
    );
}

#[test]
fn a_mark_at_a_caret_is_for_the_next_typed_text() {
    use postio_body::Mark;
    let start = doc(vec![p("a")]);
    let toggled = apply(
        &start,
        &caret(&[0], 1),
        EditIntent::ToggleMark(Mark::Strong),
    );
    assert_eq!(toggled.document, start, "nothing to mark yet");
    assert!(toggled.selection.marks.as_ref().is_some_and(|m| m.strong));
    let typed = apply(
        &toggled.document,
        &toggled.selection,
        EditIntent::Insert("b".into()),
    );
    assert_eq!(
        typed.document,
        doc(vec![Block::Paragraph(vec![t("a"), strong("b")])])
    );
}

#[test]
fn a_link_is_set_over_a_range_and_removed_from_the_caret_inside_it() {
    let linked = apply(
        &doc(vec![p("see the site")]),
        &span((&[0], 8), (&[0], 12)),
        EditIntent::SetLink(Some(href())),
    );
    let link = Inline::Link {
        href: href(),
        inlines: vec![t("site")],
    };
    assert_eq!(
        linked.document,
        doc(vec![Block::Paragraph(vec![t("see the "), link])])
    );
    let unlinked = apply(
        &linked.document,
        &caret(&[0], 10),
        EditIntent::SetLink(None),
    );
    assert_eq!(unlinked.document, doc(vec![p("see the site")]));
}

#[test]
fn a_link_at_a_caret_outside_any_link_types_its_address() {
    let edited = apply(
        &doc(vec![p("at ")]),
        &caret(&[0], 3),
        EditIntent::SetLink(Some(href())),
    );
    assert_eq!(
        edited.document,
        doc(vec![Block::Paragraph(vec![
            t("at "),
            Inline::Link {
                href: href(),
                inlines: vec![t("https://example.com/")]
            }
        ])])
    );
    assert_eq!(edited.selection, caret(&[0], 23));
}

#[test]
fn an_image_goes_in_at_the_caret_as_one_character() {
    let content_id = postio_body::ContentId::parse("cat@example.com").unwrap();
    let edited = apply(
        &doc(vec![p("ab")]),
        &caret(&[0], 1),
        EditIntent::InsertImage {
            content_id: content_id.clone(),
            alt: "a cat".into(),
        },
    );
    assert_eq!(
        edited.document,
        doc(vec![Block::Paragraph(vec![
            t("a"),
            Inline::Image {
                content_id,
                alt: "a cat".into()
            },
            t("b")
        ])])
    );
    assert_eq!(edited.selection, caret(&[0], 2));
}

#[test]
fn a_heading_toggles_on_at_each_level_and_off_again() {
    use postio_body::BlockKind;
    for level in [HeadingLevel::One, HeadingLevel::Two, HeadingLevel::Three] {
        let on = apply(
            &doc(vec![p("Plan")]),
            &caret(&[0], 2),
            EditIntent::ToggleBlock(BlockKind::Heading(level)),
        );
        assert_eq!(on.document, doc(vec![h(level, "Plan")]));
        assert_eq!(on.selection, caret(&[0], 2));
        let off = apply(
            &on.document,
            &on.selection,
            EditIntent::ToggleBlock(BlockKind::Heading(level)),
        );
        assert_eq!(off.document, doc(vec![p("Plan")]), "{level:?} toggles off");
    }
    let other = apply(
        &doc(vec![h(HeadingLevel::One, "Plan")]),
        &caret(&[0], 0),
        EditIntent::ToggleBlock(BlockKind::Heading(HeadingLevel::Two)),
    );
    assert_eq!(
        other.document,
        doc(vec![h(HeadingLevel::Two, "Plan")]),
        "another level replaces it"
    );
}

#[test]
fn a_list_wraps_the_selected_blocks_and_toggles_off_to_paragraphs() {
    use postio_body::BlockKind;
    let on = apply(
        &doc(vec![p("a"), p("b"), p("c")]),
        &span((&[0], 0), (&[1], 1)),
        EditIntent::ToggleBlock(BlockKind::BulletList),
    );
    assert_eq!(on.document, doc(vec![bullets(&["a", "b"]), p("c")]));
    assert_eq!(on.selection, span((&[0, 0, 0], 0), (&[0, 1, 0], 1)));
    let off = apply(
        &on.document,
        &on.selection,
        EditIntent::ToggleBlock(BlockKind::BulletList),
    );
    assert_eq!(off.document, doc(vec![p("a"), p("b"), p("c")]));
}

#[test]
fn a_numbered_toggle_in_a_bulleted_list_renumbers_the_list() {
    use postio_body::BlockKind;
    let edited = apply(
        &doc(vec![bullets(&["a", "b"])]),
        &caret(&[0, 1, 0], 0),
        EditIntent::ToggleBlock(BlockKind::NumberedList),
    );
    assert_eq!(
        edited.document,
        doc(vec![ol(vec![li(vec![p("a")]), li(vec![p("b")])])])
    );
    let off = apply(
        &edited.document,
        &edited.selection,
        EditIntent::ToggleBlock(BlockKind::NumberedList),
    );
    assert_eq!(off.document, doc(vec![ol(vec![li(vec![p("a")])]), p("b")]));
}

#[test]
fn a_quote_wraps_the_block_and_toggles_off() {
    use postio_body::BlockKind;
    let on = apply(
        &doc(vec![p("said")]),
        &caret(&[0], 1),
        EditIntent::ToggleBlock(BlockKind::Quote),
    );
    assert_eq!(on.document, doc(vec![Block::Quote(vec![p("said")])]));
    assert_eq!(on.selection, caret(&[0, 0], 1));
    let off = apply(
        &on.document,
        &on.selection,
        EditIntent::ToggleBlock(BlockKind::Quote),
    );
    assert_eq!(off.document, doc(vec![p("said")]));
    assert_eq!(off.selection, caret(&[0], 1));
}

#[test]
fn tab_nests_an_item_under_the_one_before_and_shift_tab_takes_it_back() {
    let start = doc(vec![bullets(&["a", "b", "c"])]);
    let nested = apply(&start, &caret(&[0, 1, 0], 1), EditIntent::Indent);
    assert_eq!(
        nested.document,
        doc(vec![ul(vec![
            li(vec![p("a"), bullets(&["b"])]),
            li(vec![p("c")])
        ])])
    );
    assert_eq!(nested.selection, caret(&[0, 0, 1, 0, 0], 1));
    let again = apply(&nested.document, &caret(&[0, 1, 0], 0), EditIntent::Indent);
    assert_eq!(
        again.document,
        doc(vec![ul(vec![li(vec![p("a"), bullets(&["b", "c"])])])]),
        "a second item joins the nested list"
    );
    let back = apply(&nested.document, &nested.selection, EditIntent::Outdent);
    assert_eq!(back.document, start);
    assert_eq!(back.selection, caret(&[0, 1, 0], 1));
}

#[test]
fn a_first_item_cannot_indent_and_shift_tab_at_the_top_leaves_the_list() {
    let start = doc(vec![bullets(&["a", "b"])]);
    let refused = apply(&start, &caret(&[0, 0, 0], 0), EditIntent::Indent);
    assert_eq!(refused.document, start);
    assert!(refused.changed.is_empty());
    let out = apply(&start, &caret(&[0, 1, 0], 1), EditIntent::Outdent);
    assert_eq!(out.document, doc(vec![bullets(&["a"]), p("b")]));
    assert_eq!(out.selection, caret(&[1], 1));
    let not_a_list = apply(&doc(vec![p("a")]), &caret(&[0], 0), EditIntent::Indent);
    assert!(
        not_a_list.changed.is_empty(),
        "Tab outside a list moves the focus instead"
    );
}

#[test]
fn shift_tab_on_a_nested_item_carries_the_items_after_it() {
    let edited = apply(
        &doc(vec![ul(vec![li(vec![p("a"), bullets(&["b", "c"])])])]),
        &caret(&[0, 0, 1, 0, 0], 0),
        EditIntent::Outdent,
    );
    assert_eq!(
        edited.document,
        doc(vec![ul(vec![
            li(vec![p("a")]),
            li(vec![p("b"), bullets(&["c"])])
        ])])
    );
    assert_eq!(edited.selection, caret(&[0, 1, 0], 0));
}

#[test]
fn a_rule_goes_after_the_text_with_a_line_for_the_caret() {
    let at_end = apply(
        &doc(vec![p("abc")]),
        &caret(&[0], 3),
        EditIntent::InsertRule,
    );
    assert_eq!(at_end.document, doc(vec![p("abc"), Block::Rule, p("")]));
    assert_eq!(at_end.selection, caret(&[2], 0));
    let empty = apply(
        &doc(vec![p(""), p("next")]),
        &caret(&[0], 0),
        EditIntent::InsertRule,
    );
    assert_eq!(
        empty.document,
        doc(vec![Block::Rule, p("next")]),
        "an empty line becomes the rule"
    );
    assert_eq!(empty.selection, caret(&[1], 0));
}

#[test]
fn pasted_inline_content_joins_the_caret_block() {
    let pasted = doc(vec![Block::Paragraph(vec![strong("big")])]);
    let edited = apply(
        &doc(vec![p("ab")]),
        &caret(&[0], 1),
        EditIntent::Paste(pasted),
    );
    assert_eq!(
        edited.document,
        doc(vec![Block::Paragraph(vec![t("a"), strong("big"), t("b")])])
    );
    assert_eq!(edited.selection, caret(&[0], 4));
    assert_eq!(edited.join, Join::New);
}

#[test]
fn pasted_blocks_go_between_the_halves_of_the_caret_block() {
    let pasted = doc(vec![p("one"), bullets(&["two"]), p("three")]);
    let edited = apply(
        &doc(vec![p("ab")]),
        &caret(&[0], 1),
        EditIntent::Paste(pasted),
    );
    assert_eq!(
        edited.document,
        doc(vec![p("aone"), bullets(&["two"]), p("threeb")])
    );
    assert_eq!(
        edited.selection,
        caret(&[2], 5),
        "the caret is after what was pasted"
    );
}

#[test]
fn a_pasted_list_into_an_empty_line_replaces_the_line() {
    let edited = apply(
        &doc(vec![p("x"), p("")]),
        &caret(&[1], 0),
        EditIntent::Paste(doc(vec![bullets(&["a", "b"])])),
    );
    assert_eq!(
        edited.document,
        doc(vec![p("x"), bullets(&["a", "b"]), p("")])
    );
    assert_eq!(edited.selection, caret(&[2], 0));
}

#[test]
fn paste_over_a_range_replaces_it() {
    let edited = apply(
        &doc(vec![p("hello world")]),
        &span((&[0], 6), (&[0], 11)),
        EditIntent::Paste(doc(vec![p("there")])),
    );
    assert_eq!(edited.document, doc(vec![p("hello there")]));
}

#[test]
fn hostile_markup_arrives_narrowed() {
    let hostile = postio_body::parse(
        "<p style=\"color:red\" onclick=\"steal()\">hi<script>steal()</script>\
         <img src=\"https://tracker.example.com/p.gif\"><a href=\"javascript:steal()\">x</a></p>\
         <iframe src=\"https://example.com/\"></iframe>",
    );
    let edited = apply(
        &doc(vec![p("")]),
        &caret(&[0], 0),
        EditIntent::Paste(hostile),
    );
    assert_eq!(
        edited.document,
        doc(vec![p("hix")]),
        "no script, no remote image, no link to code"
    );
}
