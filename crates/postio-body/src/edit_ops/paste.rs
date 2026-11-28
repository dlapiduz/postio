//! Paste and drop: a document already narrowed by `parse`, spliced at the caret. Inline content
//! joins the caret's block; blocks go between its halves.

use super::flat::{self, Atom, Cell};
use super::text::{collapse, split_off};
use super::tree;
use super::{Join, Outcome};
use crate::document::{Block, Document};
use crate::position::{DocPosition, Selection, block_at, text_len};

pub(super) fn paste(
    blocks: &mut Vec<Block>,
    selection: &Selection,
    pasted: Document,
) -> Option<Outcome> {
    let mut pasted = pasted.blocks;
    if pasted.is_empty() {
        return None;
    }
    let at = collapse(blocks, selection);
    let offset = at.offset as usize;
    if matches!(block_at(blocks, &at.path)?, Block::Code { .. }) {
        let text = Document { blocks: pasted }.to_text();
        return splice(
            blocks,
            at,
            text.chars()
                .map(|c| Cell {
                    atom: if c == '\n' {
                        Atom::Break
                    } else {
                        Atom::Char(c)
                    },
                    marks: Default::default(),
                })
                .collect(),
        );
    }
    if let [Block::Paragraph(inlines)] = pasted.as_slice() {
        let cells = flat::flatten(inlines);
        return splice(blocks, at, cells);
    }
    let (parent, index) = tree::split(&at.path)?;
    let parent = parent.to_vec();
    let right = split_off(blocks, &at)?;
    let mut first = index + 1;
    if let Some(Block::Paragraph(first)) = pasted.first() {
        let cells = flat::flatten(first);
        tree::edit_cells(blocks, &at.path, |left| left.extend(cells))?;
        pasted.remove(0);
    } else if offset == 0 && text_len(block_at(blocks, &at.path)?) == Some(0) {
        tree::container_mut(blocks, &parent)?.remove(index);
        first = index;
    }
    let right_cells = flat::of_block(&right)?;
    let landed = match pasted.last_mut() {
        Some(last @ Block::Paragraph(_)) => {
            let mut cells = flat::of_block(last)?;
            let end = cells.len() as u32;
            cells.extend(right_cells);
            flat::set_block(last, &cells);
            Some(end)
        }
        _ => {
            pasted.push(right);
            None
        }
    };
    let count = pasted.len();
    let siblings = tree::container_mut(blocks, &parent)?;
    for (step, block) in pasted.into_iter().enumerate() {
        siblings.insert(first + step, block);
    }
    let last = tree::join(&parent, first + count - 1);
    Outcome::at(DocPosition::new(last, landed.unwrap_or(0)), Join::New)
}

fn splice(blocks: &mut Vec<Block>, at: DocPosition, cells: Vec<Cell>) -> Option<Outcome> {
    let added = cells.len() as u32;
    let offset = at.offset as usize;
    tree::edit_cells(blocks, &at.path, |into| {
        into.splice(offset..offset, cells);
    })?;
    Outcome::at(DocPosition::new(at.path, at.offset + added), Join::New)
}
