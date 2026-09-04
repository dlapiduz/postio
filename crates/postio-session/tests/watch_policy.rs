//! `[sync]` on disk, all the way to the `WatchPolicy` the engine is spawned
//! with (#932).
//!
//! The same shape as `backfill_policy.rs`: `WatchPolicy::idle` and
//! `::poll_interval` can be perfect in `postio-sync` while `[sync] idle` and
//! `poll_interval_secs` reach neither, because `postio-session::engine::start`
//! spawned `EngineParts { watch: Default::default(), .. }` and nothing
//! carried a config value into it. `WatchPolicy::default()` was the only
//! constructor ever called outside its own tests.
//!
//! Nothing here touches the network.

use std::io::Write;
use std::time::Duration;

use postio_session::watch_policy_at;

fn config_file(body: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("a temp config");
    file.write_all(body.as_bytes()).expect("write");
    file.flush().expect("flush");
    file
}

#[test]
fn a_file_that_says_nothing_leaves_the_built_in_defaults_standing() {
    let file = config_file("");

    let policy = watch_policy_at(file.path());

    assert!(policy.idle, "idle is the default check_for_mail (#867)");
    assert_eq!(policy.poll_interval, Duration::from_secs(300));
}

#[test]
fn check_for_mail_idle_on_disk_turns_idle_on() {
    let file = config_file("[sync]\ncheck_for_mail = \"idle\"\n");
    assert!(watch_policy_at(file.path()).idle);
}

#[test]
fn check_for_mail_poll_on_disk_turns_idle_off() {
    let file = config_file("[sync]\ncheck_for_mail = \"poll\"\n");
    assert!(!watch_policy_at(file.path()).idle);
}

#[test]
fn the_retired_idle_key_still_reaches_the_policy() {
    // #867: `idle = true`/`idle = false` still parses, only ever writing
    // `check_for_mail` back out. The policy has to honour a file nobody has
    // migrated yet, the same way `SyncConfig::deserialize` does.
    for (text, expected) in [("idle = true", true), ("idle = false", false)] {
        let file = config_file(&format!("[sync]\n{text}\n"));
        assert_eq!(watch_policy_at(file.path()).idle, expected, "for {text:?}");
    }
}

#[test]
fn poll_interval_secs_on_disk_reaches_the_policy() {
    let file = config_file("[sync]\npoll_interval_secs = 60\n");
    assert_eq!(
        watch_policy_at(file.path()).poll_interval,
        Duration::from_secs(60)
    );
}

#[test]
fn a_file_that_will_not_parse_leaves_the_defaults_standing() {
    // Same tolerance `backfill_policy_at` and `notifications::config_at`
    // keep: a broken file is reported by the settings panel, not a reason to
    // sync differently.
    let file = config_file("[sync\ncheck_for_mail =");

    let policy = watch_policy_at(file.path());
    assert!(policy.idle);
    assert_eq!(policy.poll_interval, Duration::from_secs(300));
}

#[test]
fn idle_refresh_is_untouched_by_config_and_keeps_its_own_default() {
    // Nothing in `[sync]` names it -- it is a protocol-tolerance constant,
    // not a setting -- so a config that changes everything else must not
    // move it.
    let file = config_file("[sync]\ncheck_for_mail = \"poll\"\npoll_interval_secs = 30\n");
    assert_eq!(
        watch_policy_at(file.path()).idle_refresh,
        postio_sync::WatchPolicy::default().idle_refresh
    );
}

#[test]
fn check_for_mail_manual_does_not_idle_but_still_polls_for_now() {
    // `WatchPolicy` has no "never check automatically" state yet -- see
    // `watch_policy`'s own doc comment -- so `Manual` gets the closest
    // faithful answer available: no push connection, same interval polling
    // as `Poll`. Pinned so a future `WatchPolicy` that *can* express "off"
    // changes this test rather than drifting past it unnoticed.
    let file = config_file("[sync]\ncheck_for_mail = \"manual\"\n");
    let policy = watch_policy_at(file.path());
    assert!(!policy.idle);
    assert_eq!(policy.poll_interval, Duration::from_secs(300));
}
