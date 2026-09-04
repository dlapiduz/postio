//! `[sync]` on disk, all the way to the policy the engine is spawned with.
//!
//! The shape of bug this is about is the one `mailbox_roles.rs` names: every
//! layer passing while nothing joins them. `AttachmentPolicy` can be perfect
//! in `postio-sync` and `AttachmentFetch` perfect in `postio-config` while a
//! person's `attachment_fetch = "eager"` reaches neither — which is exactly
//! what `body_fetch` did for the whole life of this project, documented in
//! `BackfillPolicy::background` and wired to nothing.
//!
//! Nothing here touches the network.

use std::io::Write;

use postio_runtime::AttachmentPolicy;
use postio_session::backfill_policy_at;

fn config_file(body: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("a temp config");
    file.write_all(body.as_bytes()).expect("write");
    file.flush().expect("flush");
    file
}

#[test]
fn a_file_that_says_nothing_leaves_the_payload_axis_on_demand() {
    let file = config_file("");

    let policy = backfill_policy_at(file.path());

    assert_eq!(policy.attachments, AttachmentPolicy::OnOpen);
    assert!(policy.background, "and the text axis still runs");
}

#[test]
fn attachment_fetch_on_disk_reaches_the_policy() {
    for (text, expected) in [
        ("eager", AttachmentPolicy::Eager),
        ("never", AttachmentPolicy::Never),
        ("on_open", AttachmentPolicy::OnOpen),
    ] {
        let file = config_file(&format!("[sync]\nattachment_fetch = \"{text}\"\n"));

        assert_eq!(
            backfill_policy_at(file.path()).attachments,
            expected,
            "for attachment_fetch = {text:?}"
        );
    }
}

#[test]
fn body_fetch_on_disk_reaches_the_policy_too() {
    // Documented on `BackfillPolicy::background` since it was written, and
    // read by nothing: `engine::start` spawned `BackfillPolicy::default()`,
    // so turning bodies off in the file did nothing at all.
    let file = config_file("[sync]\nbody_fetch = \"eager\"\n");
    assert!(backfill_policy_at(file.path()).background);
}

#[test]
fn a_file_that_will_not_parse_leaves_the_defaults_standing() {
    // Same tolerance `mailbox_roles_at` and `notifications::config_at` keep:
    // a broken file is a thing the settings panel reports, not a reason to
    // sync differently.
    let file = config_file("[sync\nattachment_fetch =");

    assert_eq!(
        backfill_policy_at(file.path()).attachments,
        AttachmentPolicy::OnOpen
    );
}

#[test]
fn the_inline_cap_on_disk_reaches_the_policy() {
    // #751: `max_inline_bytes` was ADR 0017's rule and nothing read it. The
    // same shape of gap `body_fetch` had — a field with a docstring, wired to
    // nothing — so it is worth one assertion that the file reaches the engine.
    let file = config_file("[sync]\nmax_inline_bytes = 4096\n");
    assert_eq!(
        backfill_policy_at(file.path()).max_inline_bytes,
        Some(4_096)
    );
}

#[test]
fn a_zero_inline_cap_turns_the_rule_off_rather_than_capping_at_nothing() {
    let file = config_file("[sync]\nmax_inline_bytes = 0\n");
    assert_eq!(backfill_policy_at(file.path()).max_inline_bytes, None);
}

#[test]
fn a_file_that_says_nothing_still_carries_inline_images_with_the_text() {
    let file = config_file("");
    assert_eq!(
        backfill_policy_at(file.path()).max_inline_bytes,
        Some(256 * 1024),
        "the default has to be the one ADR 0017 named, or an untouched \
         install reads HTML mail with broken boxes in it"
    );
}
