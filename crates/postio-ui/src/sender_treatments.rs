//! A sender's remembered treatment (specs/011-focus-contacts S3, ADR 0048).
//! `Remembered` is the in-memory table a reader holds, so it need not ask the store per message.

use std::cell::RefCell;
use std::collections::BTreeMap;

use postio_body::treatment::Treatment;

pub trait SenderTreatments {
    fn treatment_for(&self, sender: &str) -> Option<Treatment>;

    fn set_treatment(&self, sender: &str, treatment: Option<Treatment>);
}

type Changed = Box<dyn Fn(&str, Option<Treatment>)>;

pub struct Remembered {
    chosen: RefCell<BTreeMap<String, Treatment>>,
    changed: Changed,
}

impl Remembered {
    pub fn new(changed: impl Fn(&str, Option<Treatment>) + 'static) -> Self {
        Remembered {
            chosen: RefCell::default(),
            changed: Box::new(changed),
        }
    }

    /// A word that names no treatment is left out.
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
        type Told = Rc<RefCell<Vec<(String, Option<Treatment>)>>>;
        let told: Told = Rc::default();
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
