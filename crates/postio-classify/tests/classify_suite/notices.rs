use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use postio_classify::{
    BodyMessage, Facts, FiledMessage, MarkedBy, MarkerKind, OwnText, Rules, Senders, at_body,
    considered, noticed,
};
use postio_model::{AccountId, EmailAddress, Identity, MailboxId, MailboxRole, Message, ThreadId};
use serde::Deserialize;

const DATASET: &str = include_str!("../data/notices.toml");

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
    addressing: String,
    text: String,
    label: String,
    quote: Option<String>,
    due: Option<String>,
}

fn dataset() -> Dataset {
    toml::from_str(DATASET).expect("notices.toml parses")
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("a YYYY-MM-DD date")
}

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

struct BuiltIn;

impl Rules for BuiltIn {
    fn senders(&self) -> &Senders {
        Senders::shipped()
    }
}

struct StopsTodos;

impl Rules for StopsTodos {
    fn senders(&self) -> &Senders {
        Senders::shipped()
    }
    fn stops(&self, _: &EmailAddress, kind: MarkerKind) -> bool {
        kind == MarkerKind::Todo
    }
}

fn message_for(addressing: &str, user: &EmailAddress, sent: DateTime<Utc>) -> Message {
    let mut message = Message::new(AccountId::new(1), MailboxId::new(1), sent);
    message.date = Some(sent);
    message.to = vec![user.clone()];
    let service = EmailAddress::new(Some("Service"), "billing@service.example");
    let (from, unsubscribe_offered, automation) = match addressing {
        "automated" => (service, false, postio_model::promoted::AUTO_GENERATED),
        "bulk" => (service, false, postio_model::promoted::BULK_SENDER),
        "table" => (
            EmailAddress::new(Some("Statements"), "statements@email.bank.example"),
            false,
            0,
        ),
        "promotion" => (service, true, postio_model::promoted::BULK_SENDER),
        "list" => {
            message.list_id = Some("club.lists.example.org".to_owned());
            (
                EmailAddress::new(Some("Club"), "club@lists.example.org"),
                false,
                postio_model::promoted::PRECEDENCE_LIST,
            )
        }
        "direct" => (
            EmailAddress::new(Some("Mateo Reyes"), "mateo@example.org"),
            false,
            0,
        ),
        other => panic!("addressing {other}"),
    };
    message.from = vec![from];
    message.promoted = Some(postio_model::promoted::PromotedHeaders {
        unsubscribe_offered,
        automation,
    });
    message
}

/// In this machine's zone, which a due day is read in.
fn sent_at(data: &Dataset) -> DateTime<Utc> {
    let noon = date(&data.sent).and_hms_opt(12, 0, 0).expect("noon");
    Local
        .from_local_datetime(&noon)
        .single()
        .expect("noon is a real local time")
        .with_timezone(&Utc)
}

fn identities(data: &Dataset) -> [Identity; 1] {
    [Identity::new(
        AccountId::new(1),
        EmailAddress::new(Some(data.user.name.as_str()), data.user.address.as_str()),
    )]
}

fn own_text(text: &str) -> String {
    postio_body::own_text(&postio_model::MessageBody {
        text: Some(text.to_owned()),
        html: None,
    })
}

/// The quote and due day of the notice marker made, if one was.
fn notice(item: &Item, data: &Dataset, rules: &dyn Rules) -> Option<(String, Option<NaiveDate>)> {
    let identities = identities(data);
    let message = message_for(&item.addressing, &identities[0].address, sent_at(data));
    let body = BodyMessage {
        filed: FiledMessage {
            message: &message,
            thread: None,
            role: MailboxRole::Inbox,
        },
        identities: &identities,
    };
    let own = own_text(&item.text);
    let text = OwnText::new(&own);
    let marker = at_body(&body, &text, &NoGuards, rules).marker?;
    if marker.by != MarkedBy::Notice {
        return None;
    }
    assert_eq!(
        marker.kind,
        MarkerKind::Todo,
        "{}: a notice is a to-do",
        item.id
    );
    let span = marker.span.expect("a notice quotes its clause");
    Some((
        own.chars().skip(span.start).take(span.len()).collect(),
        marker
            .due_at
            .map(|due| due.with_timezone(&Local).date_naive()),
    ))
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn every_notice_is_marked_with_its_clause_and_day_and_nothing_else_is() {
    let data = dataset();
    assert!(
        data.message
            .iter()
            .filter(|item| item.label == "todo")
            .count()
            >= 8
    );
    let mut wrong = Vec::new();
    for item in &data.message {
        let got = notice(item, &data, &BuiltIn).map(|(quote, due)| (normalized(&quote), due));
        let expected = (item.label == "todo").then(|| {
            (
                normalized(item.quote.as_deref().expect("a to-do quotes")),
                item.due.as_deref().map(date),
            )
        });
        if got != expected {
            wrong.push(format!("{}: expected {expected:?}, got {got:?}", item.id));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_notice_is_read_only_where_a_person_is_not() {
    let data = dataset();
    let identities = identities(&data);
    for (addressing, person, notice) in [
        ("direct", true, false),
        ("automated", false, true),
        ("bulk", false, true),
        ("table", false, true),
        ("promotion", false, false),
        ("list", false, false),
    ] {
        let message = message_for(addressing, &identities[0].address, sent_at(&data));
        let body = BodyMessage {
            filed: FiledMessage {
                message: &message,
                thread: None,
                role: MailboxRole::Inbox,
            },
            identities: &identities,
        };
        assert_eq!(considered(&body, &BuiltIn), person, "{addressing}");
        assert_eq!(noticed(&body, &BuiltIn), notice, "{addressing}");
    }
}

#[test]
fn a_sender_whose_to_dos_were_stopped_gets_no_notice() {
    let data = dataset();
    let item = data
        .message
        .iter()
        .find(|item| item.id == "payment-due-by")
        .expect("the item");
    assert!(notice(item, &data, &BuiltIn).is_some());
    assert_eq!(notice(item, &data, &StopsTodos), None);
}
