//! What Focus's pickers say (spec US5; contracts/focus-surface.md,
//! "Pickers"): the snooze, remind, label and move popovers anchored to the
//! focused row. The widgets are `postio-widgets`'; the words, the times and
//! what a typed date means are here, where a test can hold them in a
//! second.

use chrono::{DateTime, Local};
use postio_core::{CommandId, Keymap};

/// The snooze picker's title (screen 11).
pub const SNOOZE_TITLE: &str = "Snooze until";
/// The remind picker's title (screen 12).
pub const REMIND_TITLE: &str = "Remind me if no one replies by";
/// The label picker's title (screen 13).
pub const LABEL_TITLE: &str = "Labels";
/// The move picker's title (screen 14).
pub const MOVE_TITLE: &str = "Move to folder";

/// What the typed-date field says while it is empty.
pub const DATE_PLACEHOLDER: &str = "Or type a date: \u{201c}tue 9am\u{201d}";

/// When a preset or a typed date lands, as its row says it: the time alone
/// for today ("18:00"), and the day with it otherwise ("Sun 27 Sep, 08:00").
pub fn when_label(at: DateTime<Local>, now: DateTime<Local>) -> String {
    if at.date_naive() == now.date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%a %-d %b, %H:%M").to_string()
    }
}

/// What a date typed into a picker means, read on this machine: `None` for
/// words that are not a date, and for a moment already past -- nothing can
/// be snoozed or reminded into the past.
pub fn typed(text: &str, now: DateTime<Local>) -> Option<DateTime<Local>> {
    postio_search::date::parse_when(text, now).filter(|at| *at > now)
}

/// What a picker acts on, as its title row names it: the sender and the
/// subject for one conversation, and how many for a selection.
pub fn target(conversations: usize, sender: &str, subject: &str) -> String {
    if conversations > 1 {
        format!("{conversations} conversations")
    } else {
        format!("{sender} \u{b7} {subject}")
    }
}

/// The snooze picker's footnote, with the key that goes to snoozed mail.
pub fn snooze_footnote(keymap: &Keymap) -> String {
    let mut said =
        "The message leaves the inbox and comes back at the top at that time.".to_owned();
    if let Some(key) = crate::hints::key(keymap, CommandId::GoToSnoozed) {
        said.push_str(&format!(" Snoozed mail is under {key}."));
    }
    said
}

/// The remind picker's footnote: what a reply does, and what the marker
/// will say, dated `now`.
pub fn remind_footnote(now: DateTime<Local>) -> String {
    format!(
        "If anyone replies first, the reminder is cancelled. If not, the \
         thread comes back to the top of the inbox marked \u{201c}No reply since \
         {}\u{201d}.",
        now.format("%a %-d %b")
    )
}

/// The label picker's filter placeholder.
pub const LABEL_FILTER: &str = "Filter, or type a new label";
/// The move picker's filter placeholder.
pub const MOVE_FILTER: &str = "Filter folders";
/// What a label row says when the target already carries it.
pub const APPLIED: &str = "\u{2713} applied";

/// The row that makes a label nobody has: `Create label “Receipts”`.
pub fn create_label(name: &str) -> String {
    format!("Create label \u{201c}{}\u{201d}", name.trim())
}

/// The label picker's footnote, with its keys.
pub fn label_footnote(keymap: &Keymap) -> String {
    let mut said = Vec::new();
    if let Some(key) = crate::hints::key(keymap, CommandId::PickerToggle) {
        said.push(format!("{key} toggles a label"));
    }
    if let Some(key) = crate::hints::key(keymap, CommandId::PickerConfirm) {
        said.push(format!("{key} closes"));
    }
    said.push(
        "typing a name that doesn\u{2019}t exist offers \u{201c}Create label\u{201d}.".to_owned(),
    );
    said.join(" \u{b7} ")
}

/// The move picker's footnote, with its keys.
pub fn move_footnote(keymap: &Keymap) -> String {
    let mut said = Vec::new();
    match crate::hints::key(keymap, CommandId::PickerConfirm) {
        Some(key) => said.push(format!("{key} moves the message and it leaves the inbox")),
        None => said.push("The message leaves the inbox".to_owned()),
    }
    if let Some(key) = crate::hints::key(keymap, CommandId::Undo) {
        said.push(format!("{key} undoes."));
    }
    said.join(" \u{b7} ")
}

/// Which of `names` a filter keeps, in order: every name containing
/// `filter`, in any case.
pub fn filtered<'a>(names: impl IntoIterator<Item = &'a str>, filter: &str) -> Vec<usize> {
    let wanted = filter.trim().to_lowercase();
    names
        .into_iter()
        .enumerate()
        .filter(|(_, name)| wanted.is_empty() || name.to_lowercase().contains(&wanted))
        .map(|(index, _)| index)
        .collect()
}

/// Whether `filter` names a label none of `names` is, in any case: when
/// the label picker offers to create it.
pub fn offers_create<'a>(names: impl IntoIterator<Item = &'a str>, filter: &str) -> bool {
    let wanted = filter.trim().to_lowercase();
    !wanted.is_empty() && !names.into_iter().any(|name| name.to_lowercase() == wanted)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn local_at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("an unambiguous local time")
    }

    fn saturday_afternoon() -> DateTime<Local> {
        local_at(2026, 9, 26, 16, 9)
    }

    #[test]
    fn a_time_today_is_the_time_and_another_day_names_the_day() {
        let now = saturday_afternoon();
        assert_eq!(when_label(local_at(2026, 9, 26, 18, 0), now), "18:00");
        assert_eq!(
            when_label(local_at(2026, 9, 27, 8, 0), now),
            "Sun 27 Sep, 08:00"
        );
        assert_eq!(
            when_label(local_at(2026, 10, 3, 8, 0), now),
            "Sat 3 Oct, 08:00"
        );
    }

    #[test]
    fn tue_9am_is_the_coming_tuesday_and_the_past_is_nothing() {
        // US5 scenario 2.
        let now = saturday_afternoon();
        assert_eq!(typed("tue 9am", now), Some(local_at(2026, 9, 29, 9, 0)));
        assert_eq!(typed("yesterday 9am", now), None, "the past");
        assert_eq!(typed("receipts", now), None, "not a date");
    }

    #[test]
    fn the_target_is_one_conversation_or_a_count() {
        assert_eq!(
            target(1, "Ada Moreno", "Atlas Q3 budget"),
            "Ada Moreno \u{b7} Atlas Q3 budget"
        );
        assert_eq!(
            target(3, "Ada Moreno", "Atlas Q3 budget"),
            "3 conversations"
        );
    }

    #[test]
    fn a_filter_keeps_what_contains_it_and_offers_what_nobody_has() {
        let names = ["Atlas", "Harbor", "Kitchen reno"];
        assert_eq!(filtered(names, ""), [0, 1, 2]);
        assert_eq!(filtered(names, "AR"), [1]);
        assert_eq!(filtered(names, "re"), [2]);
        assert!(!offers_create(names, ""), "nothing typed, nothing to make");
        assert!(!offers_create(names, "atlas"), "Atlas exists, in any case");
        assert!(offers_create(names, "Receipts"));
        assert_eq!(
            create_label(" Receipts "),
            "Create label \u{201c}Receipts\u{201d}"
        );
    }

    #[test]
    fn the_list_pickers_footnotes_name_their_keys() {
        assert_eq!(
            label_footnote(Keymap::defaults()),
            "space toggles a label \u{b7} Return closes \u{b7} typing a name that \
             doesn\u{2019}t exist offers \u{201c}Create label\u{201d}."
        );
        assert_eq!(
            move_footnote(Keymap::defaults()),
            "Return moves the message and it leaves the inbox \u{b7} ctrl+z undoes."
        );
    }

    #[test]
    fn the_footnotes_say_what_happens_next() {
        assert_eq!(
            snooze_footnote(Keymap::defaults()),
            "The message leaves the inbox and comes back at the top at that time. \
             Snoozed mail is under g z."
        );
        assert!(
            remind_footnote(saturday_afternoon())
                .ends_with("marked \u{201c}No reply since Sat 26 Sep\u{201d}."),
            "{}",
            remind_footnote(saturday_afternoon())
        );
    }
}
