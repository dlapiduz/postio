//! Counters the tests read, in `src/` because tests in several crates need them.
//! Each is a process-wide running total: take a `before`, act, assert on the delta.

use crate::reader::cost;

pub fn documents_built() -> u64 {
    cost::read(&cost::DOCUMENTS)
}

pub fn document_bytes() -> u64 {
    cost::read(&cost::DOCUMENT_BYTES)
}

pub fn largest_document() -> u64 {
    cost::read(&cost::LARGEST_DOCUMENT)
}

pub fn bodies_sanitised() -> u64 {
    cost::read(&cost::BODIES_SANITISED)
}

pub fn bodies_treated() -> u64 {
    cost::read(&cost::BODIES_TREATED)
}

pub fn bulk_judged() -> u64 {
    cost::read(&cost::BULK_JUDGED)
}

pub fn renders_issued() -> u64 {
    cost::read(&cost::RENDERS)
}

pub fn surfaces_created() -> u64 {
    cost::read(&cost::SURFACES_CREATED)
}

/// Signed, so a release without a create shows as negative rather than saturating at zero.
pub fn surfaces_held() -> i64 {
    cost::read(&cost::SURFACES_CREATED) as i64 - cost::read(&cost::SURFACES_RELEASED) as i64
}

pub fn pages_requested() -> u64 {
    cost::read(&cost::PAGES_REQUESTED)
}

pub fn snapshot_counts() -> (u64, crate::reader::cost::SnapshotCounts) {
    use crate::reader::cost::*;
    let get = cost::read;
    (
        get(&SNAPSHOTS),
        SnapshotCounts {
            style_passes: get(&STYLE_PASSES),
            nodes: get(&SNAPSHOT_NODES),
            repaired_runs: get(&REPAIRED_RUNS),
            resources_unresolved: get(&RESOURCES_UNRESOLVED),
            images_placeholdered: get(&IMAGES_PLACEHOLDERED),
            display_list_commands: get(&DISPLAY_LIST_COMMANDS),
        },
    )
}
