//! Recipient completion: when it is offered, which suggestions it offers
//! and in what order, how a suggestion reads, and what accepting one does
//! to the field.
//!
//! Shared so the desktop's popover and the terminal's list behave alike.

use std::cmp::Ordering;

use postio_model::address::current_entry;
use postio_model::contact_group::RecipientCandidate;
use postio_model::{Contact, ContactSource, EmailAddress};

/// How much of the recipient being typed must exist before completion offers
/// anything.
///
/// Four, from #424. One character matches most of an address book, so the
/// popover opened over the field with a list nobody could choose from yet —
/// and it did it while a query ran on every keystroke. Four is where a prefix
/// starts to identify somebody.
pub const MIN_COMPLETION_PREFIX: usize = 4;

/// The completion row's label: an address for a contact, or the name and
/// size for a group -- distinguishable from a contact at a glance, since
/// accepting one inserts several addresses rather than one.
pub fn candidate_label(candidate: &RecipientCandidate) -> String {
    match candidate {
        RecipientCandidate::Contact(address) => address.to_string(),
        RecipientCandidate::Group { name, members } => {
            format!("{name} ({} people)", members.len())
        }
    }
}

/// `text` with the address being typed replaced by `candidate`, every
/// address typed before it untouched, and a `, ` left for the next one.
///
/// A contact inserts one address; a group inserts every member as its own
/// address, comma by comma, exactly as if they had been typed individually --
/// there is no group reference to insert instead (ADR 0007 Q3).
pub fn accepted(text: &str, candidate: &RecipientCandidate) -> String {
    let inserted: String = match candidate {
        RecipientCandidate::Contact(address) => format!("{address}, "),
        RecipientCandidate::Group { members, .. } => members
            .iter()
            .map(|address| format!("{address}, "))
            .collect(),
    };
    let (start, _) = current_entry(text);
    let mut replaced = text.to_owned();
    replaced.replace_range(start.., &inserted);
    replaced
}

/// What is being typed in `text`, when there is enough of it to look up.
pub fn prefix(text: &str) -> Option<&str> {
    let (_, token) = current_entry(text);
    (token.chars().count() >= MIN_COMPLETION_PREFIX).then_some(token)
}

/// One row of the recipient directory (spec 007 T076, research R15): a
/// contact, and how many times the user has written to its address -- the
/// "wrote 42 times" a suggestion shows (FR-052).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correspondent {
    /// The contact, as the store keeps it.
    pub contact: Contact,
    /// Messages the user sent to this address. `correspondents.sent_count`
    /// (T075); zero until that table is read.
    pub sent_count: u32,
}

impl From<Contact> for Correspondent {
    /// A contact nobody has counted letters to yet.
    fn from(contact: Contact) -> Self {
        Self {
            contact,
            sent_count: 0,
        }
    }
}

/// At most `limit` candidates for `prefix`, from a directory's `groups`
/// and `correspondents`: the one completion rule every app shares (research
/// R15), so an address ranks the same wherever it is typed.
///
/// * **Groups first**, whose name begins with the prefix, in the order
///   given: a group is a deliberate choice the user is likelier typing
///   towards. A group with no members expands to nothing, so it is never
///   offered.
/// * **Then contacts** whose address begins with the prefix, or any word of
///   the name the user gave or last saw on it. A suppressed contact never
///   (ADR 0007 Q2). They are ranked by [`rank`].
pub fn suggest(
    groups: &[(String, Vec<EmailAddress>)],
    correspondents: &[Correspondent],
    prefix: &str,
    limit: usize,
) -> Vec<RecipientCandidate> {
    let prefix = prefix.trim().to_lowercase();
    let mut candidates: Vec<RecipientCandidate> = groups
        .iter()
        .filter(|(name, members)| !members.is_empty() && name.to_lowercase().starts_with(&prefix))
        .map(|(name, members)| RecipientCandidate::Group {
            name: name.clone(),
            members: members.clone(),
        })
        .take(limit)
        .collect();
    let mut matching: Vec<&Correspondent> = correspondents
        .iter()
        .filter(|row| !row.contact.suppressed && completes(&row.contact, &prefix))
        .collect();
    // Stable: rows the rule cannot tell apart keep the directory's order.
    matching.sort_by(|a, b| rank(a, b));
    let room = limit.saturating_sub(candidates.len());
    candidates.extend(
        matching
            .into_iter()
            .take(room)
            .map(|row| RecipientCandidate::Contact(completed_address(&row.contact))),
    );
    candidates
}

/// Which of two contacts completion offers first (research R15, over ADR
/// 0007 Q6's bands):
///
/// 1. the more letters the user wrote to it: a letter is the user's own
///    act, the one evidence that they want to write there again;
/// 2. then a contact the user made or imported before one only seen in
///    mail, because how often a list robot is seen says nothing about
///    wanting to write to it (ADR 0007 Q6);
/// 3. then the more lately seen, and one never seen last;
/// 4. then the more often seen.
///
/// Until letters are counted (T075) every count is zero, and this is the
/// store's order, `ContactRepository::search`'s.
fn rank(a: &Correspondent, b: &Correspondent) -> Ordering {
    let band = |row: &Correspondent| row.contact.source == ContactSource::Mail;
    b.sent_count
        .cmp(&a.sent_count)
        .then_with(|| band(a).cmp(&band(b)))
        .then_with(|| b.contact.last_seen_at.cmp(&a.contact.last_seen_at))
        .then_with(|| b.contact.times_seen.cmp(&a.contact.times_seen))
}

/// Whether `contact` completes `prefix`, which is already lowercase.
fn completes(contact: &Contact, prefix: &str) -> bool {
    if prefix.is_empty() || contact.address.address.to_lowercase().starts_with(prefix) {
        return true;
    }
    [contact.name.as_deref(), contact.address.name.as_deref()]
        .into_iter()
        .flatten()
        .any(|name| begins_a_word(&name.to_lowercase(), prefix))
}

/// Whether `prefix` begins `text` or any space-separated word in it.
fn begins_a_word(text: &str, prefix: &str) -> bool {
    text.starts_with(prefix)
        || text
            .match_indices(' ')
            .any(|(at, _)| text[at + 1..].starts_with(prefix))
}

/// The address a contact completes to: the name the user gave it, or the
/// one last seen on the address.
fn completed_address(contact: &Contact) -> EmailAddress {
    let name = contact
        .name
        .clone()
        .or_else(|| contact.address.name.clone());
    EmailAddress::new(name, contact.address.address.clone())
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    /// A correspondent seen on `seen` messages, last `days_ago`, and
    /// written to `sent` times.
    fn correspondent(
        name: &str,
        address: &str,
        seen: u32,
        days_ago: i64,
        sent: u32,
    ) -> Correspondent {
        let mut contact = Contact::new(EmailAddress::new(Some(name), address));
        contact.times_seen = seen;
        contact.last_seen_at = Some(
            Utc.with_ymd_and_hms(2026, 9, 26, 16, 0, 0).unwrap() - chrono::Duration::days(days_ago),
        );
        Correspondent {
            contact,
            sent_count: sent,
        }
    }

    fn addresses(candidates: &[RecipientCandidate]) -> Vec<String> {
        candidates
            .iter()
            .map(|candidate| match candidate {
                RecipientCandidate::Contact(address) => address.address.clone(),
                RecipientCandidate::Group { name, .. } => format!("group:{name}"),
            })
            .collect()
    }

    /// Spec 007 US3 scenario 6: completion ranks by how often the user
    /// wrote to an address. Seen on a hundred messages, and more lately, but
    /// never written to, is somebody who writes to the user; written to 42
    /// times is somebody the user writes to.
    #[test]
    fn an_address_written_to_ranks_above_one_only_seen() {
        let rows = [
            correspondent("Grant Holloway", "grant@example.com", 100, 0, 0),
            correspondent("Grace Abara", "grace@example.net", 50, 30, 42),
        ];
        assert_eq!(
            addresses(&suggest(&[], &rows, "gra", 8)),
            ["grace@example.net", "grant@example.com"]
        );
    }

    /// Research R15's rule, whole: sent count, then last seen, then times
    /// seen.
    #[test]
    fn equals_are_ranked_by_the_later_seen_then_the_more_seen() {
        let rows = [
            correspondent("Ada One", "ada.one@example.com", 5, 10, 3),
            correspondent("Ada Two", "ada.two@example.com", 5, 2, 3),
            correspondent("Ada Three", "ada.three@example.com", 9, 10, 3),
            correspondent("Ada Four", "ada.four@example.com", 99, 0, 7),
        ];
        assert_eq!(
            addresses(&suggest(&[], &rows, "ada", 8)),
            [
                "ada.four@example.com",
                "ada.two@example.com",
                "ada.three@example.com",
                "ada.one@example.com",
            ]
        );
    }

    /// ADR 0007 Q6 still holds for what the user did not write: a contact
    /// they made outranks every mail sighting, however frequent or recent,
    /// because four hundred sightings of a list robot are not evidence they
    /// want to write to it. A letter they wrote is that evidence, so it
    /// outranks the band.
    #[test]
    fn a_letter_written_outranks_the_band_and_the_band_outranks_sightings() {
        let mut made = correspondent("Grace Made", "grace@example.org", 0, 0, 0);
        made.contact.source = postio_model::ContactSource::User;
        made.contact.last_seen_at = None;
        let rows = [
            correspondent("Grant Robot", "grant@example.com", 400, 0, 0),
            made,
            correspondent("Graham Friend", "graham@example.net", 3, 60, 2),
        ];
        assert_eq!(
            addresses(&suggest(&[], &rows, "gra", 8)),
            [
                "graham@example.net",
                "grace@example.org",
                "grant@example.com"
            ]
        );
    }

    /// What matched before still matches, and nothing else: a prefix of the
    /// address or of any word in a name, never a suppressed contact, and
    /// groups, which the user made on purpose, first.
    #[test]
    fn groups_come_first_and_a_suppressed_contact_never() {
        let mut gone = correspondent("Grace Gone", "grace.gone@example.org", 40, 0, 40);
        gone.contact.suppressed = true;
        let rows = [
            correspondent("Grace Abara", "grace@example.net", 5, 3, 1),
            correspondent("Lin Graham", "lin@example.com", 1, 9, 0),
            correspondent("Ada Lovelace", "ada@example.com", 90, 0, 90),
            gone,
        ];
        let groups = [
            (
                "Gravel run".to_owned(),
                vec![EmailAddress::new(None::<String>, "ada@example.com")],
            ),
            ("Grand empty".to_owned(), Vec::new()),
        ];
        assert_eq!(
            addresses(&suggest(&groups, &rows, "gra", 8)),
            ["group:Gravel run", "grace@example.net", "lin@example.com"]
        );
        assert_eq!(addresses(&suggest(&groups, &rows, "gra", 2)).len(), 2);
    }

    #[test]
    fn accepting_keeps_what_came_before() {
        let ada = RecipientCandidate::Contact(EmailAddress::new(Some("Ada"), "ada@example.com"));
        assert_eq!(
            accepted("grace@example.net, ada@", &ada),
            "grace@example.net, Ada <ada@example.com>, "
        );
    }

    #[test]
    fn a_group_is_its_members() {
        let group = RecipientCandidate::Group {
            name: "Tide".into(),
            members: vec![
                EmailAddress::new(None::<String>, "ada@example.com"),
                EmailAddress::new(None::<String>, "grace@example.net"),
            ],
        };
        assert_eq!(
            accepted("tide", &group),
            "ada@example.com, grace@example.net, "
        );
    }

    #[test]
    fn three_letters_are_not_enough_to_look_up() {
        assert_eq!(prefix("x@example.com, ada"), None);
        assert_eq!(prefix("x@example.com, ada@"), Some("ada@"));
    }
}
