//! A projection patch applied to the buffer with the buffer's own edit signals let through, so the
//! one write path is counted and nothing else may write.

use gtk::prelude::*;
use postio_body::Patch;

use super::{Inner, image, render};

/// Buffer operations and tag applications one patch made: bounded per keystroke (FR-014).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PatchStats {
    pub deleted: u32,
    pub inserted: u32,
    pub operations: u32,
    pub tags: u32,
}

pub(super) fn apply(inner: &Inner, patch: &Patch) -> PatchStats {
    let buffer = &inner.buffer;
    let mut stats = PatchStats::default();
    inner.patching.set(true);
    if patch.delete.start < patch.delete.end {
        let mut from = buffer.iter_at_offset(patch.delete.start as i32);
        let mut to = buffer.iter_at_offset(patch.delete.end as i32);
        buffer.delete(&mut from, &mut to);
        stats.deleted = patch.delete.end - patch.delete.start;
        stats.operations += 1;
    }
    if !patch.insert.is_empty() {
        let mut at = buffer.iter_at_offset(patch.delete.start as i32);
        buffer.insert(&mut at, &patch.insert);
        stats.inserted = patch.insert.chars().count() as u32;
        stats.operations += 1;
    }
    stats.operations += 2 * image::anchor(inner, patch.restyle.clone());
    stats.tags = render::retag(inner, patch.restyle.clone());
    inner.patching.set(false);
    stats
}
