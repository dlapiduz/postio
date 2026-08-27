//! Gmail objects into the seam's types, and back.

use chrono::{DateTime, Utc};
use io_gmail::v1::rest::labels::GmailLabel;
use io_gmail::v1::rest::labels::GmailLabelType;
use io_gmail::v1::rest::messages::{GmailMessage, GmailMessageHeader};
use postio_imap::backend::{Envelope, FetchedMessage, FlagChange, MailboxSummary};
use postio_model::{EmailAddress, Flag, FlagSet, RemoteId, RfcMessageId, Uid, UidValidity};

/// The synthetic generation every Gmail message reports: ids never
/// renumber, so there is exactly one, for ever.
pub(crate) const GENERATION: u32 = 0;

/// The path this adapter gives the archive — the messages no system
/// label claims (ADR 0018 Q4). It is not a Gmail label: moving here
/// *removes* labels, and listing it is a search.
pub(crate) const ARCHIVE_PATH: &str = "Archive";

/// The search that *is* the archive.
pub(crate) const ARCHIVE_QUERY: &str = "-in:inbox -in:sent -in:draft -in:trash -in:spam";

/// The system labels this adapter surfaces as role mailboxes.
const SYSTEM: &[(&str, &str, &str)] = &[
    ("INBOX", "Inbox", ""),
    ("SENT", "Sent", "\\Sent"),
    ("DRAFT", "Drafts", "\\Drafts"),
    ("TRASH", "Trash", "\\Trash"),
    ("SPAM", "Junk", "\\Junk"),
];

/// The mailboxes a label listing describes: the five system roles, the
/// synthetic archive, and every user label as a plain folder.
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

/// The label id a mailbox path names, or `None` for the archive (which
/// is the *absence* of labels) — an unknown path is the caller's error.
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

/// A fetched message bound to its synthetic enumeration position.
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
        // Whole-message fetches only in this slice; see the crate docs.
        structure: None,
    })
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

/// A light parse of an address header: display names survive the common
/// `Name <addr>` shape, and anything else is kept as a bare address. The
/// full RFC 5322 parse happens when the body is fetched and stored.
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

/// Labels into the seam's flags. `\Seen` is the absence of `UNREAD`.
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

/// A flag change as the label adds and removes `messages.modify` wants.
///
/// Only the managed pairs travel — `\Seen`/`UNREAD` (inverted) and
/// `\Flagged`/`STARRED`. `\Deleted` never reaches here: the backend maps
/// it to the trash before building a modify. Anything else Gmail either
/// manages itself (`DRAFT`) or has no label for, and is dropped rather
/// than guessed.
pub(crate) fn label_changes(change: &FlagChange) -> (Vec<String>, Vec<String>) {
    let mut adds = Vec::new();
    let mut removes = Vec::new();
    let mut apply = |flag: &Flag, setting: bool| match flag {
        Flag::Seen => {
            // Inverted: seen means *not* UNREAD.
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
