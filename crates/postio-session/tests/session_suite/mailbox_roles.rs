//! `[mailboxes]` on disk, all the way to a role a verb can find.
//!
//! The tiers themselves are unit-tested where they live — precedence in
//! `postio-model`, parsing and validation in `postio-config`, and what
//! reconciliation writes in `postio-sync`. What none of those can see is
//! whether a file a person actually edits reaches any of them, which is the
//! shape of bug `postio-bl2` is about: every layer passing while nothing
//! joins them.
//!
//! So this starts from bytes on disk and asserts the far end: the override a
//! person wrote is the one folder resolution will use.
//!
//! Nothing here touches the network.

use std::io::Write;

use postio_model::MailboxRole;
use postio_session::mailbox_roles_at;

fn config_file(body: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("a temp config");
    file.write_all(body.as_bytes()).expect("write");
    file.flush().expect("flush");
    file
}

#[test]
fn a_mailboxes_section_on_disk_reaches_resolution() {
    let file = config_file(
        r#"
[mailboxes]
archive = "Vecchia Posta"
trash = "Cestino"
"#,
    );

    let overrides = mailbox_roles_at(file.path());

    assert_eq!(
        overrides.role_for("Vecchia Posta"),
        Some(MailboxRole::Archive),
        "the section was read but did not reach the type resolution uses"
    );
    assert_eq!(overrides.role_for("Cestino"), Some(MailboxRole::Trash));

    // …and the precedence it carries is the one the model defines: above the
    // server's own attribute, not merely above the name guess.
    assert_eq!(
        overrides.resolve(["\\Junk"], "Vecchia Posta"),
        MailboxRole::Archive
    );
}

#[test]
fn a_file_with_no_mailboxes_section_overrides_nothing() {
    let file = config_file("[ui]\ndensity = \"comfortable\"\n");
    assert!(
        mailbox_roles_at(file.path()).is_empty(),
        "a config that says nothing about mailboxes must change nothing"
    );
}

#[test]
fn an_unreadable_or_broken_file_overrides_nothing() {
    // Both are reported by `validate`, with a line number, and shown in the
    // settings panel. What must not happen here is a half-applied mapping:
    // filing mail somewhere nobody chose is worse than filing it where it
    // always went.
    assert!(mailbox_roles_at(std::path::Path::new("/nonexistent/postio.toml")).is_empty());

    let broken = config_file("[mailboxes\narchive = \"Vecchia Posta\"\n");
    assert!(
        mailbox_roles_at(broken.path()).is_empty(),
        "a syntax error must not produce a partial mapping"
    );
}

#[test]
fn a_typo_in_a_role_name_maps_nothing() {
    let file = config_file("[mailboxes]\narchiv = \"Vecchia Posta\"\n");
    assert!(
        mailbox_roles_at(file.path()).is_empty(),
        "a misspelled role must not be guessed into the nearest real one"
    );
}
