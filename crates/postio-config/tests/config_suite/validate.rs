//! Validation tests for `postio-config`.
//!
//! Written before the implementation, per the TDD rule in `CLAUDE.md`.
//!
//! Canvas 3f has no OK/Cancel dialog: `config.toml` *is* the settings UI, and a
//! single always-visible validity line reports either `valid` or the first
//! problem. That line is one line long, so every error here has to carry a
//! position and prose a human can act on.

use std::path::Path;

use postio_config::validate::{self, Checked};

const GOOD: &str = r#"[ui]
density = "compact"
theme = "dark"

[keys]
archive = "x"
summarize = "g s"

[filters.needs-reply]
query = "is:unread from:team"
pinned = true

[sync]
idle = true
poll_interval_secs = 300
"#;

fn check(text: &str) -> Checked {
    validate::check_str(text)
}

fn first_message(text: &str) -> String {
    let checked = check(text);
    checked
        .validation
        .first_error()
        .unwrap_or_else(|| panic!("expected an error for:\n{text}"))
        .message
        .clone()
}

// ------------------------------------------------------------ the good path --

#[test]
fn a_good_config_is_valid() {
    let checked = check(GOOD);
    assert!(
        checked.validation.is_valid(),
        "unexpected errors: {:?}",
        checked.validation.errors()
    );
    assert_eq!(checked.validation.status(), "valid");
    assert!(checked.config.is_some());
}

#[test]
fn the_empty_config_is_valid() {
    assert!(check("").validation.is_valid());
}

#[test]
fn the_status_line_carries_the_parse_timing() {
    let checked = check(GOOD);
    let line = checked.validation.status_line();
    assert!(line.starts_with("valid"), "{line}");
    assert!(line.contains("parsed in"), "{line}");
    assert!(line.contains("ms"), "{line}");
    // No bound on `elapsed()` here. What this test is about is that the line
    // *carries* a timing, and a 50 ms ceiling on a shared machine is the same
    // wall-clock gate the rest of #917 removed -- with the added problem that
    // it would fail this test, whose subject is the wording.
}

// The 2 ms budget is asserted in `validation_cost.rs`, as allocations rather
// than as a stopwatch reading. It lived here and measured the machine: this
// repository routinely has three sessions compiling at once, and
// `.cargo/config.toml` pins `jobs = 2` because the box is oversubscribed, so
// the assertion failed for reasons that had nothing to do with validation.
// #917, and the rule #100 set.

// ------------------------------------------------------------- keybindings --

#[test]
fn a_dangling_modifier_reports_the_exact_line_and_why() {
    let text = r#"[ui]
density = "compact"

[keys]
reply = "ctrl+"
"#;
    let checked = check(text);
    let err = checked.validation.first_error().expect("an error");
    assert_eq!(err.line, 5, "{err:?}");
    assert_eq!(err.column, 1, "{err:?}");
    assert_eq!(err.path, "keys.reply");
    assert!(
        err.message.contains("ctrl+") && err.message.contains("key"),
        "{}",
        err.message
    );
    assert!(
        checked.validation.status().starts_with("line 5"),
        "{}",
        checked.validation.status()
    );
}

#[test]
fn an_unknown_modifier_is_named() {
    let msg = first_message("[keys]\narchive = \"hyper+a\"\n");
    assert!(msg.contains("hyper"), "{msg}");
    assert!(
        msg.contains("ctrl"),
        "should suggest the real modifiers: {msg}"
    );
}

#[test]
fn an_unknown_key_name_is_named() {
    let msg = first_message("[keys]\nopen_message = \"Retrun\"\n");
    assert!(msg.contains("Retrun"), "{msg}");
}

#[test]
fn an_empty_binding_is_an_error() {
    let msg = first_message("[keys]\narchive = \"\"\n");
    assert!(msg.contains("empty"), "{msg}");
}

#[test]
fn the_canvas_bindings_all_parse() {
    let text = r#"[keys]
next_message = "j"
archive_thread = "A"
open_message = "Return"
back = "Escape"
command_palette = "ctrl+k"
edit_config = "ctrl+e"
search = "/"
cheat_sheet = "?"
goto_starred = "g s"
zoom = "ctrl+shift+plus"
"#;
    let checked = check(text);
    assert!(
        checked.validation.is_valid(),
        "{:?}",
        checked.validation.errors()
    );
}

#[test]
fn two_commands_on_one_key_is_a_conflict() {
    // `a` is archive by default, so rebinding reply onto it collides.
    let text = "[keys]\nreply = \"a\"\n";
    let checked = check(text);
    let err = checked.validation.first_error().expect("a conflict");
    assert_eq!(err.line, 2);
    assert!(err.message.contains("archive"), "{}", err.message);
    assert!(err.message.contains("reply"), "{}", err.message);
}

#[test]
fn rebinding_both_sides_of_a_collision_is_fine() {
    let text = "[keys]\nreply = \"a\"\narchive = \"e\"\n";
    let checked = check(text);
    assert!(
        checked.validation.is_valid(),
        "{:?}",
        checked.validation.errors()
    );
}

// ----------------------------------------------------------- enum values --

#[test]
fn an_unknown_enum_value_points_at_the_value() {
    let text = "[ui]\ndensity = \"enormous\"\n";
    let checked = check(text);
    let err = checked.validation.first_error().expect("an error");
    assert_eq!(err.line, 2, "{err:?}");
    assert_eq!(
        err.column, 11,
        "must point at the value, not the key: {err:?}"
    );
    assert_eq!(err.path, "ui.density");
    assert!(err.message.contains("enormous"), "{}", err.message);
    assert!(
        err.message.contains("airy"),
        "expected values: {}",
        err.message
    );
}

#[test]
fn every_enum_field_is_checked() {
    for (path, snippet) in [
        ("ui.density", "[ui]\ndensity = \"enormous\"\n"),
        ("ui.theme", "[ui]\ntheme = \"sepia\"\n"),
        ("sync.body_fetch", "[sync]\nbody_fetch = \"whenever\"\n"),
        (
            "sync.attachment_fetch",
            "[sync]\nattachment_fetch = \"sometimes\"\n",
        ),
    ] {
        let checked = check(snippet);
        let err = checked
            .validation
            .first_error()
            .unwrap_or_else(|| panic!("no error for {path}"));
        assert_eq!(err.path, path, "{err:?}");
    }
}

// ------------------------------------------------------ account completeness --

// ------------------------------------------------------------ sync, filters --

#[test]
fn zero_valued_sync_settings_are_reported() {
    for (path, snippet) in [
        (
            "sync.poll_interval_secs",
            "[sync]\npoll_interval_secs = 0\n",
        ),
        ("sync.max_connections", "[sync]\nmax_connections = 0\n"),
        (
            "sync.initial_sync_messages",
            "[sync]\ninitial_sync_messages = 0\n",
        ),
    ] {
        let checked = check(snippet);
        let err = checked
            .validation
            .first_error()
            .unwrap_or_else(|| panic!("no error for {path}"));
        assert_eq!(err.path, path, "{err:?}");
        assert_eq!(err.line, 2, "{err:?}");
    }
}

#[test]
fn an_empty_filter_query_is_reported() {
    let checked = check("[filters.needs-reply]\nquery = \"\"\npinned = true\n");
    let err = checked.validation.first_error().expect("an error");
    assert_eq!(err.path, "filters.needs-reply.query");
    assert!(err.message.contains("needs-reply"), "{}", err.message);
}

// -------------------------------------------------------------- TOML syntax --

#[test]
fn a_syntax_error_reports_its_line_and_yields_no_config() {
    let text = "[ui]\ndensity = \"compact\"\n\n[keys\narchive = \"x\"\n";
    let checked = check(text);
    assert!(checked.config.is_none(), "a broken file cannot produce one");
    let err = checked.validation.first_error().expect("an error");
    assert_eq!(err.line, 4, "{err:?}");
    assert!(!checked.validation.is_valid());
}

#[test]
fn a_wrong_type_is_reported_rather_than_panicking() {
    let checked = check("[accounts.a.imap]\nport = \"nine ninety three\"\n");
    assert!(!checked.validation.is_valid());
    assert!(checked.validation.first_error().is_some());
}

// ------------------------------------------------------------------ ordering --

#[test]
fn the_first_error_is_the_topmost_one_in_the_file() {
    let text = r#"[ui]
density = "enormous"

[keys]
reply = "ctrl+"

[filters.x]
query = ""
"#;
    let checked = check(text);
    assert!(checked.validation.errors().len() >= 2);
    assert_eq!(checked.validation.first_error().unwrap().line, 2);
    let lines: Vec<usize> = checked.validation.errors().iter().map(|e| e.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "errors must read down the file");
}

// ------------------------------------------------------------------ secrets --

const PASSWORD: &str = "hunter2-do-not-persist";

#[test]
fn validation_never_quotes_a_secret() {
    let text = format!(
        r#"[ui]
density = "enormous"

[accounts.personal]
email = "ada@example.com"
password = "{PASSWORD}"

[accounts.personal.imap]
host = "imap.example.com"
app_password = "{PASSWORD}"
"#
    );
    let checked = check(&text);
    let rendered = format!(
        "{:?} {} {}",
        checked.validation.errors(),
        checked.validation.status(),
        checked.validation.status_line()
    );
    assert!(!rendered.contains(PASSWORD), "secret leaked:\n{rendered}");
}

#[test]
fn a_secret_in_the_file_is_reported_as_a_problem_to_fix() {
    let text = format!(
        "[accounts.personal]\nemail = \"ada@example.com\"\npassword = \"{PASSWORD}\"\n[accounts.personal.imap]\nhost = \"i\"\n[accounts.personal.smtp]\nhost = \"s\"\n"
    );
    let checked = check(&text);
    let err = checked
        .validation
        .errors()
        .iter()
        .find(|e| e.path == "accounts.personal.password")
        .unwrap_or_else(|| panic!("{:?}", checked.validation.errors()));
    assert_eq!(err.line, 3);
    assert!(err.message.contains("keyring"), "{}", err.message);
    assert!(!err.message.contains(PASSWORD));
}

#[test]
fn a_syntax_error_on_a_secret_line_is_redacted() {
    let checked = check(&format!("[accounts.a]\npassword = \"{PASSWORD}\n"));
    let rendered = format!("{:?}", checked.validation.errors());
    assert!(!rendered.contains(PASSWORD), "{rendered}");
}

// ----------------------------------------------------------------- on disk --

#[test]
fn a_missing_file_is_valid_defaults() {
    let checked = validate::check_path(Path::new("/nonexistent/postio/config.toml"));
    assert!(checked.validation.is_valid());
    assert_eq!(checked.config, Some(postio_config::Config::default()));
}

#[test]
fn a_mailbox_role_that_is_not_a_role_is_reported() {
    let checked = check(
        r#"
        [mailboxes]
        archiv = "Vecchia Posta"
        "#,
    );
    let problem = checked
        .validation
        .errors()
        .iter()
        .find(|error| error.path.starts_with("mailboxes."))
        .expect("a typo'd role must be reported, not silently ignored");
    assert!(
        problem.message.contains("archiv"),
        "the message has to name the key the user typed: {}",
        problem.message
    );
}

#[test]
fn mapping_inbox_is_refused_because_the_server_decides_it() {
    // INBOX is the one folder IMAP names itself, in RFC 3501. Letting someone
    // point `inbox` at another folder would make Postio disagree with every
    // other client on the same account about where mail arrives.
    let checked = check(
        r#"
        [mailboxes]
        inbox = "Somewhere Else"
        "#,
    );
    assert!(
        checked
            .validation
            .errors()
            .iter()
            .any(|error| error.path.starts_with("mailboxes.")),
        "mapping inbox must be refused"
    );
}

#[test]
fn an_empty_mailbox_path_is_reported() {
    let checked = check(
        r#"
        [mailboxes]
        archive = ""
        "#,
    );
    assert!(
        checked
            .validation
            .errors()
            .iter()
            .any(|error| error.path.starts_with("mailboxes.")),
        "an empty path names no folder and must be reported"
    );
}

#[test]
fn a_real_mailbox_mapping_validates_clean() {
    let checked = check(
        r#"
        [mailboxes]
        archive = "Vecchia Posta"
        trash = "Cestino"
        "#,
    );
    assert!(
        !checked
            .validation
            .errors()
            .iter()
            .any(|error| error.path.starts_with("mailboxes.")),
        "a valid mapping was reported as a problem: {:?}",
        checked.validation.errors()
    );
}

#[test]
fn a_retired_accounts_table_is_reported_without_blocking_the_file() {
    // #470 / ADR 0005 Q6b. `[accounts.<id>]` parsed, validated and
    // round-tripped, and nothing read it: an account's host, port, security
    // and name come from the store, written once by onboarding. Editing the
    // section saved, re-parsed with no error, and changed nothing about the
    // running account.
    //
    // Semantic, not Schema: the file still loads and everything else in it
    // still applies. What is wrong is that this one section means nothing,
    // which is exactly the non-blocking kind.
    let checked = check(
        r#"[ui]
density = "compact"

[accounts.personal]
email = "ada@example.com"
"#,
    );
    let retired = checked
        .validation
        .errors()
        .iter()
        .find(|e| e.path.starts_with("accounts"))
        .expect("the retired section is reported");

    assert!(
        !retired.kind.is_blocking(),
        "the config still loads; only this section does nothing"
    );
    assert!(
        retired.message.contains("is ignored"),
        "the message has to say the edit does nothing: {:?}",
        retired.message
    );
    assert!(
        retired.line >= 4,
        "it should point at the table, not the top of the file"
    );
}
