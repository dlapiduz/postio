//! SC-013's gate (`specs/007-postio-focus` T116, research R10): the labelled
//! needs-action dataset, through the classifier's real body stage.
//!
//! Every item becomes a message addressed the way the item says -- to the
//! user, copied to them, through a list, from an automated sender -- with its
//! text cut down to the newest message's own words. It is then handed to
//! [`postio_classify::at_body`] exactly as the body stage hands a message,
//! and what comes back is compared with the item's label.
//!
//! **The gate is precision.** At least 9 in 10 of the markers made must be
//! right: the kind, and the sentence quoted. A detector that marks nothing
//! has measured nothing, and fails. Recall is reported, not gated (SC-013).
//!
//! The dataset and the rules have one author, so read the numbers as a best
//! case: real mail will do worse (spike S4, research R10). Two numbers depend
//! on more than the detector: the own text is cut by a stand-in until T115
//! lands (below), and each item's addressing is modelled by the headers
//! [`message_for`] gives it.
//!
//! `cargo test -p postio-classify --test needs_action -- --nocapture` prints
//! the report.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use postio_classify::{BodyMessage, Facts, FiledMessage, MarkerKind, OwnText, Rules, at_body};
use postio_model::{AccountId, EmailAddress, Identity, MailboxId, MailboxRole, Message, ThreadId};
use serde::Deserialize;

const DATASET: &str = include_str!("data/needs_action.toml");

// --- The dataset ------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Dataset {
    sent: String,
    user: User,
    message: Vec<Item>,
}

#[derive(Debug, Deserialize)]
struct User {
    name: String,
    address: String,
}

#[derive(Debug, Deserialize)]
struct Item {
    id: String,
    case: String,
    addressing: String,
    text: String,
    label: String,
    quote: Option<String>,
    due: Option<String>,
    #[serde(default)]
    also: Vec<Also>,
}

/// Another genuine ask in the same message: not the marker R10's precedence
/// picks, but not a wrong one either.
#[derive(Debug, Deserialize)]
struct Also {
    label: String,
    quote: String,
}

fn dataset() -> Dataset {
    toml::from_str(DATASET).expect("needs_action.toml parses")
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("a YYYY-MM-DD date")
}

/// Whitespace runs collapsed and the sentence's closing `.`/`!` dropped: a
/// quote is compared by its words, not by how the text was wrapped.
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(['.', '!'])
        .to_owned()
}

// --- The dataset's own invariants ----------------------------------------------------

#[test]
fn the_dataset_is_big_enough_and_covers_every_case_it_promises() {
    let data = dataset();
    assert!(data.message.len() >= 150, "{} items", data.message.len());

    let mut cases: BTreeMap<&str, usize> = BTreeMap::new();
    for item in &data.message {
        *cases.entry(item.case.as_str()).or_default() += 1;
    }
    for case in [
        "question",
        "todo",
        "pleasantry",
        "rhetorical",
        "quoted",
        "signature",
        "list",
        "copied",
        "automated",
        "instruction",
    ] {
        assert!(
            cases.get(case).copied().unwrap_or(0) >= 5,
            "at least five `{case}` items: {cases:?}"
        );
    }
    for addressing in ["direct", "copied", "list", "automated"] {
        assert!(
            data.message
                .iter()
                .any(|item| item.addressing == addressing),
            "an item addressed `{addressing}`"
        );
    }
}

#[test]
fn every_label_is_well_formed_and_quotes_the_text_verbatim() {
    let data = dataset();
    let mut ids = BTreeSet::new();
    for item in &data.message {
        assert!(ids.insert(item.id.as_str()), "{}: duplicate id", item.id);
        assert!(
            ["direct", "copied", "list", "automated"].contains(&item.addressing.as_str()),
            "{}: addressing {}",
            item.id,
            item.addressing
        );
        match item.label.as_str() {
            "question" | "todo" => {
                assert_eq!(
                    item.addressing, "direct",
                    "{}: only direct mail is marked (FR-106)",
                    item.id
                );
                let quote = item.quote.as_deref().expect("a marked item has a quote");
                assert!(
                    normalized(&item.text).contains(&normalized(quote)),
                    "{}: the quote is not verbatim in the text",
                    item.id
                );
            }
            "none" => assert!(
                item.quote.is_none() && item.due.is_none(),
                "{}: an unmarked item quotes nothing",
                item.id
            ),
            other => panic!("{}: label {other}", item.id),
        }
        if let Some(due) = &item.due {
            assert_eq!(item.label, "todo", "{}: only a to-do is due", item.id);
            assert!(
                date(due) >= date(&data.sent),
                "{}: due before sent",
                item.id
            );
        }
        for also in &item.also {
            assert!(
                item.label != "none" && ["question", "todo"].contains(&also.label.as_str()),
                "{}: an alternative belongs to a marked item",
                item.id
            );
            assert!(
                normalized(&item.text).contains(&normalized(&also.quote)),
                "{}: an alternative quote is not verbatim in the text",
                item.id
            );
        }
    }
}

#[test]
fn every_address_in_the_dataset_is_reserved() {
    let data = dataset();
    assert!(data.user.address.ends_with("@example.com"));
    for item in &data.message {
        for (at, _) in item.text.match_indices('@') {
            let domain: String = item.text[at + 1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
                .collect();
            let domain = domain.trim_end_matches('.');
            let reserved = domain.ends_with(".example")
                || domain.ends_with(".test")
                || domain.ends_with(".invalid")
                || ["example.com", "example.net", "example.org"]
                    .iter()
                    .any(|reserved| {
                        domain == *reserved || domain.ends_with(&format!(".{reserved}"))
                    });
            assert!(reserved, "{}: {domain} is not a reserved domain", item.id);
        }
    }
}

// --- The own text, until T115 -------------------------------------------------------
//
// The detector reads the newest message's own words, which T115 cuts in
// postio-body ("the same boundaries the reader folds", research R10). Until it
// lands, this stands in for it: the approximation spike S4 measured with,
// unchanged, so the gate's number is not flattered by a cutter written after
// reading the data. When T115 lands, the gate reads own text through
// postio-body's extraction, and this goes.

/// A line that closes the message: what follows is the signature.
const CLOSINGS: &[&str] = &[
    "thanks",
    "thank you",
    "many thanks",
    "thanks!",
    "best",
    "best wishes",
    "best regards",
    "kind regards",
    "regards",
    "warm regards",
    "warmly",
    "cheers",
    "love",
    "all the best",
];

/// Quoted lines, forwarded and original messages, the signature and
/// everything after a closing are dropped.
fn own_text(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        let separator = lower.starts_with("-----original message")
            || (lower.starts_with("---") && lower.contains("forwarded message"))
            || (trimmed.len() >= 10 && trimmed.chars().all(|c| c == '_'))
            || lower.starts_with("sent from my")
            || trimmed == "--"
            || line == "-- "
            || CLOSINGS.contains(&lower.trim_end_matches([',', '!', '.']))
            || (lower.starts_with("from: ")
                && lines
                    .get(index + 1)
                    .is_some_and(|next| next.starts_with("Sent:") || next.starts_with("Date:")));
        if separator {
            break;
        }
        if lower.starts_with("on ") && lower.ends_with("wrote:") {
            // Top-posted over a quote: skip the attribution and let the quoted
            // lines drop one by one. Over unquoted text, stop here.
            let next = lines[index + 1..]
                .iter()
                .find(|next| !next.trim().is_empty());
            if next.is_some_and(|next| next.trim_start().starts_with('>')) {
                index += 1;
                continue;
            }
            break;
        }
        if !trimmed.starts_with('>') {
            kept.push(line);
        }
        index += 1;
    }
    kept.join("\n")
}

// --- Each item as a message ------------------------------------------------------

/// No guard applies: they decide filtering, not markers.
struct NoGuards;

impl Facts for NoGuards {
    fn wrote_to(&self, _: &EmailAddress) -> bool {
        false
    }
    fn took_part(&self, _: ThreadId) -> bool {
        false
    }
    fn own_domain(&self, _: &EmailAddress) -> bool {
        false
    }
    fn never_filter(&self, _: &EmailAddress) -> bool {
        false
    }
}

/// What the body stage is handed in production.
struct BuiltIn;

impl Rules for BuiltIn {}

/// The headers each kind of addressing arrives with.
struct Addressed {
    message: Message,
    unsubscribe_offered: Option<bool>,
    automation: Option<u8>,
}

/// Who writes to the user in the dataset: one invented correspondent, since
/// an item's text, not its sender, is what it labels.
fn correspondent() -> EmailAddress {
    EmailAddress::new(Some("Correspondent"), "correspondent@example.org")
}

/// The item as mail, with the headers its addressing arrives with.
fn message_for(item: &Item, user: &EmailAddress, sent: DateTime<Utc>) -> Addressed {
    let mut message = Message::new(AccountId::new(1), MailboxId::new(1), sent);
    message.date = Some(sent);
    message.from = vec![correspondent()];
    let (unsubscribe_offered, automation) = match item.addressing.as_str() {
        "direct" => {
            message.to = vec![user.clone()];
            (Some(false), Some(0))
        }
        "copied" => {
            message.to = vec![EmailAddress::new(
                Some("Colleague"),
                "colleague@example.org",
            )];
            message.cc = vec![user.clone()];
            (Some(false), Some(0))
        }
        "list" => {
            message.to = vec![EmailAddress::new(None::<&str>, "team@lists.example.org")];
            message.list_id = Some("team.lists.example.org".to_owned());
            // `Precedence: list`, and the list's own unsubscribe link.
            (Some(true), Some(2))
        }
        "automated" => {
            message.from = vec![EmailAddress::new(
                Some("Service"),
                "notifications@service.example",
            )];
            message.to = vec![user.clone()];
            // `Auto-Submitted: auto-generated`.
            (Some(false), Some(8))
        }
        other => panic!("{}: addressing {other}", item.id),
    };
    Addressed {
        message,
        unsubscribe_offered,
        automation,
    }
}

fn identity(data: &Dataset) -> Identity {
    let mut identity = Identity::new(
        AccountId::new(1),
        EmailAddress::new(Some(data.user.name.as_str()), data.user.address.as_str()),
    );
    identity.display_name = data.user.name.clone();
    identity
}

/// Noon on the dataset's day, in this machine's zone: the zone the detector
/// reads "by Friday" in, so the due day compares in it too.
fn sent_at(data: &Dataset) -> DateTime<Utc> {
    let noon = date(&data.sent).and_hms_opt(12, 0, 0).expect("noon");
    Local
        .from_local_datetime(&noon)
        .single()
        .expect("noon is a real local time")
        .with_timezone(&Utc)
}

/// What the classifier marked on one item: its kind, the sentence it quotes
/// (cut from the own text by the marker's span), and its due day.
struct Marked {
    kind: &'static str,
    quote: String,
    due: Option<NaiveDate>,
}

fn classify(item: &Item, data: &Dataset) -> Option<Marked> {
    let identities = [identity(data)];
    let addressed = message_for(item, &identities[0].address, sent_at(data));
    let body = BodyMessage {
        filed: FiledMessage {
            message: &addressed.message,
            mailbox: Some(MailboxRole::Inbox),
            unsubscribe_offered: addressed.unsubscribe_offered,
            automation: addressed.automation,
            has_calendar: false,
        },
        identities: &identities,
    };
    let own = own_text(&item.text);
    let text = OwnText::new(&own);
    let marker = at_body(&body, &text, &NoGuards, &BuiltIn).marker?;

    let kind = match marker.kind {
        MarkerKind::Question => "question",
        MarkerKind::Todo => "todo",
        MarkerKind::Invite => panic!("{}: an invitation made from text", item.id),
    };
    let span = marker
        .span
        .unwrap_or_else(|| panic!("{}: a {kind} with no span", item.id));
    assert!(
        span.start < span.end && span.end <= text.len_chars(),
        "{}: span {span:?} is not inside the own text",
        item.id
    );
    assert!(
        kind == "todo" || marker.due_at.is_none(),
        "{}: a question carries no due date",
        item.id
    );
    Some(Marked {
        kind,
        quote: own.chars().skip(span.start).take(span.len()).collect(),
        due: marker
            .due_at
            .map(|due| due.with_timezone(&Local).date_naive()),
    })
}

// --- The measurement ---------------------------------------------------------------

/// Markers made and right, for one kind or one case.
#[derive(Debug, Default)]
struct Tally {
    items: usize,
    labelled: usize,
    made: usize,
    right: usize,
}

#[derive(Debug, Default)]
struct Score {
    predicted: usize,
    right: usize,
    /// Right, or one of the item's other genuine asks.
    right_or_also: usize,
    labelled: usize,
    due_expected: usize,
    due_right: usize,
    by_kind: BTreeMap<&'static str, Tally>,
    by_case: BTreeMap<String, Tally>,
    errors: Vec<String>,
}

impl Score {
    /// Right markers over markers made. With none made it is 0, not
    /// undefined: a detector that says nothing has not been measured.
    fn precision(&self) -> f64 {
        self.right as f64 / self.predicted.max(1) as f64
    }
    fn lenient_precision(&self) -> f64 {
        self.right_or_also as f64 / self.predicted.max(1) as f64
    }
    fn recall(&self) -> f64 {
        self.right as f64 / self.labelled.max(1) as f64
    }
}

fn measure() -> Score {
    let data = dataset();
    let mut score = Score::default();
    for item in &data.message {
        let got = classify(item, &data);
        let marked = item.label != "none";
        let case = score.by_case.entry(item.case.clone()).or_default();
        case.items += 1;
        if marked {
            case.labelled += 1;
            score.labelled += 1;
            let kind = if item.label == "question" {
                "question"
            } else {
                "todo"
            };
            score.by_kind.entry(kind).or_default().labelled += 1;
        }
        let expected = || {
            format!(
                "{} {:?}{}",
                item.label,
                item.quote.as_deref().unwrap_or(""),
                item.due
                    .as_deref()
                    .map(|due| format!(" due {due}"))
                    .unwrap_or_default()
            )
        };
        let Some(marker) = got else {
            if marked {
                score.errors.push(format!(
                    "MISS  {:<34} [{}] expected {}",
                    item.id,
                    item.case,
                    expected()
                ));
            }
            continue;
        };
        score.predicted += 1;
        score.by_case.entry(item.case.clone()).or_default().made += 1;
        score.by_kind.entry(marker.kind).or_default().made += 1;
        let kind_right = marker.kind == item.label;
        let quote_right = item
            .quote
            .as_deref()
            .is_some_and(|quote| normalized(quote) == normalized(&marker.quote));
        let got_text = format!(
            "{} {:?}{}",
            marker.kind,
            marker.quote,
            marker
                .due
                .map(|due| format!(" due {due}"))
                .unwrap_or_default()
        );
        let an_also = item.also.iter().any(|also| {
            also.label == marker.kind && normalized(&also.quote) == normalized(&marker.quote)
        });
        if (kind_right && quote_right) || an_also {
            score.right_or_also += 1;
        }
        if kind_right && quote_right {
            score.right += 1;
            score.by_case.entry(item.case.clone()).or_default().right += 1;
            score.by_kind.entry(marker.kind).or_default().right += 1;
            let due_expected = item.due.as_deref().map(date);
            if due_expected.is_some() {
                score.due_expected += 1;
            }
            if marker.due == due_expected {
                if due_expected.is_some() {
                    score.due_right += 1;
                }
            } else {
                score.errors.push(format!(
                    "DUE   {:<34} [{}] expected {} got {}",
                    item.id,
                    item.case,
                    expected(),
                    got_text
                ));
            }
            continue;
        }
        let tag = if an_also {
            "ALSO "
        } else if marked {
            "WRONG"
        } else {
            "FALSE"
        };
        score.errors.push(format!(
            "{tag} {:<34} [{}] expected {} got {}",
            item.id,
            item.case,
            expected(),
            got_text
        ));
    }
    score
}

fn report(score: &Score) {
    println!("== The built-in detector on the labelled dataset (SC-013) ==");
    println!(
        "markers made {}, right {} -> precision {:.3} (another genuine ask counted right: {:.3})",
        score.predicted,
        score.right,
        score.precision(),
        score.lenient_precision(),
    );
    println!(
        "labelled {} -> recall {:.3}; due dates right {}/{}",
        score.labelled,
        score.recall(),
        score.due_right,
        score.due_expected
    );
    println!();
    println!(
        "{:<16} {:>9} {:>6} {:>6} {:>10} {:>7}",
        "kind", "labelled", "made", "right", "precision", "recall"
    );
    for (kind, tally) in &score.by_kind {
        println!(
            "{:<16} {:>9} {:>6} {:>6} {:>10.3} {:>7.3}",
            kind,
            tally.labelled,
            tally.made,
            tally.right,
            tally.right as f64 / tally.made.max(1) as f64,
            tally.right as f64 / tally.labelled.max(1) as f64,
        );
    }
    println!();
    println!(
        "{:<16} {:>6} {:>9} {:>6} {:>6} {:>6}",
        "case", "items", "labelled", "made", "right", "wrong"
    );
    for (case, tally) in &score.by_case {
        println!(
            "{:<16} {:>6} {:>9} {:>6} {:>6} {:>6}",
            case,
            tally.items,
            tally.labelled,
            tally.made,
            tally.right,
            tally.made - tally.right,
        );
    }
    println!();
    for error in &score.errors {
        println!("  {error}");
    }
    println!();
}

#[test]
fn the_built_in_detector_is_right_nine_times_in_ten() {
    let score = measure();
    report(&score);

    assert!(
        score.precision() >= 0.9,
        "SC-013: {} of {} markers right, precision {:.3} < 0.9 \
         (run with --nocapture for the report)",
        score.right,
        score.predicted,
        score.precision()
    );
}
