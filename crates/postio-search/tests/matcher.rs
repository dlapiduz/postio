//! The in-memory matcher, operator by operator (ADR 0008 Q1).
//!
//! The second evaluator of one query language. A rule fires on **one** message
//! as it arrives, before it is committed anywhere a query could see, so it
//! cannot be answered by the FTS5 executor — and two evaluators of one
//! language that disagree is the worst outcome available here, worse than not
//! having rules, because a dry-run would show one answer and the rule would do
//! another.
//!
//! These are the matcher's own semantics, asserted where they can be read.
//! That the *executor* agrees with them is `postio-index`'s
//! `differential.rs`, which is the test this design is safe because of.

use chrono::{TimeZone, Utc};
use postio_model::{Attachment, EmailAddress, Flag, Message};
use postio_search::matcher::{Subject, matches};
use postio_search::parse;

fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 8, 22).expect("a real date")
}

fn a_message() -> Message {
    let mut message = Message::new(
        postio_model::AccountId::new(1),
        postio_model::MailboxId::new(1),
        Utc.with_ymd_and_hms(2026, 8, 20, 9, 0, 0).unwrap(),
    );
    message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    message.to = vec![EmailAddress::new(Some("Grace Hopper"), "grace@example.com")];
    message.subject = Some("Quarterly report".to_string());
    message.size = 4096;
    message.headers = [("X-Mailer", "Mutt 1.5.24 (2015-08-30)")]
        .into_iter()
        .collect();
    message
}

/// Whether `query` selects `message`, with no facts from outside it.
fn hit(query: &str, message: &Message) -> bool {
    matches(&parse(query, today()), &Subject::new(message))
}

// ---------------------------------------------------------------------------
// The metadata operators
// ---------------------------------------------------------------------------

#[test]
fn from_matches_the_display_name_or_the_address() {
    let message = a_message();
    assert!(hit("from:ada", &message));
    assert!(hit("from:lovelace", &message), "the display name too");
    assert!(hit("from:ada@example.com", &message));
    assert!(
        !hit("from:grace", &message),
        "grace is a recipient, not the sender"
    );
}

#[test]
fn to_covers_cc_and_bcc_as_well_as_to() {
    let mut message = a_message();
    message.cc = vec![EmailAddress::new(None::<String>, "carol@example.com")];
    message.bcc = vec![EmailAddress::new(None::<String>, "dan@example.com")];
    assert!(hit("to:grace", &message));
    assert!(hit("to:carol", &message));
    assert!(hit("to:dan", &message));
    assert!(!hit("to:ada", &message), "the sender is not a recipient");
}

#[test]
fn a_value_matches_whole_tokens_and_not_arbitrary_substrings() {
    // The executor asks FTS5, which indexes tokens. `from:ad` finding
    // `ada@example.com` in one evaluator and not the other is exactly the
    // divergence this whole design is arranged to prevent.
    let message = a_message();
    assert!(!hit("from:ad", &message), "a prefix is not a token");
    assert!(!hit("subject:quarter", &message), "nor here");
    assert!(hit("subject:quarterly", &message));
}

#[test]
fn a_quoted_value_matches_a_phrase_in_order() {
    let message = a_message();
    assert!(hit(r#"subject:"quarterly report""#, &message));
    assert!(
        !hit(r#"subject:"report quarterly""#, &message),
        "a phrase is a sequence, not a set"
    );
}

#[test]
fn filename_matches_an_attachment_and_has_attach_asks_whether_there_is_one() {
    let mut message = a_message();
    assert!(!hit("has:attach", &message));
    let mut attachment = Attachment::new(message.id, "application/pdf", 1024);
    attachment.filename = Some("invoice-august.pdf".to_string());
    message.attachments = vec![attachment];
    assert!(hit("has:attach", &message));
    assert!(hit("filename:invoice", &message));
    assert!(!hit("filename:contract", &message));
}

#[test]
fn list_matches_the_list_id_header_rather_than_a_recipient() {
    let mut message = a_message();
    message.list_id = Some("harbour-dev.lists.example.org".to_string());
    assert!(hit("list:harbour", &message));
    assert!(!hit("list:ada", &message));
}

#[test]
fn is_reads_the_flags() {
    let mut message = a_message();
    assert!(hit("is:unread", &message));
    assert!(!hit("is:read", &message));
    message.flags.insert(Flag::Seen);
    assert!(hit("is:read", &message));
    assert!(!hit("is:unread", &message));
    assert!(!hit("is:flagged", &message));
    message.flags.insert(Flag::Flagged);
    assert!(hit("is:flagged", &message));
}

#[test]
fn dates_are_half_open_the_way_the_executor_compares_them() {
    // `after:` is inclusive from the start of that day and `before:` is
    // strictly earlier than the start of its day — the same two comparisons
    // `filter_condition` makes against `received_at`.
    let message = a_message(); // received 2026-08-20 09:00 UTC
    assert!(
        hit("after:2026-08-20", &message),
        "the day it arrived counts"
    );
    assert!(hit("after:2026-08-19", &message));
    assert!(!hit("after:2026-08-21", &message));
    assert!(hit("before:2026-08-21", &message));
    assert!(
        !hit("before:2026-08-20", &message),
        "before its own day is strictly earlier, so not a hit"
    );
}

#[test]
fn sizes_are_inclusive_at_both_ends() {
    let message = a_message(); // 4096 bytes
    assert!(hit("larger:4096", &message));
    assert!(hit("smaller:4096", &message));
    assert!(!hit("larger:4097", &message));
    assert!(!hit("smaller:4095", &message));
}

// ---------------------------------------------------------------------------
// `header:` — ADR 0025's operator, and the reason both evaluators normalize
// ---------------------------------------------------------------------------

#[test]
fn header_asks_presence_or_what_the_value_contains() {
    let message = a_message();
    assert!(hit("header:x-mailer", &message));
    assert!(
        hit("header:X-MAILER", &message),
        "names are case-insensitive"
    );
    assert!(hit("header:x-mailer=mutt", &message), "and so are values");
    assert!(
        hit("header:x-mailer=1.5.24", &message),
        "a substring, not a token"
    );
    assert!(!hit("header:x-mailer=pine", &message));
    assert!(!hit("header:x-spam-status", &message));
}

#[test]
fn a_header_name_is_matched_exactly_and_never_as_a_substring() {
    let message = a_message();
    assert!(!hit("header:x-mail", &message));
    assert!(!hit("header:mailer", &message));
}

#[test]
fn a_name_from_one_header_never_pairs_with_a_value_from_another() {
    let mut message = a_message();
    message.headers = [("X-Mailer", "Mutt 1.5.24"), ("Precedence", "bulk")]
        .into_iter()
        .collect();
    assert!(!hit("header:x-mailer=bulk", &message));
    assert!(!hit("header:precedence=mutt", &message));
    assert!(hit("header:precedence=bulk", &message));
}

#[test]
fn a_long_header_is_matched_against_the_prefix_the_index_holds() {
    // ADR 0025 Q3's correctness hazard, stated from the matcher's side. The
    // index holds `VALUE_LIMIT` bytes; a matcher that held the whole value
    // would find a message the executor could not, for the one class of
    // header — signatures, spam reports — where values run long.
    let mut message = a_message();
    let mut value = "x".repeat(postio_model::headers::VALUE_LIMIT);
    value.push_str("needle");
    message.headers = [("DKIM-Signature", value.as_str())].into_iter().collect();
    assert!(
        !hit("header:dkim-signature=needle", &message),
        "past the cap is past what either evaluator can see"
    );
    assert!(hit("header:dkim-signature=xxx", &message));
}

// ---------------------------------------------------------------------------
// `body:`, and what an absent body means
// ---------------------------------------------------------------------------

#[test]
fn body_matches_the_text_and_free_text_matches_either_half() {
    let message = a_message();
    let subject = Subject::new(&message).with_body(Some("the turbine schedule is attached"));

    assert!(matches(&parse("body:turbine", today()), &subject));
    assert!(!matches(&parse("body:quarterly", today()), &subject));
    assert!(
        matches(&parse("quarterly", today()), &subject),
        "free text reaches the metadata as well as the body"
    );
    assert!(matches(&parse("turbine", today()), &subject));
}

#[test]
fn a_body_that_is_not_local_yet_matches_nothing_rather_than_everything() {
    // ADR 0008 Q3: evaluating a `body:` rule against an absent body silently
    // makes it false, which files mail in the wrong place. The engine is what
    // must not ask — `needs_body` is how it knows — and the matcher's own
    // answer here is the conservative one either way.
    let message = a_message();
    let subject = Subject::new(&message);
    assert!(!matches(&parse("body:turbine", today()), &subject));
    assert!(
        matches(&parse("quarterly", today()), &subject),
        "free text can still be answered from the metadata"
    );
}

#[test]
fn needs_body_is_computed_from_the_fields_a_query_uses() {
    use postio_search::matcher::needs_body;
    assert!(!needs_body(&parse("from:ada is:unread", today())));
    assert!(needs_body(&parse("body:invoice", today())));
    assert!(
        needs_body(&parse("turbine", today())),
        "free text reaches the body, so it cannot be answered on arrival either"
    );
    assert!(
        needs_body(&parse("header:x-mailer", today())),
        "ADR 0025 Q4: headers arrive with the body, so `header:` is NEEDS_BODY \
         however its name reads"
    );
    assert!(needs_body(&parse("from:ada OR body:invoice", today())));
}

// ---------------------------------------------------------------------------
// Composition: negation, conjunction, disjunction, grouping
// ---------------------------------------------------------------------------

#[test]
fn adjacent_operators_are_conjoined_and_a_dash_negates() {
    let message = a_message();
    assert!(hit("from:ada subject:quarterly", &message));
    assert!(!hit("from:ada subject:lunch", &message));
    assert!(hit("from:ada -subject:lunch", &message));
    assert!(!hit("from:ada -subject:quarterly", &message));
}

#[test]
fn or_is_a_union_and_binds_looser_than_adjacency() {
    let message = a_message();
    assert!(hit("from:grace OR from:ada", &message));
    assert!(!hit("from:grace OR from:carol", &message));
    // `from:grace OR from:ada has:attach` is *grace, or ada-with-an-attachment*.
    assert!(
        !hit("from:grace OR from:ada has:attach", &message),
        "OR binds looser, so the right arm is the whole conjunction"
    );
}

#[test]
fn parentheses_override_the_precedence() {
    let message = a_message();
    assert!(
        hit("(from:grace OR from:ada) subject:quarterly", &message),
        "grouped, the disjunction is one conjunct"
    );
    assert!(!hit(
        "(from:grace OR from:carol) subject:quarterly",
        &message
    ));
}

#[test]
fn an_empty_query_selects_everything() {
    let message = a_message();
    assert!(hit("", &message));
    assert!(hit("   ", &message));
    assert!(
        hit("is:", &message),
        "a half-typed operator constrains nothing, exactly as it does in SQL"
    );
}

// ---------------------------------------------------------------------------
// The facts that live outside the message
// ---------------------------------------------------------------------------

#[test]
fn in_account_and_group_are_matched_against_the_names_they_are_given() {
    let message = a_message();
    let subject = Subject::new(&message)
        .in_mailbox(&["Archive", "INBOX/Archive", "archive"])
        .in_account(&["Work", "ada@example.com"])
        .with_groups(&["family", "colleagues"]);

    assert!(matches(&parse("in:archive", today()), &subject));
    assert!(matches(&parse("in:\"INBOX/Archive\"", today()), &subject));
    assert!(matches(&parse("account:work", today()), &subject));
    assert!(matches(&parse("group:colleagues", today()), &subject));
    assert!(!matches(&parse("in:inbox", today()), &subject));
    assert!(
        !matches(&parse("account:personal", today()), &subject),
        "a name that resolves to nothing matches nothing, never everything"
    );
}

#[test]
fn an_unresolvable_name_matches_nothing_rather_than_everything() {
    // The rule `filter_condition` states three times: an empty `IN` set
    // matches nothing. A matcher that treated "no names supplied" as
    // unconstrained would turn a narrowing rule into one that fires on
    // every message.
    let message = a_message();
    let subject = Subject::new(&message);
    assert!(!matches(&parse("in:archive", today()), &subject));
    assert!(!matches(&parse("account:work", today()), &subject));
    assert!(!matches(&parse("group:family", today()), &subject));
}
