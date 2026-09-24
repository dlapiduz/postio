//! Recipient completion, answered from memory.
//!
//! The composer asks for candidates on every keystroke in `To`, `Cc` and
//! `Bcc`, synchronously, on the GTK thread. That used to be a fresh store
//! connection, a groups read, one members read per matching group, and a
//! contacts query -- per key, on the thread that draws. The directory is
//! small and changes slowly, so it is read off the thread whenever the
//! composer opens and each key is answered by [`Directory::suggest`], which
//! is arithmetic.
//!
//! What is offered is a *person* (specs/005-contacts): once per address they
//! own, preferred address first, every one under the person's one name
//! (FR-030). A person completes on a prefix of any of their addresses, or of
//! their name or any word in it; the people the user made come before the
//! ones only the mail knows, then the order the store ranked them in -- most
//! recently seen first.

use postio_gtk::composer::RecipientCandidate;
use postio_model::{Contact, EmailAddress};

/// Every person and group a composer can complete, as last read.
#[derive(Debug, Default, Clone)]
pub struct Directory {
    /// Named groups with their members' addresses, in the store's order.
    groups: Vec<(String, Vec<EmailAddress>)>,
    /// Live people, in the order the store ranks them.
    people: Vec<Contact>,
}

impl Directory {
    /// A directory over `groups` and `people`, which arrive in the order
    /// they are to be offered.
    pub fn new(groups: Vec<(String, Vec<EmailAddress>)>, people: Vec<Contact>) -> Self {
        Self { groups, people }
    }

    /// At most `limit` candidates for `prefix`: groups whose name begins
    /// with it first -- a group is a deliberate choice the user is likelier
    /// typing towards -- then people, one candidate per address.
    pub fn suggest(&self, prefix: &str, limit: usize) -> Vec<RecipientCandidate> {
        let prefix = prefix.trim().to_lowercase();
        let mut candidates: Vec<RecipientCandidate> = self
            .groups
            .iter()
            // A group with no members expands to nothing, so offering it
            // would be a suggestion that does nothing when accepted.
            .filter(|(name, members)| {
                !members.is_empty() && name.to_lowercase().starts_with(&prefix)
            })
            .map(|(name, members)| RecipientCandidate::Group {
                name: name.clone(),
                members: members.clone(),
            })
            .take(limit)
            .collect();
        candidates.extend(
            self.people
                .iter()
                .filter(|person| matches(person, &prefix))
                .flat_map(offered_addresses)
                .map(RecipientCandidate::Contact),
        );
        candidates.truncate(limit);
        candidates
    }
}

/// Whether `person` completes `prefix`, which is already lowercase.
fn matches(person: &Contact, prefix: &str) -> bool {
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

/// The address a group member expands to: their preferred one (FR-041).
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
    use super::*;
    use postio_model::{AddressId, ContactAddress, ContactId};

    fn owned(id: i64, address: &str) -> ContactAddress {
        ContactAddress {
            id: AddressId::new(id),
            address: EmailAddress::new(None::<String>, address),
            times_seen: 0,
            last_seen_at: None,
            written: 0,
        }
    }

    fn person(id: i64, name: Option<&str>, addresses: &[&str]) -> Contact {
        let mut person = Contact::new(owned(id * 10, addresses[0]));
        person.id = ContactId::new(id);
        person.name = name.map(str::to_owned);
        for (i, address) in addresses.iter().enumerate().skip(1) {
            person.addresses.push(owned(id * 10 + i as i64, address));
        }
        person
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

    fn directory() -> Directory {
        Directory::new(
            vec![
                (
                    "Garden club".to_owned(),
                    vec![EmailAddress::new(None::<String>, "grace@example.com")],
                ),
                ("Ghosts".to_owned(), Vec::new()),
            ],
            vec![
                person(1, Some("Grace Hopper"), &["grace@example.com"]),
                person(2, Some("Ada Lovelace"), &["ada@example.com"]),
                person(3, None, &["gh-bot@example.com"]),
            ],
        )
    }

    #[test]
    fn a_prefix_of_an_address_the_name_or_a_word_in_it_completes() {
        let directory = directory();
        assert_eq!(
            addresses(&directory.suggest("ada@", 8)),
            vec!["ada@example.com"]
        );
        assert_eq!(
            addresses(&directory.suggest("lovel", 8)),
            vec!["ada@example.com"],
            "the second word of a name"
        );
        assert_eq!(
            addresses(&directory.suggest("HOP", 8)),
            vec!["grace@example.com"],
            "case does not matter"
        );
        assert!(
            directory.suggest("race", 8).is_empty(),
            "the middle of a word is not a beginning"
        );
    }

    #[test]
    fn groups_come_first_and_an_empty_one_is_never_offered() {
        assert_eq!(
            addresses(&directory().suggest("g", 8)),
            vec![
                "group:Garden club",
                "grace@example.com",
                "gh-bot@example.com"
            ]
        );
    }

    #[test]
    fn the_store_order_is_kept_and_the_limit_holds() {
        assert_eq!(
            addresses(&directory().suggest("", 2)),
            vec!["group:Garden club", "grace@example.com"]
        );
    }

    #[test]
    fn a_person_is_offered_once_per_address_preferred_first_under_one_name() {
        // FR-030: the second address completes too, and both carry the name
        // the person has -- not whatever each address's mail said.
        let mut ada = person(2, None, &["ada@work.example", "ada@home.example"]);
        ada.seen_name = Some("Ada Lovelace".to_owned());
        let directory = Directory::new(Vec::new(), vec![ada]);
        let offered = directory.suggest("ada@home", 8);
        assert_eq!(
            addresses(&offered),
            vec!["ada@work.example", "ada@home.example"],
            "a match on any address offers the person, preferred address first"
        );
        assert!(offered.iter().all(|candidate| matches!(
            candidate,
            RecipientCandidate::Contact(address) if address.name.as_deref() == Some("Ada Lovelace")
        )));
    }
}
