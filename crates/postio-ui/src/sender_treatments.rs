//! A sender's remembered treatment, behind one interface
//! (specs/011-focus-contacts S3, ADR 0048).
//!
//! "Always draw this sender's mail this way" is the person's, so it lives on
//! the contact in the store, and a reader that looks senders up as a message
//! opens cannot ask the store each time. [`SenderTreatments`] is what a
//! reader holds; [`Remembered`] is the table the host fills it from, kept in
//! memory, which answers at once and tells the host what changed.

use std::cell::RefCell;
use std::collections::BTreeMap;

use postio_body::treatment::Treatment;

/// Whatever remembers how a sender's mail is always drawn.
pub trait SenderTreatments {
    /// The treatment chosen for `sender`, if one was.
    fn treatment_for(&self, sender: &str) -> Option<Treatment>;

    /// Remember `treatment` for `sender`, or forget the choice with `None`.
    fn set_treatment(&self, sender: &str, treatment: Option<Treatment>);
}

/// Treatments held in memory and kept in step with the store by the app that
/// owns the reader: it fills the table ([`Remembered::replace`]) from what
/// the host holds and hears every change through the callback given to
/// [`Remembered::new`].
pub struct Remembered {
    chosen: RefCell<BTreeMap<String, Treatment>>,
    changed: Box<dyn Fn(&str, Option<Treatment>)>,
}

impl Remembered {
    /// An empty table that calls `changed` with each `(sender, treatment)`
    /// it is given to remember.
    pub fn new(changed: impl Fn(&str, Option<Treatment>) + 'static) -> Self {
        Remembered {
            chosen: RefCell::default(),
            changed: Box::new(changed),
        }
    }

    /// Replace the table with what the store holds: `(address, "app" |
    /// "paper")`, an entry per address of each person who chose. A word that
    /// names no treatment is left out.
    pub fn replace(&self, entries: impl IntoIterator<Item = (String, String)>) {
        *self.chosen.borrow_mut() = entries
            .into_iter()
            .filter_map(|(sender, word)| {
                Some((normalize(&sender), Treatment::from_attribute(&word)?))
            })
            .collect();
    }
}

impl SenderTreatments for Remembered {
    fn treatment_for(&self, sender: &str) -> Option<Treatment> {
        self.chosen.borrow().get(&normalize(sender)).copied()
    }

    fn set_treatment(&self, sender: &str, treatment: Option<Treatment>) {
        let key = normalize(sender);
        if key.is_empty() {
            return;
        }
        match treatment {
            Some(treatment) => {
                self.chosen.borrow_mut().insert(key.clone(), treatment);
            }
            None => {
                self.chosen.borrow_mut().remove(&key);
            }
        }
        (self.changed)(&key, treatment);
    }
}

fn normalize(sender: &str) -> String {
    sender.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    #[test]
    fn the_table_answers_at_once_in_any_case_and_tells_the_store_what_changed() {
        let told: Rc<RefCell<Vec<(String, Option<Treatment>)>>> = Rc::default();
        let log = Rc::clone(&told);
        let table = Remembered::new(move |sender, treatment| {
            log.borrow_mut().push((sender.to_owned(), treatment));
        });
        table.replace([
            ("News@Example.com".to_owned(), "paper".to_owned()),
            ("odd@example.com".to_owned(), "sepia".to_owned()),
        ]);
        assert_eq!(
            table.treatment_for(" news@example.COM "),
            Some(Treatment::Paper)
        );
        assert_eq!(
            table.treatment_for("odd@example.com"),
            None,
            "a word that is not a treatment"
        );

        table.set_treatment("Bea@Example.org", Some(Treatment::AppColours));
        assert_eq!(
            table.treatment_for("bea@example.org"),
            Some(Treatment::AppColours)
        );
        table.set_treatment("news@example.com", None);
        assert_eq!(table.treatment_for("news@example.com"), None);
        table.set_treatment("   ", Some(Treatment::Paper));
        assert_eq!(
            *told.borrow(),
            [
                ("bea@example.org".to_owned(), Some(Treatment::AppColours)),
                ("news@example.com".to_owned(), None)
            ],
            "the store hears each change, and nothing for nobody"
        );
    }

    #[test]
    fn a_new_table_remembers_nothing_and_a_replace_forgets_what_the_store_no_longer_holds() {
        let table = Remembered::new(|_, _| {});
        assert_eq!(table.treatment_for("a@example.com"), None);
        table.replace([("a@example.com".to_owned(), "app".to_owned())]);
        table.replace([]);
        assert_eq!(table.treatment_for("a@example.com"), None);
    }
}
