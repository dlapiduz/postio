//! What moving between messages costs, counted rather than timed: a count is the same on every
//! machine, a duration is not.
//! Counters are per thread, not per process (#1390), so delta assertions are exact under libtest's
//! thread pool.
//! Read a `before`, act, and assert on the delta.

use std::cell::Cell;
use std::thread::LocalKey;

thread_local! {
    pub(crate) static DOCUMENTS: Cell<u64> = const { Cell::new(0) };
    pub(crate) static BODIES_SANITISED: Cell<u64> = const { Cell::new(0) };
    pub(crate) static BODIES_TREATED: Cell<u64> = const { Cell::new(0) };
    pub(crate) static BULK_JUDGED: Cell<u64> = const { Cell::new(0) };
    pub(crate) static DOCUMENT_BYTES: Cell<u64> = const { Cell::new(0) };
    pub(crate) static LARGEST_DOCUMENT: Cell<u64> = const { Cell::new(0) };
    pub(crate) static RENDERS: Cell<u64> = const { Cell::new(0) };
    pub(crate) static SURFACES_CREATED: Cell<u64> = const { Cell::new(0) };
    pub(crate) static SURFACES_RELEASED: Cell<u64> = const { Cell::new(0) };
    pub(crate) static PAGES_REQUESTED: Cell<u64> = const { Cell::new(0) };
    pub(crate) static SNAPSHOTS: Cell<u64> = const { Cell::new(0) };
    pub(crate) static STYLE_PASSES: Cell<u64> = const { Cell::new(0) };
    pub(crate) static SNAPSHOT_NODES: Cell<u64> = const { Cell::new(0) };
    pub(crate) static REPAIRED_RUNS: Cell<u64> = const { Cell::new(0) };
    pub(crate) static RESOURCES_UNRESOLVED: Cell<u64> = const { Cell::new(0) };
    pub(crate) static IMAGES_PLACEHOLDERED: Cell<u64> = const { Cell::new(0) };
    pub(crate) static DISPLAY_LIST_COMMANDS: Cell<u64> = const { Cell::new(0) };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotCounts {
    pub style_passes: u64,
    pub nodes: u64,
    pub repaired_runs: u64,
    pub resources_unresolved: u64,
    pub images_placeholdered: u64,
    pub display_list_commands: u64,
}

pub(crate) fn bump(counter: &'static LocalKey<Cell<u64>>, by: u64) {
    counter.with(|count| count.set(count.get().saturating_add(by)));
}

fn raise(counter: &'static LocalKey<Cell<u64>>, value: u64) {
    counter.with(|count| count.set(count.get().max(value)));
}

pub(crate) fn read(counter: &'static LocalKey<Cell<u64>>) -> u64 {
    counter.with(Cell::get)
}

/// Called by the assembly itself, so every frontend gets the same answer to "did this document
/// carry bulk".
pub(crate) fn note_document(bytes: usize) {
    bump(&DOCUMENTS, 1);
    bump(&DOCUMENT_BYTES, bytes as u64);
    raise(&LARGEST_DOCUMENT, bytes as u64);
}

/// Called at a frontend's single load choke point; a second path to the engine shows up here
/// instead of silently doubling a keystroke's cost.
pub fn note_render() {
    bump(&RENDERS, 1);
}

pub fn note_snapshot(counts: SnapshotCounts) {
    bump(&SNAPSHOTS, 1);
    bump(&STYLE_PASSES, counts.style_passes);
    bump(&SNAPSHOT_NODES, counts.nodes);
    bump(&REPAIRED_RUNS, counts.repaired_runs);
    bump(&RESOURCES_UNRESOLVED, counts.resources_unresolved);
    bump(&IMAGES_PLACEHOLDERED, counts.images_placeholdered);
    bump(&DISPLAY_LIST_COMMANDS, counts.display_list_commands);
}

pub fn note_surface_created() {
    bump(&SURFACES_CREATED, 1);
}

/// Released rather than dropped: what matters is whether the engine process behind it can go.
pub fn note_surface_released() {
    bump(&SURFACES_RELEASED, 1);
}

/// A bound, not a measurement: opening a folder should cost the pages a screen needs, whatever it
/// holds
/// (#1534 saw 49 distinct pages in 20 s through an eight-page cache). Storage counters cannot see
/// it, each being an ordinary query.
pub fn note_page_requested() {
    bump(&PAGES_REQUESTED, 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure that started #1390: a shared counter's delta is exact only if nothing else is
    /// counting, and libtest runs tests on a thread pool.
    #[test]
    fn a_neighbours_counting_does_not_reach_this_thread() {
        let before = read(&DOCUMENTS);

        let busy = std::thread::spawn(|| {
            for _ in 0..1000 {
                note_document(512);
            }
            read(&DOCUMENTS)
        });
        note_document(16);
        let neighbour = busy.join().expect("the neighbour thread finished");

        assert_eq!(
            read(&DOCUMENTS) - before,
            1,
            "a thousand documents assembled beside this one were counted \
             against it, so every exact-delta assertion in this workspace is \
             a coin toss"
        );
        assert_eq!(
            neighbour, 1000,
            "the neighbour's own counting went somewhere else, which would \
             mean the counters are not per thread but merely broken"
        );
    }

    use crate::reader::document::{self, Sheet};
    use crate::test_support;
    use postio_body::RemoteImages;

    #[test]
    fn assembling_a_document_is_counted_with_its_size() {
        let before = test_support::documents_built();
        let bytes_before = test_support::document_bytes();

        let document = document::document_for("<p>hi</p>", "", RemoteImages::Blocked, Sheet::Theme);

        assert_eq!(
            test_support::documents_built() - before,
            1,
            "assembling a document has to be counted where it is assembled, or \
             a frontend that grows a second path to the engine is invisible"
        );
        assert_eq!(
            test_support::document_bytes() - bytes_before,
            document.len() as u64,
            "the bytes counted must be the bytes handed over -- an approximation \
             cannot catch per-message bulk coming back"
        );
        assert!(
            test_support::largest_document() >= document.len() as u64,
            "the largest document is what notices #749's inlined fonts returning"
        );
    }

    #[test]
    fn a_frontends_reports_reach_the_counters() {
        let renders = test_support::renders_issued();
        let created = test_support::surfaces_created();
        let held = test_support::surfaces_held();

        super::note_render();
        super::note_surface_created();
        super::note_surface_created();
        super::note_surface_released();

        assert_eq!(test_support::renders_issued() - renders, 1);
        assert_eq!(test_support::surfaces_created() - created, 2);
        assert_eq!(
            test_support::surfaces_held() - held,
            1,
            "held is created minus released: a conversation that never lets go \
             shows up here even while every individual render looks cheap"
        );
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::test_support;

    #[test]
    fn each_snapshot_adds_its_costs_to_the_totals() {
        let (shown, before) = test_support::snapshot_counts();
        let one = SnapshotCounts {
            style_passes: 2,
            nodes: 40,
            repaired_runs: 3,
            resources_unresolved: 1,
            images_placeholdered: 1,
            display_list_commands: 90,
        };
        note_snapshot(one);
        note_snapshot(SnapshotCounts {
            style_passes: 1,
            ..one
        });
        let (shown_after, after) = test_support::snapshot_counts();
        assert_eq!(shown_after - shown, 2);
        assert_eq!(after.style_passes - before.style_passes, 3);
        assert_eq!(after.nodes - before.nodes, 80);
        assert_eq!(after.repaired_runs - before.repaired_runs, 6);
        assert_eq!(after.resources_unresolved - before.resources_unresolved, 2);
        assert_eq!(after.images_placeholdered - before.images_placeholdered, 2);
        assert_eq!(
            after.display_list_commands - before.display_list_commands,
            180
        );
    }

    #[test]
    fn page_requests_are_counted_one_each() {
        let pages = test_support::pages_requested();
        note_page_requested();
        note_page_requested();
        assert_eq!(test_support::pages_requested() - pages, 2);
    }
}
