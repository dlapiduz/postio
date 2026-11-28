//! Block kinds at the caret's block or each block of the selection: headings, code, lists and
//! quotes toggled on and off, list items moved a level, and a rule put in.

use super::flat::Atom;
use super::lists;
use super::text::{collapse, split_off};
use super::tree::{self, Owner};
use super::{BlockKind, Join, Outcome};
use crate::document::{Block, ListItem, ListKind, Task};
use crate::position::{DocPosition, Selection, block_at, text_len};

pub(super) fn toggle(
    blocks: &mut Vec<Block>,
    selection: &Selection,
    kind: BlockKind,
) -> Option<Outcome> {
    match kind {
        BlockKind::Heading(_) | BlockKind::CodeBlock => retype(blocks, selection, kind),
        BlockKind::BulletList | BlockKind::NumberedList | BlockKind::TaskList => {
            toggle_list(blocks, selection, kind)
        }
        BlockKind::Quote => toggle_quote(blocks, selection),
    }
}

/// Every leaf of the selection becomes `kind`, or a paragraph when all of them already are.
fn retype(blocks: &mut Vec<Block>, selection: &Selection, kind: BlockKind) -> Option<Outcome> {
    let (start, end) = (selection.start(), selection.end());
    let leaves: Vec<Vec<u32>> = tree::leaves(blocks, start.top()..end.top() + 1)
        .into_iter()
        .filter(|path| *path >= start.path && *path <= end.path)
        .collect();
    let is = |block: &Block| match (block, kind) {
        (Block::Heading { level, .. }, BlockKind::Heading(wanted)) => *level == wanted,
        (Block::Code { .. }, BlockKind::CodeBlock) => true,
        _ => false,
    };
    let off = leaves
        .iter()
        .all(|path| block_at(blocks, path).is_some_and(is));
    for path in &leaves {
        let block = tree::block_mut(blocks, path)?;
        let mut cells = super::flat::of_block(block)?;
        let mut next = match (off, kind) {
            (true, _) => Block::Paragraph(Vec::new()),
            (false, BlockKind::Heading(level)) => Block::Heading {
                level,
                inlines: Vec::new(),
            },
            (false, _) => Block::Code {
                language: None,
                text: String::new(),
            },
        };
        if matches!(block, Block::Code { .. }) {
            for cell in &mut cells {
                if cell.atom == Atom::Char('\n') {
                    cell.atom = Atom::Break;
                }
            }
        }
        super::flat::set_block(&mut next, &cells);
        *block = next;
    }
    let clamp = |at: &DocPosition| {
        let len = block_at(blocks, &at.path).and_then(text_len).unwrap_or(0);
        DocPosition::new(at.path.clone(), at.offset.min(len))
    };
    Some(Outcome {
        selection: Selection::range(clamp(&selection.anchor), clamp(&selection.head)),
        join: Join::New,
    })
}

fn list_kind(kind: BlockKind) -> ListKind {
    match kind {
        BlockKind::NumberedList => ListKind::NUMBERED,
        _ => ListKind::Bullet,
    }
}

pub(super) fn is_kind(list: &Block, kind: BlockKind) -> bool {
    let Block::List { kind: have, items } = list else {
        return false;
    };
    let tasks = items.iter().all(|item| item.task.is_some());
    match kind {
        BlockKind::NumberedList => have.is_ordered(),
        BlockKind::TaskList => !have.is_ordered() && tasks,
        _ => !have.is_ordered() && !tasks,
    }
}

fn toggle_list(blocks: &mut Vec<Block>, selection: &Selection, kind: BlockKind) -> Option<Outcome> {
    let (start, end) = (selection.start().clone(), selection.end().clone());
    if let Some((list, item)) = lists::enclosing_item(blocks, &start.path) {
        if is_kind(block_at(blocks, &list)?, kind) {
            let last = match lists::enclosing_item(blocks, &end.path) {
                Some((same, last)) if same == list => last,
                _ => item,
            };
            let first_block = start.path.len() == list.len() + 2;
            let landed = lists::lift_items(blocks, &list, item, last)?;
            let offset = if first_block { start.offset } else { 0 };
            return Outcome::at(DocPosition::new(landed, offset), Join::New);
        }
        let Block::List { kind: have, items } = tree::block_mut(blocks, &list)? else {
            return None;
        };
        *have = list_kind(kind);
        for held in items.iter_mut() {
            held.task = match kind {
                BlockKind::TaskList => held.task.or(Some(Task::Open)),
                _ => None,
            };
        }
        return Some(keep(selection));
    }
    wrap_in(blocks, selection, kind)
}

/// Wraps the selection's blocks in a list or a quote, whatever holds them now.
pub(super) fn wrap_in(
    blocks: &mut Vec<Block>,
    selection: &Selection,
    kind: BlockKind,
) -> Option<Outcome> {
    if kind == BlockKind::Quote {
        return wrap(blocks, selection, Block::Quote, |offset| vec![offset]);
    }
    let task = (kind == BlockKind::TaskList).then_some(Task::Open);
    wrap(
        blocks,
        selection,
        |taken| Block::List {
            kind: list_kind(kind),
            items: taken
                .into_iter()
                .map(|block| ListItem {
                    task,
                    blocks: vec![block],
                })
                .collect(),
        },
        |offset| vec![offset, 0],
    )
}

fn toggle_quote(blocks: &mut Vec<Block>, selection: &Selection) -> Option<Outcome> {
    let (start, end) = (selection.start().clone(), selection.end().clone());
    let (parent, child) = tree::split(&start.path)?;
    if let Some(Owner::Quote(quote)) = tree::owner(blocks, parent) {
        let last = match tree::split(&end.path) {
            Some((same, last)) if same == quote.as_slice() => last,
            _ => child,
        };
        let landed = lists::lift_children(blocks, &quote, child, last)?;
        return Outcome::at(DocPosition::new(landed, start.offset), Join::New);
    }
    wrap_in(blocks, selection, BlockKind::Quote)
}

/// Replaces the selection's sibling blocks in the caret's container with one block built from
/// them; `inner` is the path step from the new block to the block that was at `offset` in it.
fn wrap(
    blocks: &mut Vec<Block>,
    selection: &Selection,
    build: impl FnOnce(Vec<Block>) -> Block,
    inner: impl Fn(u32) -> Vec<u32>,
) -> Option<Outcome> {
    let (start, end) = (selection.start(), selection.end());
    let (parent, first) = tree::split(&start.path)?;
    let parent = parent.to_vec();
    let last = if end.path.len() > parent.len() && end.path.starts_with(&parent) {
        end.path[parent.len()] as usize
    } else {
        first
    };
    let siblings = tree::container_mut(blocks, &parent)?;
    let taken: Vec<Block> = siblings.drain(first..=last).collect();
    siblings.insert(first, build(taken));
    let moved = |at: &DocPosition| {
        if !at.path.starts_with(&parent) || at.path.len() <= parent.len() {
            return at.clone();
        }
        let index = at.path[parent.len()];
        let mut path = parent.clone();
        path.push(first as u32);
        path.extend(inner(index - first as u32));
        path.extend_from_slice(&at.path[parent.len() + 1..]);
        DocPosition::new(path, at.offset)
    };
    Some(Outcome {
        selection: Selection::range(moved(&selection.anchor), moved(&selection.head)),
        join: Join::New,
    })
}

fn keep(selection: &Selection) -> Outcome {
    Outcome {
        selection: Selection::range(selection.anchor.clone(), selection.head.clone()),
        join: Join::New,
    }
}

pub(super) fn indent(blocks: &mut Vec<Block>, selection: &Selection) -> Option<Outcome> {
    let start = selection.start();
    let (list, item) = lists::enclosing_item(blocks, &start.path)?;
    let tail = start.path[list.len() + 1..].to_vec();
    let mut path = lists::indent_item(blocks, &list, item)?;
    path.extend(tail);
    Outcome::at(DocPosition::new(path, start.offset), Join::New)
}

pub(super) fn outdent(blocks: &mut Vec<Block>, selection: &Selection) -> Option<Outcome> {
    let start = selection.start();
    let (list, item) = lists::enclosing_item(blocks, &start.path)?;
    if lists::is_nested(blocks, &list) {
        let mut path = lists::outdent_item(blocks, &list, item)?;
        path.extend_from_slice(&start.path[list.len() + 1..]);
        return Outcome::at(DocPosition::new(path, start.offset), Join::New);
    }
    let first_block = start.path.len() == list.len() + 2;
    let landed = lists::lift_items(blocks, &list, item, item)?;
    let offset = if first_block { start.offset } else { 0 };
    Outcome::at(DocPosition::new(landed, offset), Join::New)
}

/// A rule after the caret's text, with a block after it for the caret; an empty block becomes the
/// rule.
pub(super) fn insert_rule(blocks: &mut Vec<Block>, selection: &Selection) -> Option<Outcome> {
    let at = collapse(blocks, selection);
    let (parent, index) = tree::split(&at.path)?;
    let parent = parent.to_vec();
    let block = block_at(blocks, &at.path)?;
    if matches!(block, Block::Code { .. }) {
        return None;
    }
    if text_len(block)? == 0 {
        let siblings = tree::container_mut(blocks, &parent)?;
        siblings[index] = Block::Rule;
        if !siblings.get(index + 1).is_some_and(tree::is_leaf) {
            siblings.insert(index + 1, Block::Paragraph(Vec::new()));
        }
        let after = tree::join(&parent, index + 1);
        return Outcome::at(DocPosition::new(after, 0), Join::New);
    }
    let right = split_off(blocks, &at)?;
    let siblings = tree::container_mut(blocks, &parent)?;
    siblings.insert(index + 1, Block::Rule);
    siblings.insert(index + 2, right);
    Outcome::at(
        DocPosition::new(tree::join(&parent, index + 2), 0),
        Join::New,
    )
}
