use std::rc::Rc;

use super::flat::{Atom, Cell, MarkSet};
use super::text::{collapse, typing_marks};
use super::tree;
use super::{Join, Mark, Outcome};
use crate::document::{Block, ContentId, Document, Href};
use crate::position::{DocPosition, Selection, block_at};

pub(super) fn toggle(
    blocks: &mut Vec<Block>,
    selection: &Selection,
    mark: Mark,
) -> Option<Outcome> {
    if selection.is_collapsed() {
        let at = &selection.head;
        let cells = tree::cells(blocks, &at.path)?;
        let current = selection
            .marks
            .clone()
            .unwrap_or_else(|| MarkSet::clone(&typing_marks(&cells, at.offset as usize)));
        let on = !has(&current, mark);
        return Some(Outcome {
            selection: Selection {
                marks: Some(with(current, mark, on)),
                ..selection.clone()
            },
            join: Join::New,
        });
    }
    let spans = covered(blocks, selection);
    let mut chars = spans.iter().flat_map(|(path, from, to)| {
        let cells = tree::cells(blocks, path).unwrap_or_default();
        cells[*from..*to]
            .iter()
            .filter(|cell| matches!(cell.atom, Atom::Char(_)))
            .map(|cell| has(&cell.marks, mark))
            .collect::<Vec<_>>()
    });
    let first = chars.next()?;
    let on = !(first && chars.all(|marked| marked));
    for (path, from, to) in &spans {
        tree::edit_cells(blocks, path, |cells| {
            for cell in &mut cells[*from..*to] {
                if mark != Mark::Code || matches!(cell.atom, Atom::Char(_)) {
                    cell.marks = Rc::new(with(MarkSet::clone(&cell.marks), mark, on));
                }
            }
        });
    }
    Some(Outcome {
        selection: Selection::range(selection.anchor.clone(), selection.head.clone()),
        join: Join::New,
    })
}

pub(super) fn set_link(
    blocks: &mut Vec<Block>,
    selection: &Selection,
    href: Option<Href>,
) -> Option<Outcome> {
    if !selection.is_collapsed() {
        for (path, from, to) in covered(blocks, selection) {
            tree::edit_cells(blocks, &path, |cells| {
                for cell in &mut cells[from..to] {
                    cell.marks = Rc::new(MarkSet {
                        link: href.clone(),
                        ..MarkSet::clone(&cell.marks)
                    });
                }
            });
        }
        return Some(Outcome {
            selection: Selection::range(selection.anchor.clone(), selection.head.clone()),
            join: Join::New,
        });
    }
    let at = selection.head.clone();
    let cells = tree::cells(blocks, &at.path)?;
    if let Some((from, to)) = link_around(&cells, at.offset as usize) {
        tree::edit_cells(blocks, &at.path, |cells| {
            for cell in &mut cells[from..to] {
                cell.marks = Rc::new(MarkSet {
                    link: href.clone(),
                    ..MarkSet::clone(&cell.marks)
                });
            }
        });
        return Outcome::at(at, Join::New);
    }
    let href = href?;
    if matches!(block_at(blocks, &at.path)?, Block::Code { .. }) {
        return None;
    }
    let marks = Rc::new(MarkSet {
        link: Some(href.clone()),
        code: false,
        ..MarkSet::clone(&typing_marks(&cells, at.offset as usize))
    });
    let added: Vec<Cell> = href
        .as_str()
        .chars()
        .map(|c| Cell {
            atom: Atom::Char(c),
            marks: marks.clone(),
        })
        .collect();
    let len = added.len() as u32;
    let offset = at.offset as usize;
    tree::edit_cells(blocks, &at.path, |cells| {
        cells.splice(offset..offset, added);
    })?;
    Outcome::at(DocPosition::new(at.path, at.offset + len), Join::New)
}

pub(super) fn insert_image(
    blocks: &mut Vec<Block>,
    selection: &Selection,
    content_id: ContentId,
    alt: String,
) -> Option<Outcome> {
    let start = selection.start();
    if matches!(block_at(blocks, &start.path)?, Block::Code { .. }) {
        return None;
    }
    let at = collapse(blocks, selection);
    let offset = at.offset as usize;
    tree::edit_cells(blocks, &at.path, |cells| {
        let marks = Rc::new(MarkSet {
            code: false,
            ..MarkSet::clone(&typing_marks(cells, offset))
        });
        cells.insert(
            offset,
            Cell {
                atom: Atom::Image { content_id, alt },
                marks,
            },
        );
    })?;
    Outcome::at(DocPosition::new(at.path, at.offset + 1), Join::New)
}

fn covered(blocks: &[Block], selection: &Selection) -> Vec<(Vec<u32>, usize, usize)> {
    let (start, end) = (selection.start(), selection.end());
    tree::leaves(blocks, start.top()..end.top() + 1)
        .into_iter()
        .filter(|path| *path >= start.path && *path <= end.path)
        .filter_map(|path| {
            let block = block_at(blocks, &path)?;
            if matches!(block, Block::Code { .. }) {
                return None;
            }
            let len = crate::position::text_len(block)? as usize;
            let from = if path == start.path {
                start.offset as usize
            } else {
                0
            };
            let to = if path == end.path {
                end.offset as usize
            } else {
                len
            };
            (from < to).then_some((path, from, to))
        })
        .collect()
}

fn link_around(cells: &[Cell], offset: usize) -> Option<(usize, usize)> {
    let at = [offset.checked_sub(1), Some(offset)]
        .into_iter()
        .flatten()
        .find(|&i| cells.get(i).is_some_and(|cell| cell.marks.link.is_some()))?;
    let link = cells[at].marks.link.clone();
    let from = cells[..at]
        .iter()
        .rposition(|cell| cell.marks.link != link)
        .map_or(0, |i| i + 1);
    let to = cells[at..]
        .iter()
        .position(|cell| cell.marks.link != link)
        .map_or(cells.len(), |i| at + i);
    Some((from, to))
}

fn has(marks: &MarkSet, mark: Mark) -> bool {
    match mark {
        Mark::Strong => marks.strong,
        Mark::Emphasis => marks.emphasis,
        Mark::Strikethrough => marks.strikethrough,
        Mark::Code => marks.code,
    }
}

pub(super) fn with(mut marks: MarkSet, mark: Mark, on: bool) -> MarkSet {
    match mark {
        Mark::Strong => marks.strong = on,
        Mark::Emphasis => marks.emphasis = on,
        Mark::Strikethrough => marks.strikethrough = on,
        Mark::Code => marks.code = on,
    }
    marks
}

/// The marks a toolbar shows: what the next typed text gets at a caret, and what every character
/// of a range has.
pub fn marks_in_force(document: &Document, selection: &Selection) -> MarkSet {
    if selection.is_collapsed() {
        if let Some(marks) = &selection.marks {
            return marks.clone();
        }
        return tree::cells(&document.blocks, &selection.head.path)
            .map(|cells| MarkSet::clone(&typing_marks(&cells, selection.head.offset as usize)))
            .unwrap_or_default();
    }
    let mut chars = covered(&document.blocks, selection)
        .into_iter()
        .flat_map(|(path, from, to)| {
            tree::cells(&document.blocks, &path).unwrap_or_default()[from..to].to_vec()
        })
        .filter(|cell| matches!(cell.atom, Atom::Char(_)));
    let Some(first) = chars.next() else {
        return MarkSet::default();
    };
    chars.fold(MarkSet::clone(&first.marks), |common, cell| MarkSet {
        strong: common.strong && cell.marks.strong,
        emphasis: common.emphasis && cell.marks.emphasis,
        strikethrough: common.strikethrough && cell.marks.strikethrough,
        code: common.code && cell.marks.code,
        link: common
            .link
            .filter(|link| cell.marks.link.as_ref() == Some(link)),
    })
}
