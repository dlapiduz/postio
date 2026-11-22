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
    let file = config_file("[sync\ncheck_for_mail =");

    let policy = watch_policy_at(file.path());
    assert!(policy.idle);
    assert_eq!(policy.poll_interval, Duration::from_secs(300));
}

#[test]
fn idle_refresh_is_untouched_by_config_and_keeps_its_own_default() {
    let file = config_file("[sync]\ncheck_for_mail = \"poll\"\npoll_interval_secs = 30\n");
    assert_eq!(
        watch_policy_at(file.path()).idle_refresh,
        postio_sync::WatchPolicy::default().idle_refresh
    );
}

#[test]
fn check_for_mail_manual_turns_off_idle_and_sets_manual() {
    let file = config_file("[sync]\ncheck_for_mail = \"manual\"\n");
    let policy = watch_policy_at(file.path());
    assert!(!policy.idle);
    assert!(policy.manual);
}

#[test]
fn check_for_mail_idle_and_poll_leave_manual_off() {
    for text in ["idle", "poll"] {
        let file = config_file(&format!("[sync]\ncheck_for_mail = \"{text}\"\n"));
        assert!(
            !watch_policy_at(file.path()).manual,
            "check_for_mail = {text:?} must not silence automatic checking"
        );
    }
}
