//! The Edit and New contact form as plain data, so every frontend draws the same one and the engine
//! saves it as one command.

use serde::{Deserialize, Serialize};

use crate::EmailAddress;
use crate::contact::{ContactAddress, ContactCardRead, ContactKind, ShowAs};
use crate::ids::AddressId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AddressLabel {
    Work,
    Personal,
    Old,
}

impl AddressLabel {
    pub const ALL: [Self; 3] = [Self::Work, Self::Personal, Self::Old];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Personal => "personal",
            Self::Old => "old",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|label| label.as_str() == name)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Work => "Work",
            Self::Personal => "Personal",
            Self::Old => "Old",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressNote {
    pub address: AddressId,
    pub label: Option<AddressLabel>,
    pub bounced: bool,
    pub bounced_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormAddress {
    /// The stored address this row is, `None` for one just typed.
    pub id: Option<AddressId>,
    pub address: String,
    pub label: Option<AddressLabel>,
    /// Whether a bounce named it: drawn "Old · bounced", never edited.
    pub bounced: bool,
}

impl FormAddress {
    pub fn blank() -> Self {
        Self {
            id: None,
            address: String::new(),
            label: None,
            bounced: false,
        }
    }

    pub fn label_text(&self) -> String {
        match (self.label, self.bounced) {
            (_, true) => "Old \u{b7} bounced".to_owned(),
            (Some(label), _) => label.title().to_owned(),
            (None, false) => "Other".to_owned(),
        }
    }

    fn typed(&self) -> &str {
        self.address.trim()
    }
}

/// Read on this machine from the newest mail the person sent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureDetails {
    pub name: Option<String>,
    pub title: Option<String>,
    pub company: Option<String>,
    pub phones: Vec<String>,
    /// As written, without a scheme.
    pub websites: Vec<String>,
}

impl SignatureDetails {
    /// Name, title and company, folded; `None` unless all three are there, since any two match
    /// colleagues and namesakes.
    pub fn key(&self) -> Option<String> {
        let fold = |text: &Option<String>| {
            text.as_deref()
                .map(|t| {
                    t.split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_lowercase()
                })
                .filter(|t| !t.is_empty())
        };
        Some(format!(
            "{}|{}|{}",
            fold(&self.name)?,
            fold(&self.title)?,
            fold(&self.company)?
        ))
    }

    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.company.is_none()
            && self.phones.is_empty()
            && self.websites.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvidenceKind {
    Signature,
    Phone,
    Resent,
    Card,
}

impl EvidenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Signature => "signature",
            Self::Phone => "phone",
            Self::Resent => "resent",
            Self::Card => "card",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairEvidence {
    pub signature: Option<String>,
    /// A phone number both give.
    pub phone: Option<String>,
    /// A `Resent-From` naming one record's address on the other's mail:
    /// the address it forwards from, and the address that forwards.
    pub resent: Option<(String, String)>,
    pub card: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptDetail {
    pub kind: DetailKind,
    pub value: String,
    pub added: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DetailKind {
    Phone,
    Url,
}

impl DetailKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Url => "url",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "phone" => Some(Self::Phone),
            "url" => Some(Self::Url),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenItem {
    pub kind: DetailKind,
    pub value: String,
    pub kept: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Introduction {
    pub details: SignatureDetails,
    pub last_at: Option<chrono::DateTime<chrono::Utc>>,
    pub with: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderPrior {
    pub saved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub title: Option<String>,
    pub organization: Option<String>,
    pub added: Vec<(DetailKind, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormProblem {
    NoAddress,
    NotAnAddress(String),
    Twice(String),
    NoName,
}

impl FormProblem {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactForm {
    pub kind: ContactKind,
    pub given: String,
    pub family: String,
    pub show_as: ShowAs,
    pub title: String,
    pub company: String,
    pub addresses: Vec<FormAddress>,
    /// Which address replies and new mail go to: an index into `addresses`.
    pub primary: usize,
    pub groups: Vec<String>,
}

impl ContactForm {
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

    pub fn full_name(&self) -> String {
        let parts = [self.given.trim(), self.family.trim()];
        parts
            .iter()
            .filter(|part| !part.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    }

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

    #[test]
    fn stored_words_round_trip_and_unknown_ones_are_refused() {
        for label in AddressLabel::ALL {
            assert_eq!(AddressLabel::from_name(label.as_str()), Some(label));
            assert!(!label.title().is_empty());
        }
        assert_eq!(AddressLabel::from_name("home"), None);
        for kind in [DetailKind::Phone, DetailKind::Url] {
            assert_eq!(DetailKind::from_name(kind.as_str()), Some(kind));
        }
        assert_eq!(DetailKind::from_name("fax"), None);
        let words: Vec<&str> = [
            EvidenceKind::Signature,
            EvidenceKind::Phone,
            EvidenceKind::Resent,
            EvidenceKind::Card,
        ]
        .into_iter()
        .map(EvidenceKind::as_str)
        .collect();
        assert_eq!(words, ["signature", "phone", "resent", "card"]);
    }

    #[test]
    fn a_signature_key_needs_all_three_and_folds_spacing_and_case() {
        let said =
            |name: Option<&str>, title: Option<&str>, company: Option<&str>| SignatureDetails {
                name: name.map(str::to_owned),
                title: title.map(str::to_owned),
                company: company.map(str::to_owned),
                ..SignatureDetails::default()
            };
        assert_eq!(
            said(Some("Ada  Moreno"), Some("Finance Lead"), Some("Northwind")).key(),
            Some("ada moreno|finance lead|northwind".to_owned())
        );
        assert_eq!(said(Some("Ada"), Some("Lead"), None).key(), None);
        assert_eq!(said(Some("Ada"), Some("  "), Some("X")).key(), None);
        assert!(said(Some("Ada"), None, None).is_empty());
        assert!(
            !SignatureDetails {
                phones: vec!["+1 555 0119".into()],
                ..SignatureDetails::default()
            }
            .is_empty()
        );
    }

    #[test]
    fn the_form_of_a_card_starts_from_what_the_person_is_called_and_their_addresses() {
        use crate::{
            AddressId, ContactAddress, ContactArrival, ContactCardRead, ContactId, ContactSettings,
            ContactStats, EmailAddress,
        };
        let mut person = crate::Contact::new(ContactAddress::new(
            AddressId::new(7),
            EmailAddress::new(Some("Ada Moreno"), "ada@example.org"),
        ));
        person.id = ContactId::new(3);
        person.title = Some("Finance lead".into());
        let card = ContactCardRead {
            person,
            groups: vec!["Atlas core".into()],
            stats: ContactStats::default(),
            open: Vec::new(),
            settings: ContactSettings::default(),
            default_arrival: ContactArrival::Inbox,
            files: Vec::new(),
            issues: Vec::new(),
            address_notes: Vec::new(),
            keys: Vec::new(),
        };
        let note = AddressNote {
            address: AddressId::new(7),
            label: Some(AddressLabel::Work),
            bounced: true,
            bounced_at: None,
        };
        let form = ContactForm::of(&card, &[note]);
        assert_eq!(
            (form.given.as_str(), form.family.as_str()),
            ("Ada", "Moreno")
        );
        assert_eq!(form.title, "Finance lead");
        assert_eq!(form.groups, ["Atlas core"]);
        assert_eq!(form.addresses.len(), 1);
        assert_eq!(form.addresses[0].label, Some(AddressLabel::Work));
        assert!(form.addresses[0].bounced);
        assert_eq!(form.primary, 0);
    }

    #[test]
    fn every_problem_says_what_to_do() {
        assert_eq!(FormProblem::NoAddress.text(), "Add at least one address.");
        assert!(
            FormProblem::NotAnAddress("x".into())
                .text()
                .contains("not an email")
        );
        assert!(FormProblem::Twice("a@b.c".into()).text().contains("twice"));
        assert_eq!(FormProblem::NoName.text(), "Give them a name.");
    }

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
        assert_eq!(
            FormAddress::blank().label_text(),
            "Other",
            "an untyped address"
        );
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
