//! A typed Markdown sequence becoming its block or mark (spec 002 FR-068). Which characters are a
//! trigger is `postio_ui::editor::markdown`'s question; this module knows no Markdown.

use std::rc::Rc;

use super::flat::{self, Cell, MarkSet};
use super::marks::with;
use super::{BlockKind, Join, Mark, Outcome, blocks, lists, tree};
use crate::document::{Block, HeadingLevel, Href, Language, ListKind, Task};
use crate::position::{DocPosition, Selection, block_at};

/// The `trigger_len` characters before the caret are the sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversion {
    pub trigger_len: u32,
    pub kind: ConversionKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversionKind {
    Heading(HeadingLevel),
    List(ListKind),
    Task(Task),
    Quote,
    /// A line alone: `---`, `***` or `___`.
    Rule,
    /// The marker under a line of text: the trigger is the break and the marker.
    Setext(HeadingLevel),
    CodeBlock(Option<Language>),
    /// The trigger opens and closes with `marker_len` characters around the text the marks cover.
    Wrap {
        marks: Vec<Mark>,
        marker_len: u32,
    },
    /// `label` is the start and length of the link's text within the trigger.
    Link {
        href: Href,
        label: (u32, u32),
    },
}

pub(super) fn convert(
    blocks: &mut Vec<Block>,
    selection: &Selection,
    conversion: Conversion,
) -> Option<Outcome> {
    if !selection.is_collapsed() || conversion.trigger_len > selection.head.offset {
        return None;
    }
    let at = selection.head.clone();
    let to = at.offset as usize;
    let from = to - conversion.trigger_len as usize;
    let start = DocPosition::new(at.path.clone(), from as u32);
    match conversion.kind {
        ConversionKind::Wrap { marks, marker_len } => {
            let width = marker_len as usize;
            if to - from <= 2 * width {
                return None;
            }
            let left = tree::edit_cells(blocks, &at.path, |cells| {
                cells.drain(to - width..to);
                cells.drain(from..from + width);
                for cell in &mut cells[from..to - 2 * width] {
                    let mut next = MarkSet::clone(&cell.marks);
                    for mark in &marks {
                        next = with(next, *mark, true);
                    }
                    cell.marks = Rc::new(next);
                }
                MarkSet::clone(&cells[to - 2 * width - 1].marks)
            })?;
            let after = marks
                .iter()
                .fold(left, |marks, mark| with(marks, *mark, false));
            let caret = DocPosition::new(at.path, (to - 2 * width) as u32);
            Some(continuing(caret, after))
        }
        ConversionKind::Link { href, label } => {
            let (label_at, label_len) = (label.0 as usize, label.1 as usize);
            let left = tree::edit_cells(blocks, &at.path, |cells| {
                let linked: Vec<Cell> = cells[from + label_at..from + label_at + label_len]
                    .iter()
                    .map(|cell| Cell {
                        atom: cell.atom.clone(),
                        marks: Rc::new(MarkSet {
                            link: Some(href.clone()),
                            ..MarkSet::clone(&cell.marks)
                        }),
                    })
                    .collect();
                let marks = linked.last().map(|cell| MarkSet::clone(&cell.marks));
                cells.splice(from..to, linked);
                marks
            })??;
            let caret = DocPosition::new(at.path, (from + label_len) as u32);
            Some(continuing(caret, MarkSet { link: None, ..left }))
        }
        kind => {
            tree::edit_cells(blocks, &at.path, |cells| {
                cells.drain(from..to);
            })?;
            let here = Selection::caret(start.clone());
            block_conversion(blocks, &here, kind)
        }
    }
}

fn continuing(caret: DocPosition, marks: MarkSet) -> Outcome {
    Outcome {
        selection: Selection {
            marks: Some(marks),
            ..Selection::caret(caret)
        },
        join: Join::New,
    }
}

fn block_conversion(
    blocks: &mut Vec<Block>,
    here: &Selection,
    kind: ConversionKind,
) -> Option<Outcome> {
    let path = here.head.path.clone();
    let as_list = |kind: ConversionKind| match kind {
        ConversionKind::List(ListKind::Bullet) => Some(BlockKind::BulletList),
        ConversionKind::List(ListKind::Ordered { .. }) => Some(BlockKind::NumberedList),
        ConversionKind::Task(_) => Some(BlockKind::TaskList),
        _ => None,
    };
    match kind {
        ConversionKind::Heading(level) => {
            let block = tree::block_mut(blocks, &path)?;
            let cells = flat::of_block(block)?;
            let mut heading = Block::Heading {
                level,
                inlines: Vec::new(),
            };
            flat::set_block(&mut heading, &cells);
            *block = heading;
            Outcome::at(here.head.clone(), Join::New)
        }
        ConversionKind::Quote => blocks::wrap_in(blocks, here, BlockKind::Quote),
        ConversionKind::List(_) | ConversionKind::Task(_) => {
            let wanted = as_list(kind.clone())?;
            if let Some((list, _)) = lists::enclosing_item(blocks, &path)
                && blocks::is_kind(block_at(blocks, &list)?, wanted)
            {
                return None;
            }
            let outcome = blocks::wrap_in(blocks, here, wanted)?;
            let (list, item) = lists::enclosing_item(blocks, &outcome.selection.head.path)?;
            let Block::List { kind: made, items } = tree::block_mut(blocks, &list)? else {
                return None;
            };
            match kind {
                ConversionKind::List(ordered @ ListKind::Ordered { .. }) => *made = ordered,
                ConversionKind::Task(task) => items[item].task = Some(task),
                _ => {}
            }
            Some(outcome)
        }
        ConversionKind::Rule => blocks::insert_rule(blocks, here),
        ConversionKind::CodeBlock(language) => {
            let block = tree::block_mut(blocks, &path)?;
            let text = flat::plain_text(&flat::of_block(block)?);
            *block = Block::Code { language, text };
            Outcome::at(DocPosition::new(path, 0), Join::New)
        }
        ConversionKind::Setext(level) => {
            let (parent, index) = tree::split(&path)?;
            let parent = parent.to_vec();
            let block = tree::block_mut(blocks, &path)?;
            let cells = flat::of_block(block)?;
            let mut heading = Block::Heading {
                level,
                inlines: Vec::new(),
            };
            flat::set_block(&mut heading, &cells);
            *block = heading;
            tree::container_mut(blocks, &parent)?.insert(index + 1, Block::Paragraph(Vec::new()));
            Outcome::at(
                DocPosition::new(tree::join(&parent, index + 1), 0),
                Join::New,
            )
        }
        ConversionKind::Wrap { .. } | ConversionKind::Link { .. } => None,
    }
}
