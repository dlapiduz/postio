//! The Edit and New contact form's content (specs/011-focus-contacts S8,
//! handoff screen 06): what the form holds, what makes it saveable, and what
//! saving it means.
//!
//! The form is plain data so every frontend draws the same one and the engine
//! saves it as one command: a person is made or changed all at once, and the
//! undo is the form as it was.

use serde::{Deserialize, Serialize};

use crate::EmailAddress;
use crate::contact::{ContactAddress, ContactCardRead, ContactKind, ShowAs};
use crate::ids::AddressId;

/// What the user calls one of a person's addresses (the type menu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AddressLabel {
    /// Their work address.
    Work,
    /// Their own.
    Personal,
    /// One they no longer use.
    Old,
}

impl AddressLabel {
    /// Every label, in the type menu's order.
    pub const ALL: [Self; 3] = [Self::Work, Self::Personal, Self::Old];

    /// The stored identifier.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Personal => "personal",
            Self::Old => "old",
        }
    }

    /// The inverse of [`AddressLabel::as_str`].
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|label| label.as_str() == name)
    }

    /// The word the menu shows.
    pub fn title(self) -> &'static str {
        match self {
            Self::Work => "Work",
            Self::Personal => "Personal",
            Self::Old => "Old",
        }
    }
}

/// What the store knows about one of a person's addresses that the person's
/// own record does not carry: the label the user gave it, and whether a
/// delivery failure named it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressNote {
    /// Which address.
    pub address: AddressId,
    /// What the user calls it.
    pub label: Option<AddressLabel>,
    /// Whether a bounce named it.
    pub bounced: bool,
}

/// One address row of the form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormAddress {
    /// The stored address this row is, `None` for one just typed.
    pub id: Option<AddressId>,
    /// What is typed.
    pub address: String,
    /// The type menu's choice.
    pub label: Option<AddressLabel>,
    /// Whether a bounce named it: drawn "Old · bounced", never edited.
    pub bounced: bool,
}

impl FormAddress {
    /// An empty row, as "+ Add address" adds.
    pub fn blank() -> Self {
        Self {
            id: None,
            address: String::new(),
            label: None,
            bounced: false,
        }
    }

    /// What the type menu says for this row.
    pub fn label_text(&self) -> String {
        match (self.label, self.bounced) {
            (_, true) => "Old \u{b7} bounced".to_owned(),
            (Some(label), _) => label.title().to_owned(),
            (None, false) => String::new(),
        }
    }

    fn typed(&self) -> &str {
        self.address.trim()
    }
}

/// Why a form cannot be saved yet, in words for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormProblem {
    /// No address is filled in.
    NoAddress,
    /// An address does not look like one.
    NotAnAddress(String),
    /// The same address twice.
    Twice(String),
    /// No name, and no company to stand for one.
    NoName,
}

impl FormProblem {
    /// What the form says beside the Save button.
    pub fn text(&self) -> String {
        match self {
            Self::NoAddress => "Add at least one address.".to_owned(),
            Self::NotAnAddress(typed) => {
                format!("\u{201c}{typed}\u{201d} is not an email address.")
            }
            Self::Twice(typed) => format!("{typed} is in the list twice."),
            Self::NoName => "Give them a name.".to_owned(),
        }
    }
}

/// The whole Edit form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactForm {
    /// Person, company, list, newsletter or automated.
    pub kind: ContactKind,
    /// First name.
    pub given: String,
    /// Last name.
    pub family: String,
    /// How the inbox names them.
    pub show_as: ShowAs,
    /// Job title.
    pub title: String,
    /// Company.
    pub company: String,
    /// Addresses, in the order drawn.
    pub addresses: Vec<FormAddress>,
    /// Which address replies and new mail go to: an index into `addresses`.
    pub primary: usize,
    /// The groups they are in, by name.
    pub groups: Vec<String>,
}

impl ContactForm {
    /// The New contact form: nothing typed, one empty address row.
    pub fn blank() -> Self {
        Self {
            kind: ContactKind::Person,
            given: String::new(),
            family: String::new(),
            show_as: ShowAs::Full,
            title: String::new(),
            company: String::new(),
            addresses: vec![FormAddress::blank()],
            primary: 0,
            groups: Vec::new(),
        }
    }

    /// The Edit form for `card`'s person, `notes` saying what each address
    /// is called.
    pub fn of(card: &ContactCardRead, notes: &[AddressNote]) -> Self {
        let person = &card.person;
        let (given, family) = match (&person.given_name, &person.family_name) {
            // Never split by the user: what the person is called now, unless
            // that is only an address, which is not a name to start from.
            (None, None) => {
                let shown = person.display_name();
                split_name(if shown.contains('@') { "" } else { shown })
            }
            (given, family) => (
                given.clone().unwrap_or_default(),
                family.clone().unwrap_or_default(),
            ),
        };
        let addresses: Vec<FormAddress> = person
            .addresses
            .iter()
            .map(|owned: &ContactAddress| {
                let note = notes.iter().find(|note| note.address == owned.id);
                FormAddress {
                    id: Some(owned.id),
                    address: owned.address.address.clone(),
                    label: note.and_then(|note| note.label),
                    bounced: note.is_some_and(|note| note.bounced),
                }
            })
            .collect();
        let primary = person
            .addresses
            .iter()
            .position(|owned| owned.id == person.preferred)
            .unwrap_or(0);
        Self {
            kind: person.kind,
            given,
            family,
            show_as: person.show_as,
            title: person.title.clone().unwrap_or_default(),
            company: person.organization.clone().unwrap_or_default(),
            addresses,
            primary,
            groups: card.groups.clone(),
        }
    }

    /// The name the form makes: first and last, as typed.
    pub fn full_name(&self) -> String {
        let parts = [self.given.trim(), self.family.trim()];
        parts
            .iter()
            .filter(|part| !part.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// What the inbox will call them, in the chosen style: what the Show as
    /// segments preview.
    pub fn shown_as(&self, style: ShowAs) -> String {
        let (given, family) = (self.given.trim(), self.family.trim());
        match style {
            ShowAs::First if !given.is_empty() => given.to_owned(),
            ShowAs::LastFirst if !family.is_empty() && !given.is_empty() => {
                format!("{family}, {given}")
            }
            ShowAs::LastFirst if !family.is_empty() => family.to_owned(),
            _ => self.full_name(),
        }
    }

    /// Why this cannot be saved, or `None` when it can.
    pub fn problem(&self) -> Option<FormProblem> {
        let filled: Vec<&FormAddress> = self
            .addresses
            .iter()
            .filter(|row| !row.typed().is_empty())
            .collect();
        if filled.is_empty() {
            return Some(FormProblem::NoAddress);
        }
        let mut seen: Vec<String> = Vec::new();
        for row in &filled {
            let email = EmailAddress::new(None::<String>, row.typed());
            if !email.is_plausible() {
                return Some(FormProblem::NotAnAddress(row.typed().to_owned()));
            }
            let key = email.normalized();
            if seen.contains(&key) {
                return Some(FormProblem::Twice(row.typed().to_owned()));
            }
            seen.push(key);
        }
        if self.full_name().is_empty() && self.company.trim().is_empty() {
            return Some(FormProblem::NoName);
        }
        None
    }

    /// The form as it will be saved: empty rows gone, the primary still the
    /// row it was, groups without repeats.
    pub fn tidy(&self) -> Self {
        let mut tidy = self.clone();
        let mut kept: Vec<FormAddress> = Vec::new();
        let mut primary = 0;
        for (index, row) in self.addresses.iter().enumerate() {
            if row.typed().is_empty() {
                continue;
            }
            if index == self.primary {
                primary = kept.len();
            }
            let mut row = row.clone();
            row.address = row.typed().to_owned();
            kept.push(row);
        }
        tidy.addresses = kept;
        tidy.primary = primary;
        let mut groups: Vec<String> = Vec::new();
        for group in &self.groups {
            let group = group.trim();
            if !group.is_empty() && !groups.iter().any(|g| g.eq_ignore_ascii_case(group)) {
                groups.push(group.to_owned());
            }
        }
        tidy.groups = groups;
        tidy
    }
}

/// "Ada Moreno" as first and last: everything after the first space is the
/// last name. One word is a first name.
pub fn split_name(name: &str) -> (String, String) {
    let name = name.trim();
    match name.split_once(char::is_whitespace) {
        Some((first, rest)) => (first.to_owned(), rest.trim().to_owned()),
        None => (name.to_owned(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(address: &str) -> FormAddress {
        FormAddress {
            address: address.to_owned(),
            ..FormAddress::blank()
        }
    }

    fn filled() -> ContactForm {
        ContactForm {
            given: "Ada".into(),
            family: "Moreno".into(),
            addresses: vec![row("ada@example.org")],
            ..ContactForm::blank()
        }
    }

    #[test]
    fn a_blank_form_asks_for_an_address_first() {
        assert_eq!(ContactForm::blank().problem(), Some(FormProblem::NoAddress));
    }

    #[test]
    fn a_form_with_a_name_and_an_address_saves() {
        assert_eq!(filled().problem(), None);
    }

    #[test]
    fn a_typo_in_an_address_is_named_back_to_the_person() {
        let mut form = filled();
        form.addresses.push(row("ada.at.example"));
        assert_eq!(
            form.problem(),
            Some(FormProblem::NotAnAddress("ada.at.example".into()))
        );
        assert_eq!(
            form.problem().unwrap().text(),
            "\u{201c}ada.at.example\u{201d} is not an email address."
        );
    }

    #[test]
    fn the_same_address_twice_is_refused_whatever_its_case() {
        let mut form = filled();
        form.addresses.push(row("ADA@example.org"));
        assert!(matches!(form.problem(), Some(FormProblem::Twice(_))));
    }

    #[test]
    fn a_company_may_stand_without_a_personal_name() {
        let mut form = filled();
        form.given.clear();
        form.family.clear();
        assert_eq!(form.problem(), Some(FormProblem::NoName));
        form.company = "Northwind Partners".into();
        assert_eq!(form.problem(), None);
    }

    #[test]
    fn show_as_previews_each_style_from_the_names_typed() {
        let form = filled();
        assert_eq!(form.shown_as(ShowAs::Full), "Ada Moreno");
        assert_eq!(form.shown_as(ShowAs::First), "Ada");
        assert_eq!(form.shown_as(ShowAs::LastFirst), "Moreno, Ada");
    }

    #[test]
    fn tidying_drops_empty_rows_and_keeps_the_primary_on_its_row() {
        let mut form = filled();
        form.addresses = vec![row(""), row(" a@example.org "), row("b@example.org")];
        form.primary = 2;
        form.groups = vec!["Finance".into(), "finance".into(), " ".into()];
        let tidy = form.tidy();
        assert_eq!(tidy.addresses.len(), 2);
        assert_eq!(tidy.addresses[0].address, "a@example.org");
        assert_eq!(tidy.primary, 1);
        assert_eq!(tidy.groups, ["Finance"]);
    }

    #[test]
    fn a_bounced_address_reads_old_and_bounced_whatever_it_was_called() {
        let mut address = row("a@example.org");
        address.label = Some(AddressLabel::Work);
        assert_eq!(address.label_text(), "Work");
        address.bounced = true;
        assert_eq!(address.label_text(), "Old \u{b7} bounced");
    }

    #[test]
    fn a_name_the_user_set_splits_at_its_first_space() {
        assert_eq!(split_name("Ada Moreno"), ("Ada".into(), "Moreno".into()));
        assert_eq!(
            split_name("Ana de la Cruz"),
            ("Ana".into(), "de la Cruz".into())
        );
        assert_eq!(split_name("Cher"), ("Cher".into(), String::new()));
    }
}
