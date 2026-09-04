//! Which rules run at which of the two evaluation points (ADR 0008 Q3, #482).
//!
//! The classification is derived, never declared: a rule says what it
//! matches and the engine works out whether that can be answered from the
//! headers alone. Getting it wrong in either direction loses mail — a
//! body-requiring rule evaluated on arrival matches against an absent body
//! and files on `false`, and a header-only rule deferred to the backfill
//! leaves the message in the Inbox until a body arrives that it never needed.

use chrono::{NaiveDate, TimeZone, Utc};
use postio_model::rule::{Rule, RuleSource};
use postio_model::{EmailAddress, Message};
use postio_search::matcher::Subject;
use postio_search::rules::{RuleSet, Stage};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 8, 22).expect("a real date")
}

/// One rule, from the text a `[[rules]]` entry would carry.
fn rule(name: &str, query: &str) -> Rule {
    Rule::parse(
        &RuleSource {
            name: name.to_owned(),
            query: Some(query.to_owned()),
            actions: vec!["flag".to_owned()],
            ..RuleSource::default()
        },
        |_| None,
    )
    .expect("a rule")
}

fn disabled(name: &str, query: &str) -> Rule {
    Rule {
        enabled: false,
        ..rule(name, query)
    }
}

fn a_message() -> Message {
    let mut message = Message::new(
        postio_model::AccountId::new(1),
        postio_model::MailboxId::new(1),
        Utc.with_ymd_and_hms(2026, 8, 20, 9, 0, 0).unwrap(),
    );
    message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    message.subject = Some("Quarterly report".to_string());
    message
}

#[test]
fn a_rules_stage_comes_from_the_fields_its_query_uses() {
    let rules = [
        rule("headers", "from:ada is:unread"),
        rule("body", "body:invoice"),
        rule("mixed", "from:ada OR body:invoice"),
        rule("header-operator", "header:x-mailer"),
        rule("free-text", "turbine"),
    ];
    let set = RuleSet::compile(&rules, today());

    let stages: Vec<(&str, Stage)> = set
        .rules()
        .iter()
        .map(|staged| (staged.rule.name.as_str(), staged.stage))
        .collect();

    assert_eq!(
        stages,
        vec![
            ("headers", Stage::OnArrival),
            ("body", Stage::OnBody),
            // One body-touching clause is enough: the query cannot be
            // answered until the body is there, whatever else it says.
            ("mixed", Stage::OnBody),
            // ADR 0025 Q4: the header block arrives *with* the body, so
            // `header:` cannot be answered on arrival however its name reads.
            ("header-operator", Stage::OnBody),
            // Free text reaches the body index as well as the metadata one.
            ("free-text", Stage::OnBody),
        ],
        "the stage is derived from the fields the query uses, never declared"
    );
}

#[test]
fn a_disabled_rule_is_not_in_the_set_at_all() {
    let set = RuleSet::compile(
        &[rule("live", "from:ada"), disabled("dry-run", "from:ada")],
        today(),
    );

    let names: Vec<&str> = set
        .rules()
        .iter()
        .map(|staged| staged.rule.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["live"],
        "`enabled = false` is how a rule is turned off, and a caller that has \
         to remember to check it is one that eventually does not"
    );
}

#[test]
fn each_point_sees_only_the_rules_it_can_answer() {
    let rules = [
        rule("on-arrival", "from:ada"),
        rule("on-body", "body:report"),
    ];
    let set = RuleSet::compile(&rules, today());
    let message = a_message();

    // The arrival point, with no body: the header rule matches and the body
    // rule is not even considered — which is the whole point, since with an
    // absent body it would evaluate to `false` and file the mail on it.
    let subject = Subject::new(&message);
    let fired: Vec<&str> = set
        .matching(Stage::OnArrival, &subject)
        .into_iter()
        .map(|rule| rule.name.as_str())
        .collect();
    assert_eq!(fired, vec!["on-arrival"]);

    // The body point, with the body now local.
    let subject = Subject::new(&message).with_body(Some("the report is attached"));
    let fired: Vec<&str> = set
        .matching(Stage::OnBody, &subject)
        .into_iter()
        .map(|rule| rule.name.as_str())
        .collect();
    assert_eq!(fired, vec!["on-body"]);

    // And a message is never evaluated against the same rule at both: the
    // stages partition the set, so "exactly once" is the shape rather than
    // something a table has to enforce.
    assert!(
        set.matching(Stage::OnBody, &subject).iter().all(|rule| !set
            .matching(Stage::OnArrival, &subject)
            .iter()
            .any(|other| other.name == rule.name)),
        "a rule reached both points, so one message would fire it twice"
    );
}

#[test]
fn a_point_with_no_rules_says_so_before_any_work_is_done() {
    let set = RuleSet::compile(&[rule("headers", "from:ada")], today());
    assert!(set.has(Stage::OnArrival));
    assert!(
        !set.has(Stage::OnBody),
        "with no body-requiring rule configured, the backfill point must not \
         read a body back out of the store to match nothing against it"
    );
}
