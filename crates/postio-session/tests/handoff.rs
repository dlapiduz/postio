//! Handing a draft to another editor (#1270).

use std::path::Path;

use postio_session::handoff::{begin, finish, read_back};

fn a_directory() -> tempfile::TempDir {
    tempfile::tempdir().expect("a scratch directory")
}

#[test]
fn the_draft_goes_to_a_file_the_other_editor_can_open() {
    let scratch = a_directory();
    let out = begin(scratch.path(), 7, "The gate closes at six.").expect("it writes");

    assert_eq!(
        std::fs::read_to_string(&out.path).expect("a read"),
        "The gate closes at six."
    );
    assert!(
        out.path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("7"),
        "named for the draft, so a second hand-off reuses it: {:?}",
        out.path
    );
}

#[cfg(unix)]
#[test]
fn unsent_mail_is_not_left_where_everyone_can_read_it() {
    // A draft is often the most private mail there is: it is the part still
    // being thought about. `/tmp` is world-readable on every Unix.
    use std::os::unix::fs::PermissionsExt;
    let scratch = a_directory();
    let out = begin(scratch.path(), 7, "something private").expect("it writes");

    let mode = std::fs::metadata(&out.path)
        .expect("a stat")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "the file is the user's alone");
    let directory = std::fs::metadata(scratch.path())
        .expect("a stat")
        .permissions()
        .mode();
    assert_eq!(directory & 0o777, 0o700, "and so is the directory");
}

#[test]
fn what_the_editor_wrote_is_what_comes_back() {
    let scratch = a_directory();
    let out = begin(scratch.path(), 7, "before").expect("it writes");
    std::fs::write(&out.path, "after, with a second line\nand a third").expect("the editor saves");

    assert_eq!(
        read_back(&out).expect("it reads back"),
        "after, with a second line\nand a third"
    );
}

#[test]
fn an_empty_file_is_refused_rather_than_believed() {
    // An editor that saves by truncating and writing is briefly empty on
    // disk. A frontend polling on window focus can land in that window, and
    // believing it would throw away everything the user had written.
    let scratch = a_directory();
    let out = begin(scratch.path(), 7, "everything they wrote").expect("it writes");
    std::fs::write(&out.path, "   \n").expect("a truncating save, caught mid-write");

    let refusal = read_back(&out).expect_err("nothing is taken back");
    assert!(refusal.contains("empty"), "{refusal}");
    assert!(
        refusal.contains("Clear the message"),
        "and says how to mean it: {refusal}"
    );
}

#[test]
fn finishing_takes_the_file_away() {
    // A file left behind is a draft's text sitting on disk.
    let scratch = a_directory();
    let out = begin(scratch.path(), 7, "the gate").expect("it writes");

    finish(&out);

    assert!(!out.path.exists());
}

#[test]
fn finishing_twice_is_harmless() {
    let scratch = a_directory();
    let out = begin(scratch.path(), 7, "the gate").expect("it writes");
    finish(&out);
    finish(&out);
}

#[test]
fn a_directory_that_cannot_be_made_says_so_rather_than_panicking() {
    let out = begin(Path::new("/dev/null/nope"), 7, "text");
    assert!(out.is_err());
}
