use std::time::{Duration, SystemTime};

use postio_session::reclaim_drag_exports;

fn backdate(path: &std::path::Path, age: Duration) {
    std::fs::File::open(path)
        .expect("open the export to backdate it")
        .set_modified(SystemTime::now() - age)
        .expect("backdate the export");
}

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
    let directory = tempfile::tempdir().expect("a scratch directory");
    let missing = directory.path().join("never-used");

    assert_eq!(
        reclaim_drag_exports(&missing, Duration::from_secs(60 * 60)).expect("not an error"),
        0
    );
}

#[test]
fn a_nested_export_directory_goes_with_its_contents() {
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
    assert!(
        postio_session::DRAG_EXPORT_GRACE_PERIOD >= Duration::from_secs(60 * 10),
        "the grace period is the only thing standing between this sweep and \
         #121's silent no-op"
    );
}
