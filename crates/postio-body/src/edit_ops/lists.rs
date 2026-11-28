//! Moving list items and quote children between containers: out of a list, out a level, out of a
//! quote. Each returns the path of the first text the move leaves the caret on.

use super::tree;
use crate::document::{Block, ListItem};

pub(super) fn is_nested(blocks: &[Block], list: &[u32]) -> bool {
    tree::split(list)
        .and_then(|(parent, _)| tree::owner(blocks, parent))
        .is_some_and(|owner| matches!(owner, tree::Owner::Item { .. }))
}

/// Items `from..=to` leave their list as plain blocks; the items after them stay a list.
pub(super) fn lift_items(
    blocks: &mut Vec<Block>,
    list: &[u32],
    from: usize,
    to: usize,
) -> Option<Vec<u32>> {
    let (parent, index) = tree::split(list)?;
    let Block::List { kind, items } = tree::block_mut(blocks, list)? else {
        return None;
    };
    let kind = *kind;
    if to >= items.len() || from > to {
        return None;
    }
    let after: Vec<ListItem> = items.drain(to + 1..).collect();
    let lifted: Vec<Block> = items.drain(from..).flat_map(|item| item.blocks).collect();
    let emptied = items.is_empty();
    let siblings = tree::container_mut(blocks, parent)?;
    let mut at = if emptied {
        siblings.remove(index);
        index
    } else {
        index + 1
    };
    let first = at;
    for block in lifted {
        siblings.insert(at, block);
        at += 1;
    }
    if !after.is_empty() {
        siblings.insert(at, Block::List { kind, items: after });
    }
    tree::first_unit(blocks, tree::join(parent, first))
}

/// A nested item moves to its parent's list, after the item that held it; the items after it in
/// its list become its own nested list.
pub(super) fn outdent_item(blocks: &mut Vec<Block>, list: &[u32], item: usize) -> Option<Vec<u32>> {
    let (holder, index) = tree::split(list)?;
    let (outer, outer_item) = tree::split(holder)?;
    let (outer, holder) = (outer.to_vec(), holder.to_vec());
    let Block::List { kind, items } = tree::block_mut(blocks, list)? else {
        return None;
    };
    let kind = *kind;
    let mut moved = items.remove(item);
    let after: Vec<ListItem> = items.drain(item..).collect();
    if !after.is_empty() {
        moved.blocks.push(Block::List { kind, items: after });
    }
    if items.is_empty() {
        tree::container_mut(blocks, &holder)?.remove(index);
    }
    let outer_items = tree::items_mut(blocks, &outer)?;
    outer_items.insert(outer_item + 1, moved);
    let mut landed = outer_item + 1;
    if outer_items[outer_item].blocks.is_empty() {
        outer_items.remove(outer_item);
        landed -= 1;
    }
    let mut path = outer;
    path.extend([landed as u32, 0]);
    tree::first_unit(blocks, path)
}

/// An item moves under the item before it, joining that item's nested list of the same kind when
/// it ends with one. The first item has nothing to move under.
pub(super) fn indent_item(blocks: &mut Vec<Block>, list: &[u32], item: usize) -> Option<Vec<u32>> {
    if item == 0 {
        return None;
    }
    let Block::List { kind, items } = tree::block_mut(blocks, list)? else {
        return None;
    };
    let kind = *kind;
    let moved = items.remove(item);
    let under = &mut items[item - 1];
    let mut path = list.to_vec();
    path.push(item as u32 - 1);
    let held = under.blocks.len() as u32;
    match under.blocks.last_mut() {
        Some(Block::List {
            kind: nested,
            items: nested_items,
        }) if nested.is_ordered() == kind.is_ordered() => {
            path.extend([held - 1, nested_items.len() as u32, 0]);
            nested_items.push(moved);
        }
        _ => {
            let nested = match kind {
                crate::document::ListKind::Ordered { .. } => crate::document::ListKind::NUMBERED,
                bullet => bullet,
            };
            path.extend([held, 0, 0]);
            under.blocks.push(Block::List {
                kind: nested,
                items: vec![moved],
            });
        }
    }
    tree::first_unit(blocks, path)
}

/// A child leaves its quote: before it when first, after it when last, else the quote splits.
pub(super) fn leave_quote(
    blocks: &mut Vec<Block>,
    quote: &[u32],
    child: usize,
) -> Option<Vec<u32>> {
    lift_children(blocks, quote, child, child)
}

pub(super) fn lift_children(
    blocks: &mut Vec<Block>,
    quote: &[u32],
    from: usize,
    to: usize,
) -> Option<Vec<u32>> {
    let (parent, index) = tree::split(quote)?;
    let Block::Quote(children) = tree::block_mut(blocks, quote)? else {
        return None;
    };
    if to >= children.len() || from > to {
        return None;
    }
    let after: Vec<Block> = children.drain(to + 1..).collect();
    let lifted: Vec<Block> = children.drain(from..).collect();
    let emptied = children.is_empty();
    let siblings = tree::container_mut(blocks, parent)?;
    let mut at = if emptied {
        siblings.remove(index);
        index
    } else {
        index + 1
    };
    let first = at;
    for block in lifted {
        siblings.insert(at, block);
        at += 1;
    }
    if !after.is_empty() {
        siblings.insert(at, Block::Quote(after));
    }
    tree::first_unit(blocks, tree::join(parent, first))
}

/// The list and item holding the block at `path`, nearest first.
pub(super) fn enclosing_item(blocks: &[Block], path: &[u32]) -> Option<(Vec<u32>, usize)> {
    let mut at = tree::split(path)?.0.to_vec();
    loop {
        match tree::owner(blocks, &at)? {
            tree::Owner::Item { list, item } => return Some((list, item)),
            tree::Owner::Quote(quote) => at = tree::split(&quote)?.0.to_vec(),
            tree::Owner::Top => return None,
        }
    }
}
