//! What Focus's Filtered view says (spec 007 US9, screen 21): its tabs, a
//! row's reason pill, the line on the right of the tabs, and the footer.
//! The view is `postio-focus`'s; the words are here, where a test holds
//! them.

use postio_core::{CommandId, Keymap};

use crate::hints::{self, Hint};

/// The view's title.
pub const TITLE: &str = "Filtered";

/// The line under the title.
pub const SUBTITLE: &str = "Archived automatically \u{b7} newest first";

/// The line on the right of the tabs (spec C4: archived, never deleted).
pub const NOTE: &str =
    "Nothing here is deleted automatically \u{b7} nothing here ever reached the inbox";

/// What the focused row's restore button says.
pub const RESTORE: &str = "Restore, never filter this sender";

/// The tabs, in order, `1` to `7`: each reason as the store spells it --
/// `None` for All -- and its name.
pub const TABS: [(Option<&str>, &str); 7] = [
    (None, "All"),
    (Some("spam"), "Spam"),
    (Some("promotion"), "Promotions"),
    (Some("notification"), "Notifications"),
    (Some("receipt"), "Receipts"),
    (Some("shipping"), "Shipping"),
    (Some("social"), "Social"),
];

/// The number keys of the tabs, in order.
pub const TAB_COMMANDS: [CommandId; 7] = [
    CommandId::FilteredTab1,
    CommandId::FilteredTab2,
    CommandId::FilteredTab3,
    CommandId::FilteredTab4,
    CommandId::FilteredTab5,
    CommandId::FilteredTab6,
    CommandId::FilteredTab7,
];

/// Each tab's count, in [`TABS`]' order, from each reason's count as the
/// store gives them: All is their sum, and a reason the store did not name
/// has none.
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

/// The header strip's count: "186 filtered today".
pub fn today(count: u32) -> String {
    format!("{count} filtered today")
}

/// The folders popover's count beside Filtered: "186 today".
pub fn today_short(count: u32) -> String {
    format!("{count} today")
}

/// A row's reason pill: "notification · Forge", or the reason alone.
pub fn pill(reason: &str, source: Option<&str>) -> String {
    match source.map(str::trim).filter(|source| !source.is_empty()) {
        Some(source) => format!("{reason} \u{b7} {source}"),
        None => reason.to_owned(),
    }
}

/// The footer's hints, read from the keymap.
pub fn footer(keymap: &Keymap) -> Vec<Hint> {
    let mut said = Vec::new();
    said.extend(hints::hint(
        keymap,
        CommandId::RestoreFiltered,
        "restore + never filter sender",
    ));
    if let (Some(first), Some(last)) = (
        hints::key(keymap, TAB_COMMANDS[0]),
        hints::key(keymap, TAB_COMMANDS[6]),
    ) {
        said.push(Hint {
            key: format!("{first}\u{2013}{last}"),
            label: "reason tabs".to_owned(),
        });
    }
    said.push(hints::fixed(
        "Return",
        "open",
        "Return activates the focused row: the list's own, not a command of Filtered",
    ));
    said.extend(hints::hint(keymap, CommandId::Back, "inbox"));
    said
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
        let said = hints::line(&footer(Keymap::defaults()));
        assert_eq!(
            said,
            "R restore + never filter sender \u{b7} 1\u{2013}7 reason tabs \u{b7} Return open \u{b7} Escape inbox"
        );
    }
}
