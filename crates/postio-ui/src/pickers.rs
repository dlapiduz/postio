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

/// What the date field says under itself while nothing is typed.
pub const TYPE_A_DATE: &str = "Tab to type";

/// What the date field says under words that are no date still ahead.
pub const NOT_A_DATE: &str = "A day and a time: \u{201c}tue 9am\u{201d}";

/// What the date field says under `text`, typed at `now`: how to start,
/// when the words land ([`when_label`]), or what it wants instead. GTK's
/// when picker said these as literals; they are here so the Mac's says
/// the same.
pub fn date_hint(text: &str, now: DateTime<Local>) -> String {
    if text.trim().is_empty() {
        return TYPE_A_DATE.to_owned();
    }
    match typed(text, now) {
        Some(at) => when_label(at, now),
        None => NOT_A_DATE.to_owned(),
    }
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

/// The labels a picker shows as applied: those every one of the
/// conversations it acts on carries. `carried` is each `(thread, label)`
/// pair the conversations hold; with no conversations nothing is applied.
pub fn applied_labels(
    carried: impl IntoIterator<Item = (postio_model::ThreadId, postio_model::Label)>,
    threads: &[postio_model::ThreadId],
) -> std::collections::HashSet<postio_model::LabelId> {
    use std::collections::{HashMap, HashSet};
    let mut on: HashMap<postio_model::LabelId, HashSet<postio_model::ThreadId>> = HashMap::new();
    for (thread, label) in carried {
        on.entry(label.id).or_default().insert(thread);
    }
    let wanted: HashSet<postio_model::ThreadId> = threads.iter().copied().collect();
    on.into_iter()
        .filter(|(_, carrying)| !wanted.is_empty() && carrying.is_superset(&wanted))
        .map(|(label, _)| label)
        .collect()
}

/// One row of the label picker.
#[derive(Debug, Clone, PartialEq)]
pub enum LabelRow {
    /// "Create label “…”": the typed name, trimmed.
    Create(String),
    /// A label the account has.
    Label(postio_model::Label),
}

/// The label picker's rows for `filter`: "Create label" first when the
/// filter names none of the labels, then the labels it keeps.
pub fn label_rows(labels: &[postio_model::Label], filter: &str) -> Vec<LabelRow> {
    let names = labels.iter().map(|label| label.name.as_str());
    let mut rows = Vec::new();
    if offers_create(names.clone(), filter) {
        rows.push(LabelRow::Create(filter.trim().to_owned()));
    }
    rows.extend(
        filtered(names, filter)
            .into_iter()
            .map(|index| LabelRow::Label(labels[index].clone())),
    );
    rows
}

/// What a label's row says on the right: "applied" when the conversations
/// carry it, otherwise how many conversations do.
pub fn label_detail(applied: bool, count: u32) -> String {
    if applied {
        APPLIED.to_owned()
    } else {
        count.to_string()
    }
}

/// `labels` with `label` added, in name order, as the picker lists them
/// once a label has been made.
pub fn with_label(
    mut labels: Vec<postio_model::Label>,
    label: postio_model::Label,
) -> Vec<postio_model::Label> {
    labels.push(label);
    labels.sort_by_key(|label| label.name.to_lowercase());
    labels
}

/// How many of Recent get a number key and a row.
pub const RECENT: usize = 2;

/// The heading over the last destinations.
pub const RECENT_HEADING: &str = "Recent";

/// The heading over every folder.
pub const ALL_FOLDERS_HEADING: &str = "All folders";

/// Whether mail can be moved to `folder` from the picker: the person's own
/// folders and the archive. Sending, drafting, snoozing and deleting have
/// verbs of their own, and the inbox is where the mail already is.
pub fn is_destination(folder: &postio_model::Mailbox) -> bool {
    use postio_model::MailboxRole;
    matches!(folder.role, MailboxRole::Regular | MailboxRole::Archive)
}

/// The destinations in the order the picker keeps them: the archive first,
/// then by name.
pub fn order_destinations(folders: &mut [postio_model::Mailbox]) {
    folders.sort_by_key(|folder| {
        (
            folder.role != postio_model::MailboxRole::Archive,
            folder.name.to_lowercase(),
        )
    });
}

/// One row of the move picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveRow {
    /// The folder it moves to.
    pub folder: postio_model::MailboxId,
    /// The section heading it opens, when it opens one.
    pub section: Option<&'static str>,
    /// The folder's name.
    pub name: String,
    /// How many conversations it holds.
    pub count: String,
    /// Whether it has a number key: the Recent rows.
    pub numbered: bool,
}

/// The move picker's rows for `filter`: Recent first -- at most [`RECENT`]
/// of the last destinations still listed, numbered -- then every folder
/// Recent does not already list.
pub fn move_rows(
    folders: &[postio_model::Mailbox],
    recent: &[postio_model::MailboxId],
    filter: &str,
) -> Vec<MoveRow> {
    let keeps = |folder: &postio_model::Mailbox| {
        !filtered([crate::places::place_name(folder).as_str()], filter).is_empty()
    };
    let recent: Vec<&postio_model::Mailbox> = recent
        .iter()
        .filter_map(|id| folders.iter().find(|folder| folder.id == *id))
        .filter(|folder| keeps(folder))
        .take(RECENT)
        .collect();
    let mut rows = Vec::new();
    for (index, folder) in recent.iter().enumerate() {
        rows.push(MoveRow {
            folder: folder.id,
            section: (index == 0).then_some(RECENT_HEADING),
            name: crate::places::place_name(folder),
            count: folder.counts.total.to_string(),
            numbered: true,
        });
    }
    let mut first = true;
    for folder in folders
        .iter()
        .filter(|folder| keeps(folder) && !recent.iter().any(|kept| kept.id == folder.id))
    {
        rows.push(MoveRow {
            folder: folder.id,
            section: first.then_some(ALL_FOLDERS_HEADING),
            name: crate::places::place_name(folder),
            count: folder.counts.total.to_string(),
            numbered: false,
        });
        first = false;
    }
    rows
}

/// Whether a key pressed in a picker is typing rather than the picker's:
/// anything typed into a field is text -- except into an empty filter,
/// where there is nothing to type into yet and a bare digit or space is the
/// picker's (`1` chooses, `Space` toggles).
pub fn is_typing(in_field: bool, in_empty_filter: bool, bare: bool, digit_or_space: bool) -> bool {
    in_field && !(in_empty_filter && bare && digit_or_space)
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
        // Undo is `mod+z`, spelled for each platform: both asserted from
        // either host, so a Mac run is not a red Linux expectation.
        for (platform, undo) in [
            (postio_config::paths::Platform::Freedesktop, "ctrl+z"),
            (postio_config::paths::Platform::Apple, "cmd+z"),
        ] {
            assert_eq!(
                move_footnote(&Keymap::resolve_on(&Default::default(), platform)),
                format!("Return moves the message and it leaves the inbox \u{b7} {undo} undoes.")
            );
        }
    }

    #[test]
    fn the_date_field_says_how_to_start_when_the_words_land_or_what_it_wants() {
        let now = saturday_afternoon();
        assert_eq!(date_hint("", now), TYPE_A_DATE);
        assert_eq!(date_hint("  ", now), TYPE_A_DATE);
        assert_eq!(date_hint("tue 9am", now), "Tue 29 Sep, 09:00");
        assert_eq!(date_hint("receipts", now), NOT_A_DATE);
        assert_eq!(date_hint("yesterday 9am", now), NOT_A_DATE, "the past");
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

    fn label(id: i64, name: &str) -> postio_model::Label {
        let mut label = postio_model::Label::new(postio_model::AccountId::new(1), name);
        label.id = postio_model::LabelId::new(id);
        label
    }

    fn folder(id: i64, name: &str, role: postio_model::MailboxRole) -> postio_model::Mailbox {
        let mut folder = postio_model::Mailbox::new(postio_model::AccountId::new(1), name, None);
        folder.id = postio_model::MailboxId::new(id);
        folder.role = role;
        folder
    }

    #[test]
    fn a_label_is_applied_only_when_every_conversation_carries_it() {
        use postio_model::ThreadId;
        let threads = [ThreadId::new(1), ThreadId::new(2)];
        let carried = vec![
            (ThreadId::new(1), label(1, "a")),
            (ThreadId::new(2), label(1, "a")),
            (ThreadId::new(1), label(2, "b")),
        ];
        let applied = applied_labels(carried.clone(), &threads);
        assert!(applied.contains(&postio_model::LabelId::new(1)));
        assert!(!applied.contains(&postio_model::LabelId::new(2)));
        assert!(applied_labels(carried, &[]).is_empty());
    }

    #[test]
    fn a_name_nobody_has_is_offered_first_as_a_new_label() {
        let labels = vec![label(1, "Travel"), label(2, "Taxes")];
        assert_eq!(
            label_rows(&labels, " Ta"),
            vec![
                LabelRow::Create("Ta".into()),
                LabelRow::Label(labels[1].clone())
            ]
        );
        assert_eq!(label_rows(&labels, "travel").len(), 1);
        assert_eq!(label_detail(true, 4), APPLIED);
        assert_eq!(label_detail(false, 4), "4");
        let made = with_label(labels, label(3, "archive"));
        assert_eq!(made[0].name, "archive");
    }

    #[test]
    fn the_archive_is_offered_first_and_recent_is_not_listed_twice() {
        use postio_model::MailboxRole::{Archive, Inbox, Regular};
        let mut folders = vec![
            folder(1, "Receipts", Regular),
            folder(2, "Archive", Archive),
            folder(3, "Travel", Regular),
            folder(4, "INBOX", Inbox),
        ];
        folders.retain(is_destination);
        order_destinations(&mut folders);
        let rows = move_rows(&folders, &[postio_model::MailboxId::new(3)], "");
        let named: Vec<_> = rows
            .iter()
            .map(|row| (row.name.as_str(), row.section, row.numbered))
            .collect();
        assert_eq!(
            named,
            vec![
                ("Travel", Some(RECENT_HEADING), true),
                ("Archive", Some(ALL_FOLDERS_HEADING), false),
                ("Receipts", None, false),
            ]
        );
        assert_eq!(move_rows(&folders, &[], "rec").len(), 1);
    }

    #[test]
    fn only_a_bare_digit_or_space_in_an_empty_filter_is_not_typing() {
        assert!(is_typing(true, false, true, true));
        assert!(!is_typing(true, true, true, true));
        assert!(is_typing(true, true, false, true));
        assert!(!is_typing(false, false, true, false));
    }
}
