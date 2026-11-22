//! Recipient completion: when it is offered, which suggestions, their order, and what accepting one
//! does to the field.

use std::cmp::Ordering;

use postio_model::address::current_entry;
use postio_model::contact_group::RecipientCandidate;
use postio_model::{Contact, ContactSource, EmailAddress};

/// One character matches most of an address book; four is where a prefix identifies somebody
/// (#424).
pub const MIN_COMPLETION_PREFIX: usize = 4;

/// A group shows its name and size, since accepting it inserts several addresses.
pub fn candidate_label(candidate: &RecipientCandidate) -> String {
    match candidate {
        RecipientCandidate::Contact(address) => address.to_string(),
        RecipientCandidate::Group { name, members } => {
            format!("{name} ({} people)", members.len())
        }
    }
}

/// A group inserts every member as its own address, as if typed; there is no group reference.
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

pub fn prefix(text: &str) -> Option<&str> {
    let (_, token) = current_entry(text);
    (token.chars().count() >= MIN_COMPLETION_PREFIX).then_some(token)
}

/// A person and how many times the user has written to any of their addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correspondent {
    pub contact: Contact,
    /// Summed from `correspondents.sent_count`; zero until read.
    pub sent_count: u32,
}

impl From<Contact> for Correspondent {
    fn from(contact: Contact) -> Self {
        Self {
            contact,
            sent_count: 0,
        }
    }
}

/// At most `limit` candidates for `prefix`: groups whose name begins with it first, then people
/// by letters written, address-book band, recency and frequency.
/// A person is offered once per address under their one name; a group with no members is never
/// offered.
pub fn suggest(
    groups: &[(String, Vec<EmailAddress>)],
    correspondents: &[Correspondent],
    prefix: &str,
    limit: usize,
) -> Vec<RecipientCandidate> {
    suggest_ranked(groups, correspondents, prefix, limit, false)
}

/// As `suggest`, and with `by_kind` a robot, list or newsletter ranks below every person unless the
/// user has written to it.
pub fn suggest_ranked(
    groups: &[(String, Vec<EmailAddress>)],
    correspondents: &[Correspondent],
    prefix: &str,
    limit: usize,
    by_kind: bool,
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
        .filter(|row| completes(&row.contact, &prefix))
        .collect();
    // Stable: rows the rule cannot tell apart keep the directory's order.
    matching.sort_by(|a, b| rank(a, b, by_kind));
    candidates.extend(
        matching
            .into_iter()
            .flat_map(|row| offered_addresses(&row.contact))
            .map(RecipientCandidate::Contact),
    );
    candidates.truncate(limit);
    candidates
}

/// Order: more letters written, then a contact the user made or imported over one only seen in
/// mail,
/// then the more lately seen (never seen last), then the more often seen.
/// Until letters are counted every count is zero and this is the store's order.
fn rank(a: &Correspondent, b: &Correspondent, by_kind: bool) -> Ordering {
    let band = |row: &Correspondent| row.contact.source == ContactSource::Mail;
    let demoted = |row: &Correspondent| {
        by_kind
            && row.sent_count == 0
            && !matches!(
                row.contact.kind,
                postio_model::ContactKind::Person | postio_model::ContactKind::Company
            )
    };
    demoted(a)
        .cmp(&demoted(b))
        .then_with(|| b.sent_count.cmp(&a.sent_count))
        .then_with(|| band(a).cmp(&band(b)))
        .then_with(|| b.contact.last_seen_at.cmp(&a.contact.last_seen_at))
        .then_with(|| b.contact.times_seen.cmp(&a.contact.times_seen))
}

fn completes(person: &Contact, prefix: &str) -> bool {
    prefix.is_empty()
        || person
            .addresses
            .iter()
            .any(|owned| owned.address.address.to_lowercase().starts_with(prefix))
        || [person.name.as_deref(), person.seen_name.as_deref()]
            .into_iter()
            .flatten()
            .any(|name| begins_a_word(&name.to_lowercase(), prefix))
}

fn begins_a_word(text: &str, prefix: &str) -> bool {
    text.starts_with(prefix)
        || text
            .match_indices(' ')
            .any(|(at, _)| text[at + 1..].starts_with(prefix))
}

fn offered_addresses(person: &Contact) -> Vec<EmailAddress> {
    let name = person_name(person);
    person
        .addresses
        .iter()
        .map(|owned| EmailAddress::new(name.clone(), owned.address.address.clone()))
        .collect()
}

pub fn preferred_address(person: &Contact) -> Option<EmailAddress> {
    person
        .preferred_address()
        .map(|owned| EmailAddress::new(person_name(person), owned.address.address.clone()))
}

fn person_name(person: &Contact) -> Option<String> {
    [person.name.as_deref(), person.seen_name.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|name| !name.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use postio_model::{AddressId, ContactAddress};

    fn owned(id: i64, name: Option<&str>, address: &str) -> ContactAddress {
        ContactAddress::new(AddressId::new(id), EmailAddress::new(name, address))
    }

    fn person(name: &str, addresses: &[&str]) -> Contact {
        let base = addresses[0].len() as i64 * 100;
        let mut person = Contact::new(owned(base, Some(name), addresses[0]));
        for (i, address) in addresses.iter().enumerate().skip(1) {
            person.addresses.push(owned(base + i as i64, None, address));
        }
        person
    }

    fn correspondent(
        name: &str,
        address: &str,
        seen: u32,
        days_ago: i64,
        sent: u32,
    ) -> Correspondent {
        let mut contact = person(name, &[address]);
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

    #[test]
    fn groups_come_first() {
        let rows = [
            correspondent("Grace Abara", "grace@example.net", 5, 3, 1),
            correspondent("Lin Graham", "lin@example.com", 1, 9, 0),
            correspondent("Ada Lovelace", "ada@example.com", 90, 0, 90),
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
    fn a_person_is_offered_once_per_address_under_one_name() {
        let mut ada = person("Ada Lovelace", &["ada@work.example", "ada@home.example"]);
        ada.name = Some("Ada Lovelace".to_owned());
        ada.seen_name = Some("A. King".to_owned());
        let rows = [Correspondent::from(ada)];
        let offered = suggest(&[], &rows, "ada@home", 8);
        assert_eq!(
            addresses(&offered),
            ["ada@work.example", "ada@home.example"],
            "a match on any address offers the person, preferred address first"
        );
        assert!(offered.iter().all(|candidate| matches!(
            candidate,
            RecipientCandidate::Contact(address) if address.name.as_deref() == Some("Ada Lovelace")
        )));
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

    #[test]
    fn ranked_by_kind_a_robot_or_a_list_never_outranks_a_person_unless_you_write_to_it() {
        let mut noisy = correspondent("Alpha Alerts", "alerts@alpha.example", 900, 0, 0);
        noisy.contact.kind = postio_model::ContactKind::Automated;
        let mut list = correspondent("Alpha List", "list@alpha.example", 800, 0, 0);
        list.contact.kind = postio_model::ContactKind::List;
        let mut posted_to = correspondent("Alpha Dev List", "dev@alpha.example", 5, 90, 3);
        posted_to.contact.kind = postio_model::ContactKind::List;
        let mut quiet = correspondent("Alpha Person", "person@alpha.example", 2, 300, 0);
        quiet.contact.kind = postio_model::ContactKind::Person;
        let directory = [noisy, list, posted_to, quiet];
        let order = |by_kind: bool| -> Vec<String> {
            suggest_ranked(&[], &directory, "alpha", 10, by_kind)
                .into_iter()
                .filter_map(|c| match c {
                    RecipientCandidate::Contact(c) => Some(c.address),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            order(false),
            [
                "dev@alpha.example",
                "alerts@alpha.example",
                "list@alpha.example",
                "person@alpha.example"
            ],
            "unranked: written-to first, then recency"
        );
        assert_eq!(
            order(true),
            [
                "dev@alpha.example",
                "person@alpha.example",
                "alerts@alpha.example",
                "list@alpha.example"
            ],
            "ranked: the person before the robot and the list nobody writes to"
        );
        assert_eq!(
            suggest(&[], &directory, "alpha", 10),
            suggest_ranked(&[], &directory, "alpha", 10, false)
        );
    }
}
