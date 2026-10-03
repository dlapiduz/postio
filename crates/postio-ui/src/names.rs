//! The address book the command bar reads a sentence against (screen 07):
//! "the invoice Ada sent last month" says `from:ada` only because someone
//! called Ada has written. `postio_search::natural::lower` asks, for a name
//! as typed, what `from:` or `to:` should say; this answers it from the
//! account's correspondents.

use std::collections::{HashMap, HashSet};

use postio_model::Contact;

/// Who a typed name can mean.
#[derive(Debug, Default, Clone)]
pub struct Names {
    /// A whole display name, lowercased, and the address it belongs to.
    whole: HashMap<String, String>,
    /// A first name or an address's local part, lowercased.
    short: HashSet<String>,
}

impl Names {
    /// The directory `contacts` make.
    pub fn new(contacts: &[Contact]) -> Self {
        let mut names = Names::default();
        for contact in contacts {
            let address = contact.address.address.to_lowercase();
            let shown = contact.display_name().trim().to_lowercase();
            if !shown.is_empty() && shown != address {
                names.whole.entry(shown.clone()).or_insert(address.clone());
                if let Some(first) = shown.split_whitespace().next() {
                    names.short.insert(first.to_owned());
                }
            }
            if let Some(local) = contact.address.local_part() {
                names.short.insert(local.to_lowercase());
            }
        }
        names
    }

    /// What `from:` should say for `typed`: the address, for a whole name
    /// that names one correspondent; the word itself, for a first name
    /// someone has; nothing for a name nobody has.
    pub fn lookup(&self, typed: &str) -> Option<String> {
        let key = typed.trim().to_lowercase();
        if key.is_empty() {
            return None;
        }
        if key.contains(char::is_whitespace) {
            return self.whole.get(&key).cloned();
        }
        self.short.contains(&key).then_some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::EmailAddress;

    fn contact(name: &str, address: &str) -> Contact {
        Contact::new(EmailAddress::new(Some(name), address))
    }

    #[test]
    fn a_first_name_a_whole_name_and_a_stranger() {
        let names = Names::new(&[
            contact("Ada Moreno", "ada.moreno@example.org"),
            contact("Lena Park", "lp@example.com"),
        ]);
        assert_eq!(names.lookup("Ada").as_deref(), Some("ada"));
        assert_eq!(
            names.lookup("ada moreno").as_deref(),
            Some("ada.moreno@example.org")
        );
        assert_eq!(names.lookup("lp").as_deref(), Some("lp"));
        assert_eq!(names.lookup("invoice"), None);
        assert_eq!(names.lookup("Ada Park"), None);
        assert_eq!(names.lookup(""), None);
    }
}
