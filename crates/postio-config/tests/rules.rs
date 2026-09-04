//! `[[rules]]` — the schema, the typed parse, and what a broken entry says.
//!
//! Written before the implementation, per the TDD rule in `CLAUDE.md`.
//!
//! ADR 0008 Q4 makes rules an **array of tables** rather than a map, because
//! the file is then the evaluation order and nobody has to renumber an
//! `order = 3` field to insert a rule in the middle. This file holds that
//! shape, and holds the promise that nothing in it is ever silently dropped:
//! a rule the user believes is running and is not is the failure mode the
//! whole section is arranged against.

use postio_config::Config;
use postio_model::rule::{Action, RuleError};

const RULES: &str = r#"[filters.needs-reply]
query = "is:unread from:team"

[[rules]]
name    = "receipts"
query   = "from:billing has:attach"
actions = ["move:Receipts", "mark-read"]
stop    = true

[[rules]]
name    = "nudge"
filter  = "needs-reply"
actions = ["flag"]
enabled = false
"#;

fn parse(text: &str) -> Config {
    Config::from_toml_str(text).expect("the config parses")
}

#[test]
fn rules_parse_in_file_order_with_their_queries_and_actions() {
    let config = parse(RULES);
    let rules: Vec<_> = config
        .rules()
        .into_iter()
        .map(|rule| rule.expect("a rule"))
        .collect();

    assert_eq!(rules.len(), 2);
    assert_eq!(
        rules
            .iter()
            .map(|rule| rule.name.as_str())
            .collect::<Vec<_>>(),
        vec!["receipts", "nudge"],
        "the file is the order (ADR 0008 Q4), so the array's order is the answer"
    );

    assert_eq!(rules[0].query, "from:billing has:attach");
    assert_eq!(
        rules[0].actions,
        vec![Action::Move("Receipts".into()), Action::MarkRead]
    );
    assert!(rules[0].stop);
    assert!(rules[0].enabled);

    assert_eq!(
        rules[1].query, "is:unread from:team",
        "`filter = \"needs-reply\"` reuses the query the user already tuned"
    );
    assert_eq!(rules[1].actions, vec![Action::Flag]);
    assert!(!rules[1].stop, "stop defaults to false");
    assert!(!rules[1].enabled, "and this one was dry-run");
}

#[test]
fn a_rule_with_no_rules_section_is_simply_no_rules() {
    assert!(parse("[ui]\ndensity = \"compact\"\n").rules().is_empty());
    assert!(Config::default().rules().is_empty());
}

#[test]
fn an_undefined_filter_reference_is_an_error_and_not_a_dropped_rule() {
    // The acceptance criterion, at the config layer: the entry still comes
    // back, carrying the reason it is not a rule. Dropping it would leave a
    // user with a rule they think is running.
    let config = parse("[[rules]]\nname = \"x\"\nfilter = \"nope\"\nactions = [\"flag\"]\n");
    let parsed = config.rules();
    assert_eq!(parsed.len(), 1, "the entry is not dropped");
    assert_eq!(
        parsed[0],
        Err(RuleError::UnknownFilter {
            filter: "nope".to_owned()
        })
    );
}

#[test]
fn one_broken_rule_does_not_hide_the_ones_around_it() {
    // Per-rule isolation starts here: `rules()` answers one result per entry,
    // so a caller can run the good ones and report the bad one rather than
    // refusing the file.
    let text = r#"[[rules]]
name = "good"
query = "from:ada"
actions = ["flag"]

[[rules]]
name = "broken"
query = "from:bob"
actions = ["teleport"]

[[rules]]
name = "also-good"
query = "from:carol"
actions = ["archive"]
"#;
    let parsed = parse(text).rules();
    assert_eq!(parsed.len(), 3);
    assert!(parsed[0].is_ok());
    assert_eq!(
        parsed[1],
        Err(RuleError::UnknownAction {
            action: "teleport".to_owned()
        })
    );
    assert!(
        parsed[2].is_ok(),
        "a later rule survives an earlier one's error"
    );
}

#[test]
fn unknown_keys_in_a_rule_survive_a_round_trip() {
    // The contract every other section here keeps: a key this version does
    // not know is preserved verbatim, so a newer Postio's file is not
    // silently rewritten by an older one.
    let text = "[[rules]]\nname = \"x\"\nquery = \"from:ada\"\nactions = [\"flag\"]\nfuture = 3\n";
    let written = parse(text).to_toml_string().expect("serializes");
    assert!(written.contains("future"), "{written}");
}

// ------------------------------------------------------------- validation --

fn problems(text: &str) -> Vec<String> {
    postio_config::validate::check_str(text)
        .validation
        .errors()
        .iter()
        .map(|error| format!("{}: {}", error.path, error.message))
        .collect()
}

#[test]
fn an_undefined_filter_reference_is_reported_with_its_line() {
    let text = "[ui]\ndensity = \"compact\"\n\n[[rules]]\nname = \"x\"\nfilter = \"nope\"\nactions = [\"flag\"]\n";
    let checked = postio_config::validate::check_str(text);
    let error = checked
        .validation
        .errors()
        .iter()
        .find(|error| error.path.starts_with("rules"))
        .expect("a rules error");
    assert!(
        error.message.contains("nope"),
        "the message has to name the filter that does not exist: {}",
        error.message
    );
    assert!(
        error.message.contains('x'),
        "and the rule it is in: {}",
        error.message
    );
    assert!(
        error.line >= 4,
        "pointing into the array, not at line 1: {error:?}"
    );
}

#[test]
fn every_way_a_rule_can_be_wrong_is_reported_rather_than_ignored() {
    // One assertion per failure mode, because "some error was reported" is
    // exactly what a section that reports the wrong thing also satisfies.
    let cases = [
        ("query = \"from:ada\"\nactions = []", "no actions"),
        ("query = \"from:ada\"\nactions = [\"teleport\"]", "teleport"),
        ("query = \"from:ada\"\nactions = [\"move\"]", "move"),
        ("query = \"from:ada\"\nactions = [\"flag:x\"]", "flag"),
        ("query = \"from:ada\"\nactions = [\"stop\"]", "stop = true"),
        ("query = \"from:ada\"\nactions = [\"delete\"]", "trash"),
        (
            "query = \"from:ada\"\nactions = [\"forward:Receipts\"]",
            "move:Receipts",
        ),
        ("actions = [\"flag\"]", "query"),
    ];
    for (body, expected) in cases {
        let text = format!("[[rules]]\nname = \"r\"\n{body}\n");
        let found = problems(&text);
        assert!(
            found.iter().any(|problem| problem.contains(expected)),
            "{body:?} should report something containing {expected:?}, got {found:?}"
        );
    }
}

#[test]
fn a_valid_rules_section_reports_nothing() {
    assert!(problems(RULES).is_empty(), "{:?}", problems(RULES));
    assert!(
        postio_config::validate::check_str(RULES)
            .validation
            .is_valid()
    );
}

#[test]
fn a_rule_naming_a_filter_that_exists_is_not_reported() {
    // The control for the test above it: the reference check has to
    // distinguish a name that resolves from one that does not, or "no errors"
    // means "the check does not run".
    let text = "[filters.a]\nquery = \"is:unread\"\n\n[[rules]]\nname = \"r\"\nfilter = \"a\"\nactions = [\"flag\"]\n";
    assert!(problems(text).is_empty(), "{:?}", problems(text));

    let broken = text.replace("filter = \"a\"", "filter = \"b\"");
    assert!(
        !problems(&broken).is_empty(),
        "and the same file with one letter changed is reported"
    );
}

// ------------------------------------------------- when a rule can be run --

/// ADR 0008 Q3's third bullet: a rule the engine has to defer says so.
///
/// The two decisions that half of Q3 already landed (#482) are invisible from
/// the file: a rule containing `body:` is evaluated when the backfill
/// completes rather than on arrival, and the user's evidence for that is mail
/// sitting in the Inbox for as long as the body takes. Without a note, that
/// is indistinguishable from the rule being broken — which is the neighbour
/// of the failure Q6 is arranged against, a rule that *is* running, later
/// than expected, silently.
///
/// It is a note and not an error. The rule is valid, it runs, and the file is
/// not rejected; `is_valid()` must stay true or a correct config stops
/// applying.
#[test]
fn a_rule_that_needs_the_body_says_when_it_will_run() {
    let text = r#"[[rules]]
name    = "receipts"
query   = "body:invoice"
actions = ["move:Receipts"]
"#;
    let checked = postio_config::validate::check_str(text);

    assert!(
        checked.validation.is_valid(),
        "a deferred rule is a working rule: {:?}",
        checked.validation.errors()
    );
    let notes = checked.validation.notes();
    assert_eq!(
        notes.len(),
        1,
        "one note for the one deferred rule: {notes:?}"
    );
    assert!(
        notes[0].message.contains("after the body is fetched"),
        "the note has to say what actually happens, in ADR 0008 Q3's own \
         words: {:?}",
        notes[0].message
    );
    assert!(
        notes[0].path.contains("receipts"),
        "and name the rule it is about: {:?}",
        notes[0].path
    );
}

/// A rule answerable from the headers carries none. A note on every rule is
/// a note nobody reads.
#[test]
fn a_header_only_rule_carries_no_note() {
    let text = r#"[[rules]]
name    = "from-team"
query   = "from:team is:unread"
actions = ["flag"]
"#;
    let checked = postio_config::validate::check_str(text);
    assert!(checked.validation.is_valid());
    assert!(
        checked.validation.notes().is_empty(),
        "{:?}",
        checked.validation.notes()
    );
}

/// `header:` is on the deferred side however its name reads.
///
/// ADR 0025 Q4: the header block arrives *with* the body, so a message whose
/// body is not local has no block to match. This is the case a user would
/// never guess, which is the one most worth a note.
#[test]
fn a_header_rule_is_deferred_too_and_says_so() {
    let text = r#"[[rules]]
name    = "mailer"
query   = "header:x-mailer"
actions = ["flag"]
"#;
    let checked = postio_config::validate::check_str(text);
    assert!(checked.validation.is_valid());
    assert_eq!(
        checked.validation.notes().len(),
        1,
        "`header:` cannot be answered on arrival either: {:?}",
        checked.validation.notes()
    );
}

/// A rule that is not a rule produces its error and no note: a note about
/// when a broken rule would run is noise on top of the thing to fix.
#[test]
fn a_broken_rule_gets_its_error_and_no_note() {
    let text = r#"[[rules]]
name    = "nameless"
query   = "body:invoice"
"#;
    let checked = postio_config::validate::check_str(text);
    assert!(!checked.validation.is_valid(), "a rule with no actions");
    assert!(
        checked.validation.notes().is_empty(),
        "{:?}",
        checked.validation.notes()
    );
}
