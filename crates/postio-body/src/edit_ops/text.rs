//! Typing, deleting and splitting: the intents a keystroke makes inside or between blocks.

use std::rc::Rc;

use super::flat::{Atom, Cell, MarkSet};
use super::tree::{self, Owner};
use super::{Join, Outcome, Unit, lists};
use crate::document::Block;
use crate::position::{DocPosition, Selection, block_at, text_len};

pub(super) fn insert(
    blocks: &mut Vec<Block>,
    selection: &Selection,
    typed: &str,
) -> Option<Outcome> {
    let typed: String = typed.chars().filter(|c| *c != '\r').collect();
    if typed.is_empty() {
        return None;
    }
    let join = if selection.is_collapsed() {
        Join::Run
    } else {
        Join::New
    };
    let at = collapse(blocks, selection);
    let offset = at.offset as usize;
    let stored = selection.marks.clone().map(Rc::new);
    let added = tree::edit_cells(blocks, &at.path, |cells| {
        let marks = stored.unwrap_or_else(|| typing_marks(cells, offset));
        let new: Vec<Cell> = typed
            .chars()
            .map(|c| Cell {
                atom: if c == '\n' {
                    Atom::Break
                } else {
                    Atom::Char(c)
                },
                marks: marks.clone(),
            })
            .collect();
        let added = new.len() as u32;
        cells.splice(offset..offset, new);
        added
    })?;
    Outcome::at(DocPosition::new(at.path, at.offset + added), join)
}

/// The left neighbour's marks continue; a link continues only between two of its characters.
pub(super) fn typing_marks(cells: &[Cell], offset: usize) -> Rc<MarkSet> {
    let right = cells.get(offset);
    match offset.checked_sub(1).and_then(|left| cells.get(left)) {
        Some(left)
            if left.marks.link.is_none()
                || right.is_some_and(|r| r.marks.link == left.marks.link) =>
        {
            left.marks.clone()
        }
        Some(left) => Rc::new(MarkSet {
            link: None,
            ..MarkSet::clone(&left.marks)
        }),
        None => Rc::new(MarkSet {
            link: None,
            ..right.map(|r| MarkSet::clone(&r.marks)).unwrap_or_default()
        }),
    }
}

/// Deletes a range selection; the caret is where it started.
pub(super) fn collapse(blocks: &mut Vec<Block>, selection: &Selection) -> DocPosition {
    let (start, end) = (selection.start().clone(), selection.end().clone());
    if start != end {
        delete_range(blocks, &start, &end);
    }
    start
}

/// Removes everything between the two positions and joins what is left of the end's block onto
/// the start's. A reply quote or a rule inside the range goes whole.
pub(super) fn delete_range(blocks: &mut Vec<Block>, start: &DocPosition, end: &DocPosition) {
    if start.path == end.path {
        tree::edit_cells(blocks, &start.path, |cells| {
            cells.drain(start.offset as usize..end.offset as usize);
        });
        return;
    }
    prune(
        blocks,
        Some((&start.path, start.offset)),
        Some((&end.path, end.offset)),
    );
    let shared = start
        .path
        .iter()
        .zip(&end.path)
        .take_while(|(a, b)| a == b)
        .count();
    let mut moved = start.path[..shared].to_vec();
    moved.push(start.path[shared] + 1);
    moved.extend(std::iter::repeat_n(0, end.path.len() - shared - 1));
    let tail = tree::cells(blocks, &moved).unwrap_or_default();
    tree::remove_block(blocks, &moved);
    tree::edit_cells(blocks, &start.path, |cells| cells.extend(tail));
}

type End<'a> = Option<(&'a [u32], u32)>;

fn deeper(end: End<'_>) -> End<'_> {
    end.map(|(path, offset)| (&path[1..], offset))
}

/// Keeps what is before `start` and after `end` in this container; `None` is unbounded.
fn prune(blocks: &mut Vec<Block>, start: End<'_>, end: End<'_>) {
    let lo = start
        .and_then(|(path, _)| path.first())
        .map(|&i| i as usize);
    let hi = end.and_then(|(path, _)| path.first()).map(|&i| i as usize);
    for index in (0..blocks.len()).rev() {
        let (at_start, at_end) = (lo == Some(index), hi == Some(index));
        if at_start || at_end {
            let inner_start = if at_start { deeper(start) } else { None };
            let inner_end = if at_end { deeper(end) } else { None };
            prune_block(&mut blocks[index], inner_start, inner_end);
        } else if lo.is_none_or(|lo| index > lo) && hi.is_none_or(|hi| index < hi) {
            blocks.remove(index);
        }
    }
}

fn prune_block(block: &mut Block, start: End<'_>, end: End<'_>) {
    match block {
        Block::Quote(children) => prune(children, start, end),
        Block::List { items, .. } => {
            let lo = start
                .and_then(|(path, _)| path.first())
                .map(|&i| i as usize);
            let hi = end.and_then(|(path, _)| path.first()).map(|&i| i as usize);
            for index in (0..items.len()).rev() {
                let (at_start, at_end) = (lo == Some(index), hi == Some(index));
                if at_start || at_end {
                    let inner_start = if at_start { deeper(start) } else { None };
                    let inner_end = if at_end { deeper(end) } else { None };
                    prune(&mut items[index].blocks, inner_start, inner_end);
                } else if lo.is_none_or(|lo| index > lo) && hi.is_none_or(|hi| index < hi) {
                    items.remove(index);
                }
            }
        }
        leaf => {
            let Some(mut cells) = super::flat::of_block(leaf) else {
                return;
            };
            match (start, end) {
                (Some((_, from)), None) => cells.truncate(from as usize),
                (None, Some((_, to))) => {
                    cells.drain(..to as usize);
                }
                (Some((_, from)), Some((_, to))) => {
                    cells.drain(from as usize..to as usize);
                }
                (None, None) => {}
            }
            super::flat::set_block(leaf, &cells);
        }
    }
}

pub(super) fn insert_break(blocks: &mut Vec<Block>, selection: &Selection) -> Option<Outcome> {
    let at = collapse(blocks, selection);
    let offset = at.offset as usize;
    tree::edit_cells(blocks, &at.path, |cells| {
        cells.insert(
            offset,
            Cell {
                atom: Atom::Break,
                marks: Rc::default(),
            },
        );
    })?;
    Outcome::at(DocPosition::new(at.path, at.offset + 1), Join::New)
}

pub(super) fn split_block(blocks: &mut Vec<Block>, selection: &Selection) -> Option<Outcome> {
    let at = collapse(blocks, selection);
    let (parent, index) = tree::split(&at.path)?;
    let parent = parent.to_vec();
    let block = block_at(blocks, &at.path)?;
    if matches!(block, Block::Code { .. }) {
        return insert_break(blocks, &Selection::caret(at));
    }
    let len = text_len(block)?;
    let siblings = tree::container(blocks, &parent)?.len();
    match tree::owner(blocks, &parent)? {
        Owner::Item { list, item } if index == 0 => {
            if len == 0 && siblings == 1 {
                let moved = if lists::is_nested(blocks, &list) {
                    let mut path = lists::outdent_item(blocks, &list, item)?;
                    path.push(0);
                    path
                } else {
                    lists::lift_items(blocks, &list, item, item, &[0])?
                };
                return Outcome::at(DocPosition::new(moved, 0), Join::New);
            }
            let right = split_off(blocks, &at)?;
            let items = tree::items_mut(blocks, &list)?;
            let old = &mut items[item];
            let mut moved = vec![right];
            moved.extend(old.blocks.drain(1..));
            let task = old.task.map(|_| crate::document::Task::Open);
            items.insert(
                item + 1,
                crate::document::ListItem {
                    task,
                    blocks: moved,
                },
            );
            let mut path = list;
            path.extend([item as u32 + 1, 0]);
            Outcome::at(DocPosition::new(path, 0), Join::New)
        }
        Owner::Quote(quote) if len == 0 => {
            let moved = lists::leave_quote(blocks, &quote, index)?;
            Outcome::at(DocPosition::new(moved, 0), Join::New)
        }
        _ => {
            let right = split_off(blocks, &at)?;
            tree::container_mut(blocks, &parent)?.insert(index + 1, right);
            Outcome::at(
                DocPosition::new(tree::join(&parent, index + 1), 0),
                Join::New,
            )
        }
    }
}

/// Cuts the leaf at the caret and returns the right half as a new block: a heading split inside
/// its text stays a heading, and the line after a heading's end is a paragraph.
pub(super) fn split_off(blocks: &mut Vec<Block>, at: &DocPosition) -> Option<Block> {
    let block = tree::block_mut(blocks, &at.path)?;
    let mut cells = super::flat::of_block(block)?;
    let right_cells = cells.split_off((at.offset as usize).min(cells.len()));
    let mut right = match block {
        Block::Heading { level, .. } if !right_cells.is_empty() => Block::Heading {
            level: *level,
            inlines: Vec::new(),
        },
        _ => Block::Paragraph(Vec::new()),
    };
    super::flat::set_block(block, &cells);
    super::flat::set_block(&mut right, &right_cells);
    Some(right)
}

pub(super) fn delete_backward(
    blocks: &mut Vec<Block>,
    selection: &Selection,
    unit: Unit,
) -> Option<Outcome> {
    if !selection.is_collapsed() {
        let at = collapse(blocks, selection);
        return Outcome::at(at, Join::New);
    }
    let at = selection.head.clone();
    if at.offset == 0 {
        return join_backward(blocks, &at);
    }
    let offset = at.offset as usize;
    let gone = tree::edit_cells(blocks, &at.path, |cells| {
        let from = match unit {
            Unit::Char => offset - 1,
            Unit::Word => word_start(cells, offset),
            Unit::Line => line_start(cells, offset),
        };
        cells.drain(from..offset);
        offset - from
    })?;
    let join = if unit == Unit::Char {
        Join::Run
    } else {
        Join::New
    };
    Outcome::at(DocPosition::new(at.path, at.offset - gone as u32), join)
}

pub(super) fn delete_forward(
    blocks: &mut Vec<Block>,
    selection: &Selection,
    unit: Unit,
) -> Option<Outcome> {
    if !selection.is_collapsed() {
        let at = collapse(blocks, selection);
        return Outcome::at(at, Join::New);
    }
    let at = selection.head.clone();
    let len = text_len(block_at(blocks, &at.path)?)?;
    if at.offset == len {
        return join_forward(blocks, &at);
    }
    let offset = at.offset as usize;
    tree::edit_cells(blocks, &at.path, |cells| {
        let to = match unit {
            Unit::Char => offset + 1,
            Unit::Word => word_end(cells, offset),
            Unit::Line => line_end(cells, offset),
        };
        cells.drain(offset..to);
    })?;
    let join = if unit == Unit::Char {
        Join::Run
    } else {
        Join::New
    };
    Outcome::at(at, join)
}

/// Backspace at the start of a block: merge into what precedes it, take a list item or a quote
/// child out of its container, or remove the rule or reply quote before it whole.
fn join_backward(blocks: &mut Vec<Block>, at: &DocPosition) -> Option<Outcome> {
    let (parent, index) = tree::split(&at.path)?;
    let parent = parent.to_vec();
    if index == 0 {
        return match tree::owner(blocks, &parent)? {
            Owner::Item { list, item: 0 } => {
                let moved = lists::lift_items(blocks, &list, 0, 0, &[0])?;
                Outcome::at(DocPosition::new(moved, 0), Join::New)
            }
            Owner::Item { list, item } => merge_into_previous_item(blocks, &list, item),
            Owner::Quote(quote) => {
                let moved = lists::leave_quote(blocks, &quote, 0)?;
                Outcome::at(DocPosition::new(moved, 0), Join::New)
            }
            Owner::Top => None,
        };
    }
    let previous = tree::join(&parent, index - 1);
    let target = tree::last_unit(blocks, previous.clone())?;
    if !tree::is_leaf(block_at(blocks, &target)?) {
        let before = tree::container(blocks, &parent)?.len();
        tree::remove_block(blocks, &target);
        let after = tree::container(blocks, &parent)?.len();
        let index = index - (before - after);
        return Outcome::at(DocPosition::new(tree::join(&parent, index), 0), Join::New);
    }
    let joined = text_len(block_at(blocks, &target)?)?;
    let moved = tree::cells(blocks, &at.path)?;
    tree::remove_block(blocks, &at.path);
    tree::edit_cells(blocks, &target, |cells| cells.extend(moved))?;
    Outcome::at(DocPosition::new(target, joined), Join::New)
}

fn merge_into_previous_item(blocks: &mut Vec<Block>, list: &[u32], item: usize) -> Option<Outcome> {
    let mut previous = list.to_vec();
    let items = tree::items_mut(blocks, list)?;
    previous.extend([item as u32 - 1, items[item - 1].blocks.len() as u32 - 1]);
    let target = tree::last_unit(blocks, previous)?;
    if !tree::is_leaf(block_at(blocks, &target)?) {
        return None;
    }
    let joined = text_len(block_at(blocks, &target)?)?;
    let items = tree::items_mut(blocks, list)?;
    let mut taken = items.remove(item).blocks;
    let first = taken.remove(0);
    items[item - 1].blocks.extend(taken);
    let moved = super::flat::of_block(&first)?;
    tree::edit_cells(blocks, &target, |cells| cells.extend(moved))?;
    Outcome::at(DocPosition::new(target, joined), Join::New)
}

/// Delete at the end of a block: the next block's text joins this one; a rule or reply quote after
/// it goes whole.
fn join_forward(blocks: &mut Vec<Block>, at: &DocPosition) -> Option<Outcome> {
    let (parent, index) = tree::split(&at.path)?;
    let parent = parent.to_vec();
    let siblings = tree::container(blocks, &parent)?.len();
    if index + 1 < siblings {
        let target = tree::first_unit(blocks, tree::join(&parent, index + 1))?;
        if tree::is_leaf(block_at(blocks, &target)?) {
            let moved = tree::cells(blocks, &target)?;
            tree::edit_cells(blocks, &at.path, |cells| cells.extend(moved))?;
        }
        tree::remove_block(blocks, &target);
        return Outcome::at(at.clone(), Join::New);
    }
    let Owner::Item { list, item } = tree::owner(blocks, &parent)? else {
        return None;
    };
    let items = tree::items_mut(blocks, &list)?;
    if item + 1 >= items.len() || !tree::is_leaf(items[item + 1].blocks.first()?) {
        return None;
    }
    let mut taken = items.remove(item + 1).blocks;
    let first = taken.remove(0);
    items[item].blocks.extend(taken);
    let moved = super::flat::of_block(&first)?;
    tree::edit_cells(blocks, &at.path, |cells| cells.extend(moved))?;
    Outcome::at(at.clone(), Join::New)
}

fn is_space(cell: &Cell) -> bool {
    matches!(cell.atom, Atom::Char(c) if c.is_whitespace())
}

fn is_break(cell: &Cell) -> bool {
    matches!(cell.atom, Atom::Break)
}

fn word_start(cells: &[Cell], offset: usize) -> usize {
    let mut at = offset;
    while at > 0 && is_space(&cells[at - 1]) && !is_break(&cells[at - 1]) {
        at -= 1;
    }
    while at > 0 && !is_space(&cells[at - 1]) && !is_break(&cells[at - 1]) {
        at -= 1;
    }
    at.min(offset - 1)
}

fn word_end(cells: &[Cell], offset: usize) -> usize {
    let mut at = offset;
    while at < cells.len() && is_space(&cells[at]) && !is_break(&cells[at]) {
        at += 1;
    }
    while at < cells.len() && !is_space(&cells[at]) && !is_break(&cells[at]) {
        at += 1;
    }
    at.max(offset + 1)
}

fn line_start(cells: &[Cell], offset: usize) -> usize {
    let start = cells[..offset]
        .iter()
        .rposition(is_break)
        .map_or(0, |i| i + 1);
    start.min(offset - 1)
}

fn line_end(cells: &[Cell], offset: usize) -> usize {
    let end = cells[offset..]
        .iter()
        .position(is_break)
        .map_or(cells.len(), |i| offset + i);
    end.max(offset + 1)
}
