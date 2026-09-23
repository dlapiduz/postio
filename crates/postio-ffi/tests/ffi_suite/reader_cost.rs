//! What moving between messages costs, counted on the Mac as well (#1586).
//!
//! `postio_ui::reader::cost` counts rendering surfaces because "how many" is
//! the same number on any machine and sixteen milliseconds is not. `postio-gtk`
//! notes both ends of a surface's life; until this crossed, the macOS reader
//! noted neither, so the claim was gated on one platform and unmeasured on the
//! other.
//!
//! The point of these cases is that there is **one** set of counters. A Swift
//! test reads back exactly what `postio_ui::test_support` reads, so both
//! frontends are measured against one definition of the cost rather than two
//! that could drift.

use postio_ui::test_support as shared;

#[test]
fn a_surface_noted_across_the_boundary_lands_in_the_shared_counter() {
    let created = shared::surfaces_created();
    let held = shared::surfaces_held();

    postio_ffi::note_reader_surface_created();
    postio_ffi::note_reader_surface_created();
    postio_ffi::note_reader_surface_released();

    assert_eq!(
        shared::surfaces_created() - created,
        2,
        "the boundary's note did not reach `postio_ui::reader::cost`, so the \
         Mac is counting into a second set nobody else reads"
    );
    assert_eq!(
        shared::surfaces_held() - held,
        1,
        "held is created minus released, and it is the number that notices a \
         conversation that never lets go"
    );
}

#[test]
fn a_render_noted_across_the_boundary_lands_in_the_shared_counter() {
    let renders = shared::renders_issued();

    postio_ffi::note_reader_render();

    assert_eq!(shared::renders_issued() - renders, 1);
}

#[test]
fn the_boundary_reads_back_what_the_shared_counters_hold() {
    // Noted through `postio_ui` directly -- the way `postio-gtk` notes -- and
    // read through the boundary, so a Swift test asserting on the readers is
    // asserting on the shared numbers and not on a copy.
    postio_ui::reader::cost::note_surface_created();
    postio_ui::reader::cost::note_render();

    assert_eq!(
        postio_ffi::reader_surfaces_created(),
        shared::surfaces_created()
    );
    assert_eq!(postio_ffi::reader_surfaces_held(), shared::surfaces_held());
    assert_eq!(
        postio_ffi::reader_renders_issued(),
        shared::renders_issued()
    );
}
