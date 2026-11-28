//! What Focus's Filtered view says: tabs, reason pill, the note beside the tabs, the footer.

use chrono::NaiveDate;
use postio_core::{CommandId, Keymap};

use crate::focus_dialog::Verb;
use crate::hints::{self, Hint};

pub const TITLE: &str = "Filtered";

pub const SUBTITLE: &str = "Archived automatically \u{b7} newest first";

pub const NOTE: &str =
    "Nothing here is deleted automatically \u{b7} nothing here ever reached the inbox";

pub const RESTORE: &str = "Restore, never filter this sender";

/// Tabs `1` to `7`: each reason as the store spells it (`None` for All) and its name.
pub const TABS: [(Option<&str>, &str); 7] = [
    (None, "All"),
    (Some("spam"), "Spam"),
    (Some("promotion"), "Promotions"),
    (Some("notification"), "Notifications"),
    (Some("receipt"), "Receipts"),
    (Some("shipping"), "Shipping"),
    (Some("social"), "Social"),
];

pub const TAB_COMMANDS: [CommandId; 7] = [
    CommandId::FilteredTab1,
    CommandId::FilteredTab2,
    CommandId::FilteredTab3,
    CommandId::FilteredTab4,
    CommandId::FilteredTab5,
    CommandId::FilteredTab6,
    CommandId::FilteredTab7,
];

pub fn tab_counts(reasons: &[(String, u32)]) -> [u32; 7] {
    let mut counts = [0; 7];
    for (index, (reason, _)) in TABS.iter().enumerate() {
        counts[index] = match reason {
            None => reasons.iter().map(|(_, count)| *count).sum(),
            Some(reason) => reasons
                .iter()
                .find(|(name, _)| name == reason)
                .map_or(0, |(_, count)| *count),
        };
    }
    counts
}

pub fn today(count: u32) -> String {
    format!("{count} filtered today")
}

pub fn today_short(count: u32) -> String {
    format!("{count} today")
}

pub const SWEEP_BUTTON: &str = "Sweep the inbox\u{2026}";

pub const SWEEP_HEADING: &str = "Sweep the inbox?";

pub const SWEEP_NOTHING: &str = "Nothing in the inbox would be filtered";

pub fn sweep_body(count: u32, undo: Option<&str>) -> String {
    let messages = if count == 1 { "message" } else { "messages" };
    let mut said =
        format!("{count} {messages} in the inbox would move to Filtered, each with its reason.");
    if let Some(undo) = undo {
        said.push_str(&format!(" One {undo} puts them back."));
    }
    said
}

pub fn sweep_action(count: u32) -> String {
    format!("Move {count} to Filtered")
}

pub fn pill(reason: &str, source: Option<&str>) -> String {
    match source.map(str::trim).filter(|source| !source.is_empty()) {
        Some(source) => format!("{reason} \u{b7} {source}"),
        None => reason.to_owned(),
    }
}

pub fn footer(keymap: &Keymap, rows: bool) -> Vec<Hint> {
    let mut said = Vec::new();
    if rows {
        said.extend(hints::hint(
            keymap,
            CommandId::RestoreFiltered,
            "restore + never filter sender",
        ));
    }
    if let (Some(first), Some(last)) = (
        hints::key(keymap, TAB_COMMANDS[0]),
        hints::key(keymap, TAB_COMMANDS[6]),
    ) {
        said.push(Hint {
            key: format!("{first}\u{2013}{last}"),
            label: "reason tabs".to_owned(),
        });
    }
    if rows {
        said.extend(hints::hint(keymap, CommandId::OpenMessage, "open"));
    }
    said.extend(hints::hint(keymap, CommandId::GoToInbox, "inbox"));
    said
}

pub const EMPTY: &str = "Nothing is filtered here";

/// The selection bar's verbs over Filtered. Delete and Flag stay keys; Archive is absent since the
/// mail is archived already.
pub const BAR_VERBS: &[Verb] = &[
    Verb {
        command: CommandId::RestoreFiltered,
        label: "Restore",
        slug: "restore",
    },
    Verb {
        command: CommandId::ToggleRead,
        label: "Mark read",
        slug: "read",
    },
];

pub fn bar_hints(keymap: &Keymap) -> Vec<Hint> {
    let mut said = Vec::new();
    said.extend(hints::hint(keymap, CommandId::ToggleSelection, "toggle"));
    said.extend(hints::pair(
        keymap,
        CommandId::ExtendSelectionDown,
        CommandId::ExtendSelectionUp,
        "extend",
    ));
    said.extend(hints::hint(keymap, CommandId::Back, "clear"));
    said
}

pub const PAGE: u32 = 50;

pub fn page_is_full(read: usize) -> bool {
    read as u32 == PAGE
}

/// `Some(None)` for All, `None` for a tab that does not exist.
pub fn tab_reason(index: usize) -> Option<Option<&'static str>> {
    TABS.get(index).map(|(reason, _)| *reason)
}

pub fn tab_tooltip(name: &str, key: &str) -> String {
    format!("{name} ({key})")
}

/// `Some` for the first row of each local day ("Today · 9"), `None` for the rest; `days` is newest
/// first.
pub fn day_headings(days: &[NaiveDate], today: NaiveDate) -> Vec<Option<String>> {
    let mut previous = None;
    days.iter()
        .map(|day| {
            if previous == Some(*day) {
                return None;
            }
            previous = Some(*day);
            let count = days.iter().filter(|other| *other == day).count();
            Some(format!(
                "{} \u{b7} {count}",
                crate::focus_row::day_heading(*day, today)
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_the_sum_and_each_tab_its_reason() {
        let counts = tab_counts(&[
            ("notification".to_owned(), 88),
            ("spam".to_owned(), 12),
            ("promotion".to_owned(), 41),
        ]);
        assert_eq!(counts, [141, 12, 41, 88, 0, 0, 0]);
    }

    #[test]
    fn the_sweep_says_how_many_and_how_to_take_it_back() {
        assert_eq!(
            sweep_body(2, Some("ctrl+z")),
            "2 messages in the inbox would move to Filtered, each with its reason. \
             One ctrl+z puts them back."
        );
        assert_eq!(
            sweep_body(1, None),
            "1 message in the inbox would move to Filtered, each with its reason."
        );
    }

    #[test]
    fn a_pill_names_its_source_when_there_is_one() {
        assert_eq!(
            pill("notification", Some("Forge")),
            "notification \u{b7} Forge"
        );
        assert_eq!(pill("promotion", None), "promotion");
        assert_eq!(pill("promotion", Some(" ")), "promotion");
    }

    #[test]
    fn the_footer_names_the_keys_the_keymap_binds() {
        let said = hints::line(&footer(Keymap::defaults(), true));
        assert_eq!(
            said,
            "R restore + never filter sender \u{b7} 1\u{2013}7 reason tabs \u{b7} Return open \u{b7} g i inbox"
        );
    }

    #[test]
    fn a_page_is_full_at_fifty_and_tabs_name_their_reason() {
        assert!(page_is_full(50));
        assert!(!page_is_full(49));
        assert_eq!(tab_reason(0), Some(None));
        assert!(matches!(tab_reason(1), Some(Some(_))));
        assert_eq!(tab_reason(7), None);
        assert_eq!(tab_tooltip("Spam", "3"), "Spam (3)");
    }

    #[test]
    fn each_day_gets_one_heading_with_its_count() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
        let headings = day_headings(&[day(2), day(2), day(1)], day(2));
        assert_eq!(
            headings[0].as_deref(),
            Some("Today \u{b7} Friday 2 October \u{b7} 2")
        );
        assert_eq!(headings[1], None);
        assert!(headings[2].as_deref().unwrap().ends_with("\u{b7} 1"));
    }
}
