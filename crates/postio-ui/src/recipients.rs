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
/// there is no group reference to insert instead (specs/005-contacts).
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
/// person, and how many times the user has written to any of their
/// addresses -- the "wrote 42 times" a suggestion shows (FR-052).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correspondent {
    /// The person, as the store keeps them.
    pub contact: Contact,
    /// Messages the user sent to any of this person's addresses, summed
    /// from `correspondents.sent_count` (T075); zero until that table is
    /// read.
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
/// * **Then people** one of whose addresses begins with the prefix, or any
///   word of the name the user gave or the mail last gave them. A person
///   is offered once per address they own, preferred address first, each
///   under the person's one name (specs/005-contacts FR-030). The directory
///   holds live people only: a deleted one is never offered. They are
///   ranked by the letters written to each, then the address-book band,
///   then how lately and how often each was seen.
pub fn suggest(
    groups: &[(String, Vec<EmailAddress>)],
    correspondents: &[Correspondent],
    prefix: &str,
    limit: usize,
) -> Vec<RecipientCandidate> {
    suggest_ranked(groups, correspondents, prefix, limit, false)
}

/// [`suggest`], and with `by_kind` a robot, a list or a newsletter ranks
/// below every person and company unless the user has written to it: what a
/// platform that has adopted people asks for (specs/011-focus-contacts S4).
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

/// Which of two people completion offers first (research R15, over the
/// address-book band of specs/005-contacts):
///
/// 1. the more letters the user wrote to it: a letter is the user's own
///    act, the one evidence that they want to write there again;
/// 2. then a contact the user made or imported before one only seen in
///    mail, because how often a list robot is seen says nothing about
///    wanting to write to it (specs/005-contacts);
/// 3. then the more lately seen, and one never seen last;
/// 4. then the more often seen.
///
/// Until letters are counted (T075) every count is zero, and this is the
/// store's order, `ContactRepository::search`'s.
fn rank(a: &Correspondent, b: &Correspondent, by_kind: bool) -> Ordering {
    let band = |row: &Correspondent| row.contact.source == ContactSource::Mail;
    // Not someone you write to: nobody you have written to is demoted.
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

/// Whether `person` completes `prefix`, which is already lowercase.
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

/// Whether `prefix` begins `text` or any space-separated word in it.
fn begins_a_word(text: &str, prefix: &str) -> bool {
    text.starts_with(prefix)
        || text
            .match_indices(' ')
            .any(|(at, _)| text[at + 1..].starts_with(prefix))
}

/// Every address a person offers, preferred first (the store's order), each
/// under the person's one name.
fn offered_addresses(person: &Contact) -> Vec<EmailAddress> {
    let name = person_name(person);
    person
        .addresses
        .iter()
        .map(|owned| EmailAddress::new(name.clone(), owned.address.address.clone()))
        .collect()
}

/// The address a group member expands to: their preferred one
/// (specs/005-contacts FR-041).
pub fn preferred_address(person: &Contact) -> Option<EmailAddress> {
    person
        .preferred_address()
        .map(|owned| EmailAddress::new(person_name(person), owned.address.address.clone()))
}

/// A person's name for a recipient header -- the one the user set, else the
/// one the mail gave them -- when they have one beyond their address.
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

    /// A person called `name` owning `addresses`, preferred first.
    fn person(name: &str, addresses: &[&str]) -> Contact {
        let base = addresses[0].len() as i64 * 100;
        let mut person = Contact::new(owned(base, Some(name), addresses[0]));
        for (i, address) in addresses.iter().enumerate().skip(1) {
            person.addresses.push(owned(base + i as i64, None, address));
        }
        person
    }

    /// A correspondent seen on `seen` messages, last `days_ago`, and
    /// written to `sent` times.
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

    /// The address-book band still holds for what the user did not write: a contact
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

    /// What matched before still matches, and nothing else: a prefix of
    /// an address or of any word in a name, and groups, which the user made
    /// on purpose, first. (A deleted person is not in the directory at all:
    /// the store hands completion live people only.)
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

    /// Specs/005-contacts FR-030: a person is offered once per address they
    /// own, preferred first, every one under the person's one name.
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
