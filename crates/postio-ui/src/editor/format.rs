use postio_body::{Document, HeadingLevel, ListKind, Selection, block_context, marks_in_force};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormatState {
    pub bold: bool,
    pub italic: bool,
    pub heading_1: bool,
    pub bullet_list: bool,
    pub numbered_list: bool,
    pub quote_block: bool,
}

pub fn format_state(document: &Document, selection: &Selection) -> FormatState {
    let marks = marks_in_force(document, selection);
    let context = block_context(document, &selection.head);
    FormatState {
        bold: marks.strong,
        italic: marks.emphasis,
        heading_1: context.heading == Some(HeadingLevel::One),
        bullet_list: matches!(context.list, Some((ListKind::Bullet, false))),
        numbered_list: matches!(context.list, Some((ListKind::Ordered { .. }, _))),
        quote_block: context.quoted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_body::{Block, DocPosition, Inline, ListItem};

    fn caret(path: &[u32], offset: u32) -> Selection {
        Selection::caret(DocPosition::new(path, offset))
    }

    fn draft() -> Document {
        Document {
            blocks: vec![
                Block::Heading {
                    level: HeadingLevel::One,
                    inlines: vec![Inline::Text("Plan".into())],
                },
                Block::List {
                    kind: ListKind::Bullet,
                    items: vec![ListItem::new(vec![Block::Paragraph(vec![
                        Inline::Strong(vec![Inline::Text("bold".into())]),
                        Inline::Text(" plain".into()),
                    ])])],
                },
                Block::Quote(vec![Block::List {
                    kind: ListKind::NUMBERED,
                    items: vec![ListItem::new(vec![Block::Paragraph(vec![
                        Inline::Emphasis(vec![Inline::Text("said".into())]),
                    ])])],
                }]),
            ],
        }
    }

    #[test]
    fn a_heading_lights_heading_and_nothing_else() {
        assert_eq!(
            format_state(&draft(), &caret(&[0], 2)),
            FormatState {
                heading_1: true,
                ..FormatState::default()
            }
        );
    }

    #[test]
    fn a_caret_in_bold_in_a_bullet_lights_both() {
        let state = format_state(&draft(), &caret(&[1, 0, 0], 2));
        assert!(state.bold && state.bullet_list && !state.numbered_list);
        let past = format_state(&draft(), &caret(&[1, 0, 0], 6));
        assert!(!past.bold, "the plain words after it are not bold");
    }

    #[test]
    fn a_numbered_list_in_a_quote_lights_both_and_italic() {
        let state = format_state(&draft(), &caret(&[2, 0, 0, 0], 2));
        assert!(state.quote_block && state.numbered_list && state.italic && !state.bullet_list);
    }

    #[test]
    fn a_range_is_bold_only_when_all_of_it_is() {
        let partly = Selection::range(
            DocPosition::new([1, 0, 0], 0),
            DocPosition::new([1, 0, 0], 6),
        );
        assert!(!format_state(&draft(), &partly).bold);
        let wholly = Selection::range(
            DocPosition::new([1, 0, 0], 0),
            DocPosition::new([1, 0, 0], 4),
        );
        assert!(format_state(&draft(), &wholly).bold);
    }
}
