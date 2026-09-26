//! Recipient completion: when it is offered, how a suggestion reads, and
//! what accepting one does to the field.
//!
//! Shared so the desktop's popover and the terminal's list behave alike.

use postio_model::address::current_entry;
use postio_model::contact_group::RecipientCandidate;

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

#[cfg(test)]
mod tests {
    use postio_model::EmailAddress;

    use super::*;

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
