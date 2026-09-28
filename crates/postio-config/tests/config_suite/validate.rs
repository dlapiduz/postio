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
    // Two `[keys]` entries on one key: a mistake with no principled winner,
    // and the only collision visible without the command registry.
    //
    // It used to be `reply = "a"` alone, against archive's *default*, which
    // this crate could see only because it kept a 23-command copy of the
    // defaults — and so was silent for the other 56 (#1227). An override
    // landing on a default's key is not an error at all now: the override
    // wins, deliberately, and the displaced command is reported by
    // `Keymap::resolve_on`. See
    // `core_suite::config::an_override_that_takes_a_default_key_is_reported`.
    let text = "[keys]\nreply = \"a\"\narchive_thread = \"a\"\n";
    let checked = check(text);
    let err = checked.validation.first_error().expect("a conflict");
    assert_eq!(err.line, 3);
    assert!(err.message.contains("archive_thread"), "{}", err.message);
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

// ------------------------------------------------------- [[focus.digests]] --
//
// Spec 007 T132, contracts/config.md: validation reports an unparsable
// query by the rule's name and the query's index, an unknown cadence, a day
// that does not fit the cadence, and a duplicate name. A rule that fails is
// not applied; the others still are (ADR 0008 Q6), so none of these stop
// the file from loading.

/// data-model.md's example rule, and a daily one beside it.
const DIGESTS: &str = r#"[[focus.digests]]
name    = "Newsletters"
match   = ["from:news@localfirst.example", "from:editor@ledger.example"]
cadence = "weekly"
day     = "saturday"
at      = "16:00"

[[focus.digests]]
name    = "School"
match   = ["from:office@school.example"]
cadence = "monthly"
day     = 28
at      = "08:30"
"#;

/// Every error for `text`, as `(path, message)`.
fn errors(text: &str) -> Vec<(String, String)> {
    check(text)
        .validation
        .errors()
        .iter()
        .map(|err| (err.path.clone(), err.message.clone()))
        .collect()
}

/// A digest rule named `name`, with the rest given as TOML lines.
fn rule(name: &str, rest: &str) -> String {
    format!("[[focus.digests]]\nname = \"{name}\"\n{rest}\n")
}

#[test]
fn good_digest_rules_are_valid_and_all_apply() {
    let checked = check(DIGESTS);
    assert!(
        checked.validation.is_valid(),
        "{:?}",
        checked.validation.errors()
    );
    let config = checked.config.expect("a config");
    let names: Vec<&str> = config
        .focus
        .applicable_digests()
        .into_iter()
        .map(|(rule, _)| rule.name.as_str())
        .collect();
    assert_eq!(names, ["Newsletters", "School"]);
}

#[test]
fn an_unknown_cadence_is_reported_by_the_rules_name() {
    let text = rule(
        "Newsletters",
        "match = [\"from:news@localfirst.example\"]\ncadence = \"fortnightly\"\nat = \"16:00\"",
    );
    let found = errors(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    let (path, message) = &found[0];
    assert_eq!(path, "focus.digests[0].cadence");
    assert!(message.contains("Newsletters"), "{message}");
    assert!(message.contains("fortnightly"), "{message}");
}

#[test]
fn a_day_that_does_not_fit_the_cadence_is_reported() {
    for (rest, why) in [
        (
            "cadence = \"weekly\"\nday = 15\nat = \"09:00\"",
            "a weekly rule's day is a weekday",
        ),
        (
            "cadence = \"weekly\"\nat = \"09:00\"",
            "a weekly rule names its day",
        ),
        (
            "cadence = \"weekly\"\nday = \"someday\"\nat = \"09:00\"",
            "a weekday by name",
        ),
        (
            "cadence = \"monthly\"\nday = \"saturday\"\nat = \"09:00\"",
            "a monthly rule's day is a day of the month",
        ),
        (
            "cadence = \"monthly\"\nday = 31\nat = \"09:00\"",
            "a monthly day stops at 28, so every month has one",
        ),
        (
            "cadence = \"monthly\"\nday = 0\nat = \"09:00\"",
            "a monthly day starts at 1",
        ),
        (
            "cadence = \"daily\"\nday = \"monday\"\nat = \"09:00\"",
            "a daily rule has no day",
        ),
    ] {
        let text = rule("Rule", &format!("match = [\"from:a@example.com\"]\n{rest}"));
        let found = errors(&text);
        assert_eq!(found.len(), 1, "{why}: {found:?}");
        assert!(found[0].1.contains("Rule"), "{why}: {}", found[0].1);
    }
}

#[test]
fn a_time_that_is_not_a_clock_time_is_reported() {
    for at in ["25:00", "9", "noon", ""] {
        let text = rule(
            "Rule",
            &format!("match = [\"from:a@example.com\"]\ncadence = \"daily\"\nat = \"{at}\""),
        );
        let found = errors(&text);
        assert_eq!(found.len(), 1, "{at:?}: {found:?}");
        assert!(found[0].1.contains("Rule"), "{}", found[0].1);
    }
}

#[test]
fn a_duplicate_name_is_reported_at_the_later_rule_and_only_the_first_applies() {
    let body = "match = [\"from:a@example.com\"]\ncadence = \"daily\"\nat = \"09:00\"";
    let text = format!("{}\n{}", rule("Twice", body), rule("Twice", body));
    let checked = check(&text);
    let found = errors(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "focus.digests[1].name");
    assert!(found[0].1.contains("Twice"), "{}", found[0].1);
    let config = checked
        .config
        .expect("a semantic problem does not stop the file loading");
    assert_eq!(config.focus.applicable_digests().len(), 1);
}

#[test]
fn an_empty_query_is_reported_by_the_rules_name_and_its_place() {
    let text = rule(
        "Newsletters",
        "match = [\"from:news@localfirst.example\", \"  \"]\ncadence = \"daily\"\nat = \"09:00\"",
    );
    let found = errors(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    let (path, message) = &found[0];
    assert_eq!(path, "focus.digests[0].match[1]");
    assert!(message.contains("Newsletters"), "{message}");
    assert!(message.contains('2'), "the second query: {message}");
}

#[test]
fn a_rule_with_no_query_or_no_name_is_reported() {
    let no_query = rule("Empty", "cadence = \"daily\"\nat = \"09:00\"");
    assert_eq!(errors(&no_query).len(), 1, "{:?}", errors(&no_query));
    let no_name = "[[focus.digests]]\nmatch = [\"from:a@example.com\"]\ncadence = \"daily\"\nat = \"09:00\"\n";
    assert_eq!(errors(no_name).len(), 1, "{:?}", errors(no_name));
}

#[test]
fn a_rule_that_fails_is_not_applied_and_the_others_still_are() {
    let bad = rule(
        "Broken",
        "match = [\"from:a@example.com\"]\ncadence = \"hourly\"\nat = \"09:00\"",
    );
    let text = format!("{DIGESTS}\n{bad}");
    let checked = check(&text);
    assert!(!checked.validation.is_valid());
    let config = checked
        .config
        .expect("a semantic problem does not stop the file loading");
    let names: Vec<&str> = config
        .focus
        .applicable_digests()
        .into_iter()
        .map(|(rule, _)| rule.name.as_str())
        .collect();
    assert_eq!(names, ["Newsletters", "School"]);
}

// --------------------------------------------------- [focus.filter] never --
//
// Spec 007 T123, contracts/config.md: an entry is an address or a whole
// domain (`@example.org`). One that is neither is reported by its place,
// never by what it says, and it pins nobody.

#[test]
fn never_entries_that_are_addresses_or_domains_are_valid() {
    let checked = check(
        "[focus.filter]\nnever = [\"pinned@example.org\", \"@example.net\", \"Ada@Example.COM\"]\n",
    );
    assert!(
        checked.validation.is_valid(),
        "{:?}",
        checked.validation.errors()
    );
}

#[test]
fn a_never_entry_that_is_neither_an_address_nor_a_domain_is_reported_by_its_place() {
    for (entry, why) in [
        ("pinned", "no domain"),
        ("@", "an at sign and nothing after"),
        ("pinned@", "no domain after the at sign"),
        ("two@at@example.org", "two at signs"),
        ("   ", "blank"),
        ("pinned @example.org", "a space inside"),
    ] {
        let text = format!("[focus.filter]\nnever = [\"ada@example.com\", \"{entry}\"]\n");
        let found = errors(&text);
        assert_eq!(found.len(), 1, "{why}: {found:?}");
        let (path, message) = &found[0];
        assert_eq!(path, "focus.filter.never[1]", "{why}");
        // A lone `@` is in the message's own example; anything longer the
        // message has no business repeating.
        assert!(
            entry.trim().len() < 2 || !message.contains(entry.trim()),
            "{why}: the message repeats the entry: {message}"
        );
        assert!(message.contains('2'), "{why}: the second entry: {message}");
    }
}

// ------------------------------------------- [focus.filter] stop_markers --
//
// Spec 007 T118, contracts/config.md: each entry names a sender and a kind,
// `question` or `todo`. One that does not is reported by its place, never
// by what it says, and stops nothing.

#[test]
fn stop_markers_that_name_a_sender_and_a_kind_are_valid() {
    let checked = check(
        "[focus.filter]\nstop_markers = [{ sender = \"news@ledger.example\", kind = \"question\" }, \
         { sender = \"@example.net\", kind = \"todo\" }]\n",
    );
    assert!(
        checked.validation.is_valid(),
        "{:?}",
        checked.validation.errors()
    );
}

#[test]
fn a_stop_marker_with_no_sender_or_an_unknown_kind_is_reported_by_its_place() {
    for (entry, why) in [
        (
            "{ sender = \"news@ledger.example\", kind = \"invite\" }",
            "a kind nobody dismisses",
        ),
        (
            "{ sender = \"news\", kind = \"question\" }",
            "not an address",
        ),
        ("{ kind = \"todo\" }", "no sender"),
    ] {
        let text = format!(
            "[focus.filter]\nstop_markers = [{{ sender = \"ada@example.com\", kind = \"todo\" }}, {entry}]\n"
        );
        let found = errors(&text);
        assert_eq!(found.len(), 1, "{why}: {found:?}");
        let (path, message) = &found[0];
        assert_eq!(path, "focus.filter.stop_markers[1]", "{why}");
        assert!(message.contains('2'), "{why}: the second entry: {message}");
        assert!(
            !message.contains("ledger") && !message.contains("news"),
            "{why}: the message repeats the entry: {message}"
        );
    }
}

// --------------------------------------------------------- [focus.model] --
//
// Spec 007 T152, contracts/config.md: the person's own model, off unless the
// section is there. Its endpoint must be on this computer, and anything else
// is reported with the reason; a section that cannot be used names no model
// for any feature, and the file still loads.

const MODEL: &str = r#"[focus.model]
endpoint = "http://127.0.0.1:11434/v1"
model    = "a-small-model"
"#;

#[test]
fn no_model_section_names_no_model_for_any_feature() {
    // SC-016's first half at the config: absent means off, and nothing in
    // the file points anywhere.
    let checked = check("[focus]\nfiltering = true\n");
    assert!(checked.validation.is_valid());
    let config = checked.config.expect("a config");
    assert_eq!(config.focus.model, None);
    for feature in postio_config::ModelFeature::ALL {
        assert!(config.focus.model_for(feature).is_none(), "{feature:?}");
    }
}

#[test]
fn a_model_section_names_the_model_for_each_feature_it_leaves_on() {
    let checked = check(MODEL);
    assert!(
        checked.validation.is_valid(),
        "{:?}",
        checked.validation.errors()
    );
    let config = checked.config.expect("a config");
    for feature in postio_config::ModelFeature::ALL {
        let (endpoint, model) = config.focus.model_for(feature).expect("switched on");
        assert_eq!(model, "a-small-model");
        assert_eq!(endpoint.authority(), "127.0.0.1:11434");
    }

    let off = check(&format!("{MODEL}needs_action = false\nlike_this = false\n"))
        .config
        .expect("a config");
    use postio_config::ModelFeature::{DigestSummary, LikeThis, NeedsAction};
    assert!(off.focus.model_for(NeedsAction).is_none());
    assert!(off.focus.model_for(LikeThis).is_none());
    assert!(off.focus.model_for(DigestSummary).is_some());
}

#[test]
fn an_endpoint_on_another_computer_is_refused_and_says_why() {
    let text = MODEL.replace("127.0.0.1", "192.0.2.7");
    let found = errors(&text);
    assert_eq!(found.len(), 1, "{found:?}");
    let (path, message) = &found[0];
    assert_eq!(path, "focus.model.endpoint");
    assert!(
        message.contains("the model must run on this computer"),
        "{message}"
    );
    let config = check(&text).config.expect("the file still loads");
    for feature in postio_config::ModelFeature::ALL {
        assert!(
            config.focus.model_for(feature).is_none(),
            "a refused endpoint is used by no feature"
        );
    }
}

#[test]
fn a_model_section_with_no_endpoint_or_no_model_is_reported() {
    let found = errors("[focus.model]\nmodel = \"a-small-model\"\n");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "focus.model");
    assert!(found[0].1.contains("endpoint"), "{}", found[0].1);

    let found = errors("[focus.model]\nendpoint = \"http://localhost:8080/v1\"\n");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "focus.model");
    assert!(found[0].1.contains("model"), "{}", found[0].1);
    let config = check("[focus.model]\nendpoint = \"http://localhost:8080/v1\"\n")
        .config
        .expect("a config");
    assert!(
        config
            .focus
            .model_for(postio_config::ModelFeature::NeedsAction)
            .is_none()
    );
}

#[test]
fn a_model_section_keeps_what_this_version_does_not_know() {
    let config = check(&format!("{MODEL}temperature = 0.2\n"))
        .config
        .expect("a config");
    let model = config.focus.model.expect("the section");
    assert!(model.extras.contains_key("temperature"));
}

// --------------------------------------------------------- [focus.vault] --
//
// Spec 007 T157, contracts/config.md: the Obsidian vault Focus captures
// into, a folder on this computer, with its tasks note and projects folder
// relative to it.

#[test]
fn a_vault_section_names_its_folder_and_notes() {
    let checked = check(
        "[focus.vault]\npath = \"/home/someone/Notes\"\ntasks_note = \"Inbox/Tasks.md\"\n\
         projects = \"Projects\"\n",
    );
    assert!(
        checked.validation.is_valid(),
        "{:?}",
        checked.validation.errors()
    );
    let vault = checked
        .config
        .expect("a config")
        .focus
        .vault
        .expect("the section");
    assert_eq!(
        vault.root(),
        Some(std::path::PathBuf::from("/home/someone/Notes"))
    );
    assert_eq!(
        vault.tasks_note(),
        std::path::PathBuf::from("Inbox/Tasks.md")
    );
    assert_eq!(vault.projects(), Some(std::path::PathBuf::from("Projects")));

    let plain = check("[focus.vault]\npath = \"/srv/vault\"\n")
        .config
        .expect("a config")
        .focus
        .vault
        .expect("the section");
    assert_eq!(plain.tasks_note(), std::path::PathBuf::from("Tasks.md"));
    assert_eq!(plain.projects(), None);
    assert_eq!(check("").config.expect("a config").focus.vault, None);
}

#[test]
fn a_vault_path_that_is_not_a_folder_on_this_computer_or_a_note_outside_it_is_reported() {
    for (text, path) in [
        ("[focus.vault]\npath = \"Notes\"\n", "focus.vault.path"),
        ("[focus.vault]\ntasks_note = \"Tasks.md\"\n", "focus.vault"),
        (
            "[focus.vault]\npath = \"/srv/vault\"\ntasks_note = \"../Tasks.md\"\n",
            "focus.vault.tasks_note",
        ),
        (
            "[focus.vault]\npath = \"/srv/vault\"\nprojects = \"/srv/projects\"\n",
            "focus.vault.projects",
        ),
    ] {
        let found = errors(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(found[0].0, path, "{text}");
    }
}
