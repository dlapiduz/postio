//! Recipient completion, answered from memory.
//!
//! The composer asks for candidates on every keystroke in `To`, `Cc` and
//! `Bcc`, synchronously, on the GTK thread. That used to be a fresh store
//! connection, a groups read, one members read per matching group, and a
//! contacts query of five `LIKE`s with a leading wildcard -- a scan of the
//! table, per key, on the thread that draws. The directory is small (the
//! search surface already holds the same list for `@`) and changes slowly,
//! so it is read off the thread whenever the composer opens and each key is
//! answered by [`Directory::suggest`], which is arithmetic.
//!
//! The matching rules are the ones `ContactRepository::search` applied, kept
//! so that nothing about *what* is offered changed: a prefix of the address,
//! of the contact's name or the address's display name, or of any word in
//! either name; mail-sourced contacts after the ones the user made; then the
//! order the store read them in -- most recently seen first.

use postio_gtk::composer::RecipientCandidate;
use postio_model::{Contact, EmailAddress};

/// Every contact and group a composer can complete, as last read.
#[derive(Debug, Default, Clone)]
pub struct Directory {
    /// Named groups with their members' addresses, in the store's order.
    groups: Vec<(String, Vec<EmailAddress>)>,
    /// Contacts in the order the store ranks them.
    contacts: Vec<Contact>,
}

impl Directory {
    /// A directory over `groups` and `contacts`, which arrive in the order
    /// they are to be offered.
    pub fn new(groups: Vec<(String, Vec<EmailAddress>)>, contacts: Vec<Contact>) -> Self {
        Self { groups, contacts }
    }

    /// At most `limit` candidates for `prefix`: groups whose name begins
    /// with it first -- a group is a deliberate choice the user is likelier
    /// typing towards -- then contacts.
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
        if candidates.len() < limit {
            candidates.extend(
                self.contacts
                    .iter()
                    .filter(|contact| !contact.suppressed && matches(contact, &prefix))
                    .take(limit - candidates.len())
                    .map(|contact| RecipientCandidate::Contact(resolved_address(contact))),
            );
        }
        candidates
    }
}

/// Whether `contact` completes `prefix`, which is already lowercase.
fn matches(contact: &Contact, prefix: &str) -> bool {
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

/// The address a contact completes to: its own name when it has one, the
/// address's display name otherwise.
pub fn resolved_address(contact: &Contact) -> EmailAddress {
    let name = contact
        .name
        .clone()
        .or_else(|| contact.address.name.clone());
    EmailAddress::new(name, contact.address.address.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(name: Option<&str>, address: &str) -> Contact {
        let mut contact = Contact::new(EmailAddress::new(None::<String>, address));
        contact.name = name.map(str::to_owned);
        contact
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
                contact(Some("Grace Hopper"), "grace@example.com"),
                contact(Some("Ada Lovelace"), "ada@example.com"),
                contact(None, "gh-bot@example.com"),
            ],
        )
    }

    #[test]
    fn a_prefix_of_the_address_the_name_or_a_word_in_it_completes() {
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
    fn a_suppressed_contact_is_not_offered() {
        let mut deleted = contact(Some("Ada Lovelace"), "ada@example.com");
        deleted.suppressed = true;
        let directory = Directory::new(Vec::new(), vec![deleted]);
        assert!(directory.suggest("ada", 8).is_empty());
    }

    #[test]
    fn a_contact_completes_to_its_own_name_before_the_address_name() {
        let mut named = contact(Some("Ada Lovelace"), "ada@example.com");
        named.address.name = Some("A. King".to_owned());
        let directory = Directory::new(Vec::new(), vec![named]);
        let [RecipientCandidate::Contact(address)] = &directory.suggest("ada", 8)[..] else {
            panic!("one contact");
        };
        assert_eq!(address.name.as_deref(), Some("Ada Lovelace"));
    }
}
