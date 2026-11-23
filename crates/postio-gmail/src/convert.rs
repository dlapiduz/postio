use chrono::{DateTime, Utc};
use io_gmail::v1::rest::labels::GmailLabel;
use io_gmail::v1::rest::labels::GmailLabelType;
use io_gmail::v1::rest::messages::{GmailMessage, GmailMessageHeader};
use postio_account::backend::{Envelope, FetchedMessage, FlagChange, MailboxSummary};
use postio_model::{EmailAddress, Flag, FlagSet, RemoteId, RfcMessageId, Uid, UidValidity};

/// Ids never renumber, so there is exactly one.
pub(crate) const GENERATION: u32 = 0;

/// Not a Gmail label: moving here removes labels, and listing it is a search (ADR 0018 Q4).
pub(crate) const ARCHIVE_PATH: &str = "Archive";

pub(crate) const ARCHIVE_QUERY: &str = "-in:inbox -in:sent -in:draft -in:trash -in:spam";

const SYSTEM: &[(&str, &str, &str)] = &[
    ("INBOX", "Inbox", ""),
    ("SENT", "Sent", "\\Sent"),
    ("DRAFT", "Drafts", "\\Drafts"),
    ("TRASH", "Trash", "\\Trash"),
    ("SPAM", "Junk", "\\Junk"),
];

pub(crate) fn mailboxes(labels: &[GmailLabel]) -> Vec<MailboxSummary> {
    let mut summaries = Vec::new();
    for (id, path, attribute) in SYSTEM {
        if labels.iter().any(|label| label.id == *id) {
            let attributes: Vec<String> = if attribute.is_empty() {
                Vec::new()
            } else {
                vec![(*attribute).to_owned()]
            };
            summaries.push(MailboxSummary::new(*path, None, attributes));
        }
    }
    summaries.push(MailboxSummary::new(
        ARCHIVE_PATH,
        None,
        vec!["\\Archive".to_owned()],
    ));
    for label in labels {
        if label.label_type == Some(GmailLabelType::User) && !label.name.is_empty() {
            summaries.push(MailboxSummary::new(
                label.name.clone(),
                Some('/'),
                Vec::<String>::new(),
            ));
        }
    }
    summaries
}

/// `None` for the archive, which is the absence of labels.
pub(crate) fn label_id(path: &str, labels: &[GmailLabel]) -> Option<String> {
    for (id, known, _) in SYSTEM {
        if *known == path {
            return Some((*id).to_owned());
        }
    }
    labels
        .iter()
        .find(|label| label.name == path)
        .map(|label| label.id.clone())
}

pub(crate) fn fetched(message: &GmailMessage, position: u32) -> Option<FetchedMessage> {
    if message.id.is_empty() {
        return None;
    }
    let headers = message
        .payload
        .as_ref()
        .map(|payload| payload.headers.as_slice())
        .unwrap_or_default();
    Some(FetchedMessage {
        content_identity: Some(postio_model::ContentIdentity::new(
            "gmail-message",
            message.id.clone(),
        )),
        remote_id: RemoteId::new(message.id.clone()),
        uid: Uid::new(position),
        uid_validity: UidValidity::new(GENERATION),
        mod_seq: None,
        flags: flags(&message.label_ids),
        internal_date: message
            .internal_date
            .as_deref()
            .and_then(|millis| millis.parse::<i64>().ok())
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .unwrap_or_else(Utc::now),
        size: message.size_estimate.unwrap_or_default(),
        envelope: Some(envelope(headers)),
        structure: None,
        promoted: Some(promoted(headers)),
    })
}

fn promoted(headers: &[GmailMessageHeader]) -> postio_model::promoted::PromotedHeaders {
    let mut all = postio_model::Headers::new();
    for header in headers {
        all.push(header.name.clone(), header.value.clone());
    }
    postio_model::promoted::PromotedHeaders::from_headers(&all)
}

fn envelope(headers: &[GmailMessageHeader]) -> Envelope {
    let header = |name: &str| {
        headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.clone())
    };
    Envelope {
        date: header("Date").and_then(|value| {
            DateTime::parse_from_rfc2822(&value)
                .ok()
                .map(|date| date.with_timezone(&Utc))
        }),
        subject: header("Subject"),
        from: addresses(header("From")),
        sender: addresses(header("Sender")).into_iter().next(),
        reply_to: addresses(header("Reply-To")),
        to: addresses(header("To")),
        cc: addresses(header("Cc")),
        bcc: addresses(header("Bcc")),
        message_id: header("Message-ID").map(RfcMessageId::new),
        in_reply_to: header("In-Reply-To").map(RfcMessageId::new),
        references: header("References")
            .map(|value| {
                value
                    .split_whitespace()
                    .map(RfcMessageId::new)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        list_id: header("List-Id"),
    }
}

/// A light parse: anything but `Name <addr>` is kept as a bare address.
fn addresses(header: Option<String>) -> Vec<EmailAddress> {
    let Some(header) = header else {
        return Vec::new();
    };
    header
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            if let Some((name, rest)) = part.split_once('<') {
                let address = rest.trim_end_matches('>').trim();
                let name = name.trim().trim_matches('"');
                Some(EmailAddress::new(
                    (!name.is_empty()).then(|| name.to_owned()),
                    address,
                ))
            } else {
                Some(EmailAddress::new(None::<String>, part))
            }
        })
        .collect()
}

/// `\Seen` is the absence of `UNREAD`.
pub(crate) fn flags(label_ids: &[String]) -> FlagSet {
    let mut flags = FlagSet::new();
    if !label_ids.iter().any(|id| id == "UNREAD") {
        flags.insert(Flag::Seen);
    }
    for id in label_ids {
        match id.as_str() {
            "STARRED" => {
                flags.insert(Flag::Flagged);
            }
            "DRAFT" => {
                flags.insert(Flag::Draft);
            }
            "SPAM" => {
                flags.insert(Flag::Junk);
            }
            _ => {}
        }
    }
    flags
}

/// Only `\Seen`/`UNREAD` (inverted) and `\Flagged`/`STARRED` travel; `\Deleted`
/// never reaches here, and other flags are dropped rather than guessed.
pub(crate) fn label_changes(change: &FlagChange) -> (Vec<String>, Vec<String>) {
    let mut adds = Vec::new();
    let mut removes = Vec::new();
    let mut apply = |flag: &Flag, setting: bool| match flag {
        Flag::Seen => {
            if setting {
                removes.push("UNREAD".to_owned());
            } else {
                adds.push("UNREAD".to_owned());
            }
        }
        Flag::Flagged => {
            if setting {
                adds.push("STARRED".to_owned());
            } else {
                removes.push("STARRED".to_owned());
            }
        }
        _ => {}
    };
    match change {
        FlagChange::Add(flags) => {
            for flag in flags.iter() {
                apply(flag, true);
            }
        }
        FlagChange::Remove(flags) => {
            for flag in flags.iter() {
                apply(flag, false);
            }
        }
        FlagChange::Replace(flags) => {
            for flag in [Flag::Seen, Flag::Flagged] {
                apply(&flag, flags.contains(&flag));
            }
        }
    }
    (adds, removes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_message_identity_is_account_wide_content_identity() {
        let wire = GmailMessage {
            id: "message-1".into(),
            ..Default::default()
        };
        let message = fetched(&wire, 1).unwrap().into_message(
            postio_model::AccountId::new(1),
            postio_model::MailboxId::new(2),
        );
        assert_eq!(
            message.server.content_identity,
            Some(postio_model::ContentIdentity::new(
                "gmail-message",
                "message-1"
            ))
        );
    }

    #[test]
    fn seen_is_the_absence_of_unread() {
        let unread = flags(&["INBOX".to_owned(), "UNREAD".to_owned()]);
        assert!(!unread.is_seen());
        let read = flags(&["INBOX".to_owned()]);
        assert!(read.is_seen());

        let (adds, removes) = label_changes(&FlagChange::Add(FlagSet::from_iter([Flag::Seen])));
        assert_eq!(adds, Vec::<String>::new());
        assert_eq!(removes, vec!["UNREAD".to_owned()]);

        let (adds, removes) = label_changes(&FlagChange::Remove(FlagSet::from_iter([Flag::Seen])));
        assert_eq!(adds, vec!["UNREAD".to_owned()]);
        assert_eq!(removes, Vec::<String>::new());
    }

    #[test]
    fn a_replace_settles_both_managed_pairs() {
        let (adds, removes) =
            label_changes(&FlagChange::Replace(FlagSet::from_iter([Flag::Flagged])));
        assert!(adds.contains(&"UNREAD".to_owned()), "not seen -> UNREAD on");
        assert!(adds.contains(&"STARRED".to_owned()));
        assert!(removes.is_empty());
    }
}
