//! The merge sheet, in words (specs/011-focus-contacts S10, handoff screen
//! 07): the records that look like one person, which one's name and photo
//! are kept, why they look alike, what the result would be, and the three
//! ways out. No toolkit here; every frontend draws the same sheet.

use chrono::{DateTime, Local};
use postio_core::{CommandId, Keymap};
use postio_model::{Contact, ContactKind, MergeGroup};

use crate::contact_edit::EditButton;
use crate::contact_popover::{Avatar, avatar, last_word, role_line};

/// One record's card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeCard {
    /// The avatar.
    pub avatar: Avatar,
    /// "Ada Moreno".
    pub name: String,
    /// "saved" or "not saved".
    pub source: String,
    /// The address it would show first.
    pub address: String,
    /// "412 messages \u{b7} last today".
    pub stats: String,
    /// "Keep this name and photo" on the kept record, else "Address kept".
    pub keep: String,
    /// Whether it is the kept record: drawn with the accent ring.
    pub kept: bool,
}

/// What the merge would make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    /// The kept record's avatar.
    pub avatar: Avatar,
    /// "Ada Moreno".
    pub name: String,
    /// "Finance lead, Northwind Partners".
    pub role: String,
    /// "\u{2605} ada@example.org \u{b7} a.moreno@example.net".
    pub addresses: String,
    /// "Result \u{b7} 431 messages \u{b7} saved \u{b7} the two unsaved records are folded in".
    pub line: String,
}

/// The sheet, drawn whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeSheetView {
    /// "These 3 look like the same person".
    pub title: String,
    /// "Merging keeps every address, ..."
    pub subtitle: String,
    /// "2 of 5": where this group is in the queue; none for records chosen
    /// by hand.
    pub position: Option<String>,
    /// One card per record.
    pub cards: Vec<MergeCard>,
    /// "Why we think so".
    pub why_title: String,
    /// The evidence, a line each.
    pub evidence: Vec<String>,
    /// What the merge would make.
    pub result: MergeResult,
    /// "Undo with \u{2318}Z. Merged records can be split again from the card."
    pub foot: String,
    /// Not the same, with its key.
    pub not_same: EditButton,
    /// Later, with its key.
    pub later: EditButton,
    /// Merge, with its key.
    pub merge: EditButton,
}

fn number_word(n: usize) -> String {
    match n {
        2 => "2".to_owned(),
        n => n.to_string(),
    }
}

fn count_word(n: usize) -> &'static str {
    match n {
        1 => "one",
        2 => "two",
        3 => "three",
        4 => "four",
        5 => "five",
        _ => "more",
    }
}

fn first_address(person: &Contact) -> String {
    person
        .preferred_address()
        .or_else(|| person.addresses.first())
        .map(|owned| owned.address.address.clone())
        .unwrap_or_default()
}

/// The sheet for `group`, with the record at `kept` keeping its name and
/// photo.
pub fn view(
    group: &MergeGroup,
    kept: usize,
    position: Option<(usize, usize)>,
    keymap: &Keymap,
    now: DateTime<Local>,
    synced_with: Option<&str>,
) -> MergeSheetView {
    let kept = kept.min(group.people.len().saturating_sub(1));
    let keeper = &group.people[kept];
    let key = |command| keymap.binding(command).map(str::to_owned);
    let button = |label: &str, command| EditButton {
        label: label.to_owned(),
        key: key(command),
    };
    let cards = group
        .people
        .iter()
        .enumerate()
        .map(|(index, person)| {
            let name = person.shown_as().into_owned();
            let messages = person.times_seen;
            MergeCard {
                avatar: avatar(person.id, &name, person.kind),
                name,
                source: if person.is_saved() {
                    "saved"
                } else {
                    "not saved"
                }
                .to_owned(),
                address: first_address(person),
                stats: format!(
                    "{messages} message{} \u{b7} last {}",
                    if messages == 1 { "" } else { "s" },
                    last_word(person.last_seen_at, now)
                ),
                keep: if index == kept {
                    "Keep this name and photo"
                } else {
                    "Address kept"
                }
                .to_owned(),
                kept: index == kept,
            }
        })
        .collect();
    let others: Vec<&Contact> = group
        .people
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != kept)
        .map(|(_, person)| person)
        .collect();
    let unsaved = others.iter().filter(|p| !p.is_saved()).count();
    let total: u32 = group.people.iter().map(|p| p.times_seen).sum();
    let addresses: Vec<String> = std::iter::once(keeper)
        .chain(others.iter().copied())
        .flat_map(|person| {
            person
                .addresses
                .iter()
                .map(|owned| owned.address.address.clone())
        })
        .collect();
    let mut shown = String::new();
    for (index, address) in addresses.iter().enumerate() {
        if index == 0 {
            shown.push_str(&format!("\u{2605} {address}"));
        } else {
            shown.push_str(&format!(" \u{b7} {address}"));
        }
    }
    let saved = group.people.iter().any(|p| p.is_saved());
    let mut line = format!(
        "Result \u{b7} {total} message{}",
        if total == 1 { "" } else { "s" }
    );
    if saved {
        line.push_str(" \u{b7} ");
        line.push_str(&crate::contacts_sync::synced_status(true, synced_with));
    }
    if unsaved > 0 {
        line.push_str(&format!(
            " \u{b7} the {} unsaved record{} {} folded in",
            count_word(unsaved),
            if unsaved == 1 { "" } else { "s" },
            if unsaved == 1 { "is" } else { "are" }
        ));
    }
    let name = keeper.shown_as().into_owned();
    MergeSheetView {
        title: format!("These {} look like the same person", number_word(group.people.len())),
        subtitle: "Merging keeps every address, so mail from any of them shows as one person. Nothing is deleted from your mail."
            .to_owned(),
        position: position.map(|(at, of)| format!("{} of {of}", at + 1)),
        cards,
        why_title: "Why we think so".to_owned(),
        evidence: group.evidence.clone(),
        result: MergeResult {
            avatar: avatar(keeper.id, &name, ContactKind::Person),
            role: role_line(keeper),
            name,
            addresses: shown,
            line,
        },
        foot: "Undo with \u{2318}Z. Merged records can be split again from the card.".to_owned(),
        not_same: button("Not the same", CommandId::SuggestionDismiss),
        later: button("Later", CommandId::Back),
        merge: button("Merge", CommandId::ContactShowMail),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use postio_model::{AddressId, ContactAddress, ContactId, EmailAddress};

    use super::*;

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 26, 16, 5, 0).unwrap()
    }

    fn keymap() -> Keymap {
        Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        )
    }

    fn person(id: i64, name: &str, address: &str, seen: u32, saved: bool) -> Contact {
        let mut person = Contact::new(ContactAddress::new(
            AddressId::new(id),
            EmailAddress::new(Some(name), address),
        ));
        person.id = ContactId::new(id);
        person.times_seen = seen;
        person.last_seen_at = Some(Utc.with_ymd_and_hms(2026, 9, 26, 12, 0, 0).unwrap());
        if saved {
            person.saved_at = Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
            person.title = Some("Finance lead".into());
            person.organization = Some("Northwind Partners".into());
        }
        person
    }

    fn group() -> MergeGroup {
        MergeGroup {
            people: vec![
                person(1, "Ada Moreno", "ada@example.org", 412, true),
                person(2, "A. Moreno", "a.moreno@example.net", 14, false),
                person(3, "Ada M.", "ada.moreno@example.com", 6, false),
            ],
            evidence: vec!["The same signature on all three".to_owned()],
        }
    }

    #[test]
    fn the_sheet_says_how_many_and_what_it_keeps() {
        let sheet = view(&group(), 0, Some((0, 2)), &keymap(), now(), None);
        assert_eq!(sheet.title, "These 3 look like the same person");
        assert!(sheet.subtitle.starts_with("Merging keeps every address"));
        assert_eq!(sheet.position.as_deref(), Some("1 of 2"));
        assert_eq!(sheet.why_title, "Why we think so");
        assert_eq!(sheet.evidence, ["The same signature on all three"]);
    }

    #[test]
    fn each_card_says_what_it_is_and_the_kept_one_has_the_ring() {
        let sheet = view(&group(), 0, None, &keymap(), now(), None);
        let cards: Vec<(&str, &str, bool, &str)> = sheet
            .cards
            .iter()
            .map(|c| (c.name.as_str(), c.source.as_str(), c.kept, c.keep.as_str()))
            .collect();
        assert_eq!(
            cards,
            [
                ("Ada Moreno", "saved", true, "Keep this name and photo"),
                ("A. Moreno", "not saved", false, "Address kept"),
                ("Ada M.", "not saved", false, "Address kept")
            ]
        );
        assert_eq!(sheet.cards[0].stats, "412 messages \u{b7} last today");
        assert_eq!(sheet.position, None);
    }

    #[test]
    fn choosing_another_record_moves_the_ring_and_the_name_of_the_result() {
        let sheet = view(&group(), 1, None, &keymap(), now(), None);
        assert!(sheet.cards[1].kept && !sheet.cards[0].kept);
        assert_eq!(sheet.result.name, "A. Moreno");
        assert!(
            sheet
                .result
                .addresses
                .starts_with("\u{2605} a.moreno@example.net")
        );
        assert_eq!(sheet.cards[0].keep, "Address kept");
    }

    #[test]
    fn the_result_adds_up_the_mail_and_says_what_is_folded_in() {
        let sheet = view(&group(), 0, None, &keymap(), now(), None);
        assert_eq!(sheet.result.name, "Ada Moreno");
        assert_eq!(sheet.result.role, "Finance lead, Northwind Partners");
        assert_eq!(
            sheet.result.addresses,
            "\u{2605} ada@example.org \u{b7} a.moreno@example.net \u{b7} ada.moreno@example.com"
        );
        assert_eq!(
            sheet.result.line,
            "Result \u{b7} 432 messages \u{b7} saved \u{b7} the two unsaved records are folded in"
        );
        assert_eq!(
            sheet.foot,
            "Undo with \u{2318}Z. Merged records can be split again from the card."
        );
    }

    #[test]
    fn the_three_ways_out_carry_the_keys_that_do_them() {
        let sheet = view(&group(), 0, None, &keymap(), now(), None);
        let keys: Vec<(&str, Option<&str>)> = [&sheet.not_same, &sheet.later, &sheet.merge]
            .iter()
            .map(|b| (b.label.as_str(), b.key.as_deref()))
            .collect();
        assert_eq!(
            keys,
            [
                ("Not the same", Some("n")),
                ("Later", Some("Escape")),
                ("Merge", Some("Return"))
            ]
        );
    }

    #[test]
    fn a_result_that_syncs_says_so_and_one_that_does_not_does_not() {
        let plain = view(&group(), 0, None, &keymap(), now(), None);
        assert!(
            plain.result.line.contains("\u{b7} saved \u{b7} the two"),
            "{}",
            plain.result.line
        );
        let synced = view(
            &group(),
            0,
            None,
            &keymap(),
            now(),
            Some("Contacts (iCloud)"),
        );
        assert!(
            synced
                .result
                .line
                .contains("saved, synced with Contacts (iCloud)"),
            "{}",
            synced.result.line
        );
    }
}
