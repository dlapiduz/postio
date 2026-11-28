use crate::document::{Block, Document, Inline};

/// `path` is a block index, then for each list entered an item index and a block index within the
/// item, and for each quote entered a child index. `Ord` is document order.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocPosition {
    pub path: Vec<u32>,
    pub offset: u32,
}

impl DocPosition {
    pub fn new(path: impl Into<Vec<u32>>, offset: u32) -> Self {
        DocPosition {
            path: path.into(),
            offset,
        }
    }

    pub fn top(&self) -> usize {
        self.path.first().map_or(0, |&top| top as usize)
    }

    pub fn is_valid_in(&self, document: &Document) -> bool {
        block_at(&document.blocks, &self.path)
            .and_then(text_len)
            .is_some_and(|len| self.offset <= len)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: DocPosition,
    pub head: DocPosition,
    pub marks: Option<crate::MarkSet>,
    pub preedit: Option<String>,
}

impl Selection {
    pub fn caret(at: DocPosition) -> Self {
        Selection {
            anchor: at.clone(),
            head: at,
            marks: None,
            preedit: None,
        }
    }

    pub fn range(anchor: DocPosition, head: DocPosition) -> Self {
        Selection {
            anchor,
            head,
            marks: None,
            preedit: None,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.head
    }

    pub fn start(&self) -> &DocPosition {
        if self.anchor <= self.head {
            &self.anchor
        } else {
            &self.head
        }
    }

    pub fn end(&self) -> &DocPosition {
        if self.anchor <= self.head {
            &self.head
        } else {
            &self.anchor
        }
    }

    pub fn is_valid_in(&self, document: &Document) -> bool {
        self.anchor.is_valid_in(document) && self.head.is_valid_in(document)
    }

    pub fn composing(&self) -> Option<crate::projection::Preedit> {
        self.preedit
            .as_ref()
            .map(|text| crate::projection::Preedit {
                at: self.head.clone(),
                text: text.clone(),
            })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockContext {
    pub heading: Option<crate::HeadingLevel>,
    pub code: bool,
    pub list: Option<(crate::ListKind, bool)>,
    pub quoted: bool,
}

pub fn block_context(document: &Document, at: &DocPosition) -> BlockContext {
    let mut context = BlockContext::default();
    let mut blocks: &[Block] = &document.blocks;
    let mut path = at.path.as_slice();
    while let Some((&index, rest)) = path.split_first() {
        let Some(block) = blocks.get(index as usize) else {
            break;
        };
        match block {
            Block::Quote(children) => {
                context.quoted = true;
                blocks = children;
                path = rest;
            }
            Block::List { kind, items } => {
                let Some((&item, rest)) = rest.split_first() else {
                    break;
                };
                let Some(held) = items.get(item as usize) else {
                    break;
                };
                context.list = Some((*kind, held.task.is_some()));
                blocks = &held.blocks;
                path = rest;
            }
            Block::Heading { level, .. } => {
                context.heading = Some(*level);
                break;
            }
            Block::Code { .. } => {
                context.code = true;
                break;
            }
            _ => break,
        }
    }
    context
}

pub(crate) fn block_at<'a>(blocks: &'a [Block], path: &[u32]) -> Option<&'a Block> {
    let (&first, rest) = path.split_first()?;
    let block = blocks.get(first as usize)?;
    if rest.is_empty() {
        return Some(block);
    }
    match block {
        Block::Quote(children) => block_at(children, rest),
        Block::List { items, .. } => {
            let (&item, rest) = rest.split_first()?;
            if rest.is_empty() {
                return None;
            }
            block_at(&items.get(item as usize)?.blocks, rest)
        }
        _ => None,
    }
}

pub(crate) fn text_len(block: &Block) -> Option<u32> {
    match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => Some(inlines_len(inlines)),
        Block::Code { text, .. } => Some(text.chars().count() as u32),
        _ => None,
    }
}

pub(crate) fn inlines_len(inlines: &[Inline]) -> u32 {
    inlines
        .iter()
        .map(|inline| match inline {
            Inline::Text(text) | Inline::Code(text) => text.chars().count() as u32,
            Inline::Strong(inner)
            | Inline::Emphasis(inner)
            | Inline::Strikethrough(inner)
            | Inline::Link { inlines: inner, .. } => inlines_len(inner),
            Inline::Image { .. } | Inline::RemoteImage { .. } | Inline::Break => 1,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ListItem, ListKind};
    use crate::{Quoted, quote_of};

    fn para(text: &str) -> Block {
        Block::Paragraph(vec![Inline::Text(text.to_owned())])
    }

    fn quoted() -> Quoted {
        quote_of(None, "earlier words", "quote")
    }

    fn sample() -> Document {
        Document {
            blocks: vec![
                para("one"),
                Block::List {
                    kind: ListKind::Bullet,
                    items: vec![
                        ListItem::new(vec![para("a")]),
                        ListItem::new(vec![
                            para("bc"),
                            Block::List {
                                kind: ListKind::NUMBERED,
                                items: vec![ListItem::new(vec![para("nested")])],
                            },
                        ]),
                    ],
                },
                Block::Quote(vec![para("said")]),
                Block::Rule,
                Block::Quoted(quoted()),
                Block::Paragraph(vec![
                    Inline::Strong(vec![Inline::Text("日本".to_owned())]),
                    Inline::Image {
                        content_id: crate::ContentId::parse("x@example.com").unwrap(),
                        alt: "a long alt".to_owned(),
                    },
                ]),
            ],
        }
    }

    #[test]
    fn a_position_is_valid_only_in_a_block_with_text() {
        let doc = sample();
        for (path, offset) in [
            (vec![0], 3),
            (vec![1, 0, 0], 1),
            (vec![1, 1, 0], 2),
            (vec![1, 1, 1, 0, 0], 6),
            (vec![2, 0], 0),
            (vec![5], 3),
        ] {
            assert!(
                DocPosition::new(path.clone(), offset).is_valid_in(&doc),
                "{path:?}@{offset}"
            );
        }
    }

    #[test]
    fn no_position_is_in_a_reply_quote_a_rule_a_container_or_past_the_text() {
        let doc = sample();
        for (path, offset, why) in [
            (vec![3], 0, "a rule"),
            (vec![4], 0, "the reply quote"),
            (vec![1], 0, "a list is not text"),
            (vec![1, 0], 0, "an item is not text"),
            (vec![2], 0, "a quote is not text"),
            (vec![0], 4, "past the end"),
            (vec![5], 4, "an image's alt is one character, not its words"),
            (vec![6], 0, "no such block"),
            (vec![], 0, "no block at all"),
        ] {
            assert!(!DocPosition::new(path, offset).is_valid_in(&doc), "{why}");
        }
    }

    #[test]
    fn positions_order_as_the_document_reads() {
        let reading = [
            DocPosition::new([0], 0),
            DocPosition::new([0], 3),
            DocPosition::new([1, 0, 0], 0),
            DocPosition::new([1, 1, 0], 1),
            DocPosition::new([1, 1, 1, 0, 0], 0),
            DocPosition::new([2, 0], 0),
            DocPosition::new([5], 0),
        ];
        for pair in reading.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{:?} reads before {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn a_selection_knows_its_ends_whichever_way_it_was_made() {
        let early = DocPosition::new([0], 1);
        let late = DocPosition::new([2, 0], 2);
        let backwards = Selection::range(late.clone(), early.clone());
        assert_eq!((backwards.start(), backwards.end()), (&early, &late));
        assert!(!backwards.is_collapsed());
        assert!(Selection::caret(early.clone()).is_collapsed());
        assert!(backwards.is_valid_in(&sample()));
        assert!(!Selection::range(early, DocPosition::new([4], 0)).is_valid_in(&sample()));
    }
}
