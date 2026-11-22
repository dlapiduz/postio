use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use io_jmap::rfc8621::email::JmapEmail;
use io_jmap::rfc8621::mailbox::{JmapMailbox, JmapMailboxRole};
use postio_account::backend::{Envelope, FetchedMessage, MailboxSummary};
use postio_model::{EmailAddress, Flag, FlagSet, RemoteId, RfcMessageId, Uid, UidValidity};

/// JMAP ids never renumber, so there is exactly one generation; `MailboxStatus` reports the same.
pub(crate) const GENERATION: u32 = 0;

pub(crate) fn summary(
    mailbox: &JmapMailbox,
    by_id: &BTreeMap<String, JmapMailbox>,
) -> MailboxSummary {
    let mut names = vec![mailbox.name.clone().unwrap_or_default()];
    let mut parent = mailbox.parent_id.clone();
    // Bounded: a cycle in parent ids is server nonsense, not a hang here.
    for _ in 0..16 {
        let Some(id) = parent else { break };
        let Some(above) = by_id.get(&id) else { break };
        names.push(above.name.clone().unwrap_or_default());
        parent = above.parent_id.clone();
    }
    names.reverse();
    let path = names.join("/");

    let attributes: Vec<String> = match &mailbox.role {
        Some(JmapMailboxRole::Archive) => vec!["\\Archive".to_owned()],
        Some(JmapMailboxRole::Drafts) => vec!["\\Drafts".to_owned()],
        Some(JmapMailboxRole::Flagged) => vec!["\\Flagged".to_owned()],
        Some(JmapMailboxRole::Junk) => vec!["\\Junk".to_owned()],
        Some(JmapMailboxRole::Sent) => vec!["\\Sent".to_owned()],
        Some(JmapMailboxRole::Trash) => vec!["\\Trash".to_owned()],
        // Inbox has no special-use attribute; the resolver reads it off the
        // name, which RFC 8621 fixes as the role's own spelling.
        _ => Vec::new(),
    };
    MailboxSummary::new(path, Some('/'), attributes)
}

/// `uid` is only the position in the `receivedAt`-ascending enumeration; rows
/// match by identity (#544), so a shifting position re-fetches at worst.
pub(crate) fn fetched(email: &JmapEmail, position: u32) -> Option<FetchedMessage> {
    let id = email.id.clone().filter(|id| !id.is_empty())?;
    Some(FetchedMessage {
        content_identity: Some(postio_model::ContentIdentity::new("jmap-email", id.clone())),
        remote_id: RemoteId::new(id),
        uid: Uid::new(position),
        uid_validity: UidValidity::new(GENERATION),
        mod_seq: None,
        flags: flags(email.keywords.as_ref()),
        internal_date: rfc3339(email.received_at.as_deref()).unwrap_or_else(Utc::now),
        size: email.size.unwrap_or_default(),
        envelope: Some(envelope(email)),
        structure: None,
        promoted: None,
    })
}

fn envelope(email: &JmapEmail) -> Envelope {
    Envelope {
        date: rfc3339(email.sent_at.as_deref()),
        subject: email.subject.clone(),
        from: addresses(email.from.as_deref()),
        sender: addresses(email.sender.as_deref()).into_iter().next(),
        reply_to: addresses(email.reply_to.as_deref()),
        to: addresses(email.to.as_deref()),
        cc: addresses(email.cc.as_deref()),
        bcc: addresses(email.bcc.as_deref()),
        message_id: first_id(email.message_id.as_deref()),
        in_reply_to: first_id(email.in_reply_to.as_deref()),
        references: email
            .references
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(RfcMessageId::new)
            .collect(),
        list_id: None,
    }
}

fn addresses(list: Option<&[io_jmap::rfc8621::email::JmapEmailAddress]>) -> Vec<EmailAddress> {
    list.unwrap_or_default()
        .iter()
        .map(|address| EmailAddress::new(address.name.clone(), address.email.clone()))
        .collect()
}

fn first_id(list: Option<&[String]>) -> Option<RfcMessageId> {
    list.and_then(|ids| ids.first()).map(RfcMessageId::new)
}

fn rfc3339(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|date| date.with_timezone(&Utc))
}

pub(crate) fn flags(keywords: Option<&BTreeMap<String, bool>>) -> FlagSet {
    let mut flags = FlagSet::new();
    for (keyword, set) in keywords.into_iter().flatten() {
        if !set {
            continue;
        }
        flags.insert(flag(keyword));
    }
    flags
}

fn flag(keyword: &str) -> Flag {
    match keyword {
        "$seen" => Flag::Seen,
        "$answered" => Flag::Answered,
        "$flagged" => Flag::Flagged,
        "$draft" => Flag::Draft,
        "$forwarded" => Flag::Forwarded,
        "$junk" => Flag::Junk,
        "$notjunk" => Flag::NotJunk,
        other => Flag::Keyword(other.to_owned()),
    }
}

/// `\Deleted` maps to `$deleted`, only a marker between mark and expunge.
/// `\Recent` has no JMAP meaning and is dropped.
pub(crate) fn keyword(flag: &Flag) -> Option<String> {
    Some(match flag {
        Flag::Seen => "$seen".to_owned(),
        Flag::Answered => "$answered".to_owned(),
        Flag::Flagged => "$flagged".to_owned(),
        Flag::Draft => "$draft".to_owned(),
        Flag::Forwarded => "$forwarded".to_owned(),
        Flag::Junk => "$junk".to_owned(),
        Flag::NotJunk => "$notjunk".to_owned(),
        Flag::Deleted => "$deleted".to_owned(),
        Flag::Recent => return None,
        Flag::Keyword(keyword) => keyword.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_email_identity_is_account_wide_content_identity() {
        let email = JmapEmail {
            id: Some("email-1".into()),
            ..Default::default()
        };
        let message = fetched(&email, 1).unwrap().into_message(
            postio_model::AccountId::new(1),
            postio_model::MailboxId::new(2),
        );
        assert_eq!(
            message.server.content_identity,
            Some(postio_model::ContentIdentity::new("jmap-email", "email-1"))
        );
    }

    #[test]
    fn keywords_round_trip_through_the_seams_flags() {
        let mut keywords = BTreeMap::new();
        for k in ["$seen", "$flagged", "customtag"] {
            keywords.insert(k.to_owned(), true);
        }
        keywords.insert("$draft".to_owned(), false);

        let set = flags(Some(&keywords));
        assert!(set.is_seen() && set.is_flagged());
        assert!(!set.is_draft(), "a false-valued keyword is not set");

        let back: Vec<String> = set.iter().filter_map(keyword).collect();
        assert!(back.contains(&"$seen".to_owned()));
        assert!(back.contains(&"customtag".to_owned()));
    }

    #[test]
    fn a_child_mailbox_gets_its_parents_in_the_path() {
        let mut by_id = BTreeMap::new();
        let parent = JmapMailbox {
            id: Some("p".to_owned()),
            name: Some("Projects".to_owned()),
            ..Default::default()
        };
        by_id.insert("p".to_owned(), parent);
        let child = JmapMailbox {
            id: Some("c".to_owned()),
            name: Some("Postio".to_owned()),
            parent_id: Some("p".to_owned()),
            ..Default::default()
        };

        assert_eq!(summary(&child, &by_id).path, "Projects/Postio");
    }
}
