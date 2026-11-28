//! Walking a document by `DocPosition` paths: a list adds an item and a block index, a quote one
//! child index. A container path names a `Vec<Block>`; a block path is a container path plus one.

use crate::document::{Block, ListItem};
use crate::edit_ops::flat::{self, Cell};
use crate::position::block_at;

pub(crate) fn container<'a>(blocks: &'a [Block], path: &[u32]) -> Option<&'a [Block]> {
    let Some((&first, rest)) = path.split_first() else {
        return Some(blocks);
    };
    match blocks.get(first as usize)? {
        Block::Quote(children) => container(children, rest),
        Block::List { items, .. } => {
            let (&item, rest) = rest.split_first()?;
            container(&items.get(item as usize)?.blocks, rest)
        }
        _ => None,
    }
}

pub(crate) fn container_mut<'a>(
    blocks: &'a mut Vec<Block>,
    path: &[u32],
) -> Option<&'a mut Vec<Block>> {
    let Some((&first, rest)) = path.split_first() else {
        return Some(blocks);
    };
    match blocks.get_mut(first as usize)? {
        Block::Quote(children) => container_mut(children, rest),
        Block::List { items, .. } => {
            let (&item, rest) = rest.split_first()?;
            container_mut(&mut items.get_mut(item as usize)?.blocks, rest)
        }
        _ => None,
    }
}

pub(crate) fn block_mut<'a>(blocks: &'a mut Vec<Block>, path: &[u32]) -> Option<&'a mut Block> {
    let (parent, index) = split(path)?;
    container_mut(blocks, parent)?.get_mut(index)
}

/// The container path and the index in it.
pub(crate) fn split(path: &[u32]) -> Option<(&[u32], usize)> {
    let (&last, parent) = path.split_last()?;
    Some((parent, last as usize))
}

pub(crate) fn join(parent: &[u32], index: usize) -> Vec<u32> {
    let mut path = parent.to_vec();
    path.push(index as u32);
    path
}

/// What holds a container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Top,
    Quote(Vec<u32>),
    Item { list: Vec<u32>, item: usize },
}

pub(crate) fn owner(blocks: &[Block], container: &[u32]) -> Option<Owner> {
    if container.is_empty() {
        return Some(Owner::Top);
    }
    if let Some(Block::Quote(_)) = block_at(blocks, container) {
        return Some(Owner::Quote(container.to_vec()));
    }
    let (list, item) = split(container)?;
    match block_at(blocks, list)? {
        Block::List { .. } => Some(Owner::Item {
            list: list.to_vec(),
            item,
        }),
        _ => None,
    }
}

pub(crate) fn items_mut<'a>(
    blocks: &'a mut Vec<Block>,
    list: &[u32],
) -> Option<&'a mut Vec<ListItem>> {
    match block_mut(blocks, list)? {
        Block::List { items, .. } => Some(items),
        _ => None,
    }
}

pub(crate) fn is_leaf(block: &Block) -> bool {
    matches!(
        block,
        Block::Paragraph(_) | Block::Heading { .. } | Block::Code { .. }
    )
}

pub(crate) fn is_atomic(block: &Block) -> bool {
    matches!(block, Block::Rule | Block::Quoted(_) | Block::Table(_))
}

pub(crate) fn cells(blocks: &[Block], path: &[u32]) -> Option<Vec<Cell>> {
    flat::of_block(block_at(blocks, path)?)
}

/// Runs `change` on the leaf's cells and writes them back; `None` when `path` is not a leaf.
pub(crate) fn edit_cells<R>(
    blocks: &mut Vec<Block>,
    path: &[u32],
    change: impl FnOnce(&mut Vec<Cell>) -> R,
) -> Option<R> {
    let block = block_mut(blocks, path)?;
    let mut cells = flat::of_block(block)?;
    let out = change(&mut cells);
    flat::set_block(block, &cells);
    Some(out)
}

/// The first unit a caret reaches entering `path` from the front: a leaf or an atomic block.
pub(crate) fn first_unit(blocks: &[Block], path: Vec<u32>) -> Option<Vec<u32>> {
    let mut path = path;
    loop {
        match block_at(blocks, &path)? {
            Block::Quote(children) if !children.is_empty() => path.push(0),
            Block::List { items, .. } if !items.is_empty() => path.extend([0, 0]),
            Block::Quote(_) | Block::List { .. } => return None,
            _ => return Some(path),
        }
    }
}

pub(crate) fn last_unit(blocks: &[Block], path: Vec<u32>) -> Option<Vec<u32>> {
    let mut path = path;
    loop {
        match block_at(blocks, &path)? {
            Block::Quote(children) if !children.is_empty() => path.push(children.len() as u32 - 1),
            Block::List { items, .. } if !items.is_empty() => {
                let item = items.len() - 1;
                let last = items[item].blocks.len().saturating_sub(1);
                path.extend([item as u32, last as u32]);
            }
            Block::Quote(_) | Block::List { .. } => return None,
            _ => return Some(path),
        }
    }
}

/// Removes the block and every container left empty by it, so a list never holds an empty item.
pub(crate) fn remove_block(blocks: &mut Vec<Block>, path: &[u32]) -> Option<Block> {
    let (parent, index) = split(path)?;
    let siblings = container_mut(blocks, parent)?;
    if index >= siblings.len() {
        return None;
    }
    let removed = siblings.remove(index);
    prune_empty(blocks, parent);
    Some(removed)
}

pub(crate) fn prune_empty(blocks: &mut Vec<Block>, container_path: &[u32]) {
    let mut path = container_path.to_vec();
    while !path.is_empty() && container(blocks, &path).is_some_and(<[Block]>::is_empty) {
        match owner(blocks, &path) {
            Some(Owner::Quote(quote)) => {
                remove_at(blocks, &quote);
                path.pop();
            }
            Some(Owner::Item { list, item }) => {
                let Some(items) = items_mut(blocks, &list) else {
                    return;
                };
                items.remove(item);
                if !items.is_empty() {
                    return;
                }
                remove_at(blocks, &list);
                path = list;
                path.pop();
            }
            Some(Owner::Top) | None => return,
        }
    }
}

fn remove_at(blocks: &mut Vec<Block>, path: &[u32]) {
    if let Some((parent, index)) = split(path)
        && let Some(siblings) = container_mut(blocks, parent)
        && index < siblings.len()
    {
        siblings.remove(index);
    }
}
