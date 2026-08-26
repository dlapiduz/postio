//! The drag-export directory is bounded (#278).
//!
//! Every message or attachment dragged out of Postio writes a file into
//! `paths::export_dir()`, and until this landed **nothing ever deleted one**:
//! three writers, no reader. A person who drags mail out regularly
//! accumulated a plaintext copy of every message they had ever dragged, in
//! the cache, forever.
//!
//! The obvious fix is the dangerous one, and these tests are mostly about the
//! guard rather than the sweep. #121 established that the file transfer
//! portal hands the receiver a **path** and never copies the bytes, so a file
//! removed after the drop but before the receiver reads it produces a silent
//! no-op: the receiver gets nothing and Postio reports success. A sweep that
//! can touch a file a drop is still using is exactly that bug.
//!
//! The age guard is what makes it safe, and it is a real bound rather than a
//! hope: a drop's file is seconds old, and nothing here may delete anything
//! younger than [`postio_session::DRAG_EXPORT_GRACE_PERIOD`].

use std::time::{Duration, SystemTime};

use postio_session::reclaim_drag_exports;

/// Backdate whatever is at `path` to `age` ago.
fn backdate(path: &std::path::Path, age: Duration) {
    std::fs::File::open(path)
        .expect("open the export to backdate it")
        .set_modified(SystemTime::now() - age)
        .expect("backdate the export");
}

/// A file in `directory`, backdated to `age` ago.
fn aged(directory: &std::path::Path, name: &str, age: Duration) -> std::path::PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, b"From: ada@example.com\r\n\r\nbody").expect("write an export");
    backdate(&path, age);
    path
}

#[test]
fn an_export_older_than_the_grace_period_is_reclaimed() {
    let directory = tempfile::tempdir().expect("a scratch directory");
    let stale = aged(
        directory.path(),
        "old.eml",
        Duration::from_secs(60 * 60 * 24),
    );

    let removed = reclaim_drag_exports(directory.path(), Duration::from_secs(60 * 60))
        .expect("the sweep must not fail");

    assert_eq!(removed, 1);
    assert!(!stale.exists(), "a day-old export is nobody's live drag");
}

#[test]
fn a_file_a_drop_might_still_be_reading_is_never_touched() {
    // The whole point. A drop hands over a path and the receiver reads it on
    // its own schedule; the file is seconds old while that happens.
    let directory = tempfile::tempdir().expect("a scratch directory");
    let live = aged(directory.path(), "live.eml", Duration::from_secs(2));

    let removed = reclaim_drag_exports(directory.path(), Duration::from_secs(60 * 60))
        .expect("the sweep must not fail");

    assert_eq!(removed, 0);
    assert!(
        live.exists(),
        "a sweep that can delete a file a drop is still using is #121's \
         silent failure, not a tidy-up"
    );
}

#[test]
fn a_second_instance_mid_drag_is_safe_from_this_ones_startup() {
    // Parallel instances are the reason the guard is an age and not "nothing
    // of mine is in flight". This process cannot see another one's drag, and
    // does not have to: that drag's file is young.
    let directory = tempfile::tempdir().expect("a scratch directory");
    let theirs = aged(directory.path(), "theirs.eml", Duration::from_secs(30));
    let mine = aged(
        directory.path(),
        "mine.eml",
        Duration::from_secs(60 * 60 * 3),
    );

    let removed = reclaim_drag_exports(directory.path(), Duration::from_secs(60 * 60))
        .expect("the sweep must not fail");

    assert_eq!(removed, 1);
    assert!(theirs.exists());
    assert!(!mine.exists());
}

#[test]
fn a_directory_that_was_never_created_is_not_an_error() {
    // The ordinary case on a machine where nobody has ever dragged anything.
    let directory = tempfile::tempdir().expect("a scratch directory");
    let missing = directory.path().join("never-used");

    assert_eq!(
        reclaim_drag_exports(&missing, Duration::from_secs(60 * 60)).expect("not an error"),
        0
    );
}

#[test]
fn a_nested_export_directory_goes_with_its_contents() {
    // Attachments are exported under a directory of their own so two parts
    // with the same filename do not collide, so the sweep has to handle a
    // directory entry as well as a file.
    let directory = tempfile::tempdir().expect("a scratch directory");
    let nested = directory.path().join("message-4");
    std::fs::create_dir(&nested).expect("a nested export");
    aged(&nested, "report.pdf", Duration::from_secs(60 * 60 * 24));
    backdate(&nested, Duration::from_secs(60 * 60 * 24));

    let removed = reclaim_drag_exports(directory.path(), Duration::from_secs(60 * 60))
        .expect("the sweep must not fail");

    assert_eq!(removed, 1);
    assert!(!nested.exists());
}

#[test]
fn the_production_grace_period_is_long_enough_to_be_a_guard() {
    // A drop's file is seconds old. Anything in minutes is already a large
    // margin; this asserts nobody tunes it down to a racy value by accident.
    assert!(
        postio_session::DRAG_EXPORT_GRACE_PERIOD >= Duration::from_secs(60 * 10),
        "the grace period is the only thing standing between this sweep and \
         #121's silent no-op"
    );
}
