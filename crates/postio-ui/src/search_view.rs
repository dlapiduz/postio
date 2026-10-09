//! The words Focus's search dropdown and results view say (spec 010).
//!
//! The copy comes from the Mac design (`Design/focus-macos-search`, SPEC
//! section 2 and screens 01 and 03) and is composed here, once, so the Mac
//! and any later frontend say the same thing and a test can hold it.

use std::time::Duration;

use chrono::{DateTime, Datelike, TimeZone};
use postio_core::{CommandId, Keymap};

use crate::hints::{self, Hint};

/// The operators the empty dropdown lists under "Search by", each with the
/// hint after it (screen 01), in the design's order.
pub fn cheat_sheet() -> [(&'static str, &'static str); 8] {
    [
        ("from:", "person or address"),
        ("to:", "recipient"),
        ("subject:", "words in subject"),
        ("in:", "folder"),
        ("label:", "label name"),
        ("has:attachment", ""),
        ("after: before:", "dates"),
        ("-word", "exclude"),
    ]
}

/// When a recent search ran, as the right-hand column of its row says it:
/// "today", "yesterday", the weekday within the last week ("Mon"), then the
/// day and month ("21 Sep"), with the year for another year. Both times are
/// read in their own zone, so pass them in the person's.
pub fn recent_when<Tz: TimeZone>(at: DateTime<Tz>, now: DateTime<Tz>) -> String {
    let (ran, today) = (at.date_naive(), now.date_naive());
    let days = (today - ran).num_days();
    match days {
        ..=0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        2..=6 => ran.format("%a").to_string(),
        _ if ran.year() == today.year() => ran.format("%-d %b").to_string(),
        _ => ran.format("%-d %b %Y").to_string(),
    }
}

/// `n` with thousands separators: 18204 is "18,204".
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The count `capped` makes a floor of: "10,000+".
fn count(total: u64, capped: bool) -> String {
    format!("{}{}", grouped(total), if capped { "+" } else { "" })
}

/// The dropdown footer's right-hand side: "48 matches · 38 ms", or
/// "10,000+ matches · 41 ms" when the count stopped at the cap.
pub fn footer_count(total: u64, elapsed: Duration, capped: bool) -> String {
    let noun = if total == 1 && !capped {
        "match"
    } else {
        "matches"
    };
    let took = match elapsed.as_millis() {
        0 => "<1 ms".to_owned(),
        ms => format!("{ms} ms"),
    };
    format!("{} {noun} \u{b7} {took}", count(total, capped))
}

/// One "Narrow to" pill: the operator in monospace, the value, and how
/// many of the results it would keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NarrowPill {
    /// The operator with its colon: "from:", "has:", "label:".
    pub op: String,
    /// What it is set to: a person, "attachment", a label.
    pub value: String,
    /// The count, grouped: "9", "1,204".
    pub count: String,
}

impl NarrowPill {
    /// The pill as one line, for a screen reader and for tests.
    pub fn label(&self) -> String {
        format!("{} {} {}", self.op, self.value, self.count)
    }
}

/// The "Narrow to" pill for `op` `value`, which keeps `count` results.
pub fn narrow_pill(op: &str, value: &str, count: u64) -> NarrowPill {
    NarrowPill {
        op: op.to_owned(),
        value: value.to_owned(),
        count: grouped(count),
    }
}

/// The dropdown's last row: "Show all 48 results", or, while a short prefix
/// is being typed, "Show all 214 results for \u{201c}at\u{201d}". One result is
/// "Show 1 result"; a capped count is a floor ("10,000+").
pub fn show_all(total: u64, capped: bool, typed: Option<&str>) -> String {
    let for_typed = typed
        .map(|typed| format!(" for \u{201c}{typed}\u{201d}"))
        .unwrap_or_default();
    if total == 1 && !capped {
        return format!("Show 1 result{for_typed}");
    }
    format!("Show all {} results{for_typed}", count(total, capped))
}

/// The field's placeholder while it is empty (screen 01).
pub const PLACEHOLDER: &str = "Search mail, people and files, or type > for commands";
/// The empty dropdown's first section: the searches run lately.
pub const RECENT: &str = "Recent";
/// What the Recent section's note says after its key: "⌥⌫ forgets one".
pub const FORGETS_ONE: &str = "forgets one";
/// The pinned saved searches' section.
pub const SAVED_SEARCHES: &str = "Saved searches";
/// The cheat sheet's section.
pub const SEARCH_BY: &str = "Search by";
/// The cheat sheet's last line, before the example sentence.
pub const JUST_TYPE_IT: &str = "Or just type it:";
/// Between the example sentence and the query it becomes.
pub const BECOMES: &str = "becomes";
/// The plain English the cheat sheet lowers live, as screen 01 has it.
pub const EXAMPLE: &str = "invoices from ada last month";
/// The words state's hits.
pub const TOP_HITS: &str = "Top hits";
/// The Top hits section's note.
pub const TOP_HITS_NOTE: &str = "ranked by sender, recency and where the words matched";
/// The words state's filter pills.
pub const NARROW_TO: &str = "Narrow to";
/// What the Show all row says after its count.
pub const SHOW_ALL_DETAIL: &str = "in the main window, with filters, a timeline and Quick Look";

/// How many conversations a recent search found: "48 results".
pub fn results_count(n: u64) -> String {
    match n {
        1 => "1 result".to_owned(),
        n => format!("{} results", grouped(n)),
    }
}

/// A top hit's date column: the day and month ("26 Sep"), with the year
/// for another year. Both times in the person's zone.
pub fn hit_date<Tz: TimeZone>(at: DateTime<Tz>, now: DateTime<Tz>) -> String {
    let (day, today) = (at.date_naive(), now.date_naive());
    if day.year() == today.year() {
        day.format("%-d %b").to_string()
    } else {
        day.format("%-d %b %Y").to_string()
    }
}

/// A hit's folder column, as the operator that would find it there:
/// `in:Inbox`, quoted when the name has a space.
pub fn in_folder(name: &str) -> String {
    if name.contains(char::is_whitespace) {
        format!("in:\"{name}\"")
    } else {
        format!("in:{name}")
    }
}

/// The arrows, as one hint: the panel's own walk, which the toolkit keeps
/// (009 FR-004), not a command.
fn arrows(label: &str) -> Hint {
    hints::fixed(
        "Up Down",
        label,
        "the arrows move the dropdown's highlight, which the toolkit keeps",
    )
}

/// The empty dropdown's footer (screen 01): move, run a recent search
/// again, the saved searches' keys, and `>` for commands.
pub fn empty_hints(keymap: &Keymap) -> Vec<Hint> {
    let mut hints = vec![
        arrows("move"),
        hints::fixed(
            "Return",
            "run again",
            "Return runs the highlighted row: the field's own key, not a command",
        ),
    ];
    // ⌥1 to ⌥4 as one cap, when the four keys are one modifier and the digits.
    let saved = [
        CommandId::SavedSearch1,
        CommandId::SavedSearch2,
        CommandId::SavedSearch3,
        CommandId::SavedSearch4,
    ];
    let keys: Vec<Option<&str>> = saved.iter().map(|id| keymap.binding(*id)).collect();
    if let [Some(first), .., Some(last)] = keys.as_slice() {
        let run = first
            .strip_suffix('1')
            .filter(|modifier| last.strip_suffix('4') == Some(*modifier))
            .map(|modifier| format!("{modifier}1\u{2013}4"));
        hints.extend(match run {
            Some(key) => Some(Hint {
                key,
                label: "saved".to_owned(),
            }),
            None => hints::pair(keymap, saved[0], saved[3], "saved"),
        });
    }
    hints.push(hints::fixed(
        ">",
        "commands",
        "`>` typed first is the bar's commands-only prefix, not a key",
    ));
    hints
}

/// The words dropdown's footer (screen 03): move, open the highlighted
/// message, all results, and Tab for the first filter.
pub fn words_hints(keymap: &Keymap) -> Vec<Hint> {
    let mut hints = vec![
        arrows("move"),
        hints::fixed(
            "Return",
            "open message",
            "Return runs the highlighted row: the field's own key, not a command",
        ),
    ];
    hints.extend(hints::hint(
        keymap,
        CommandId::ShowAllResults,
        "all results",
    ));
    hints.push(hints::fixed(
        "Tab",
        "add first filter",
        "Tab in the field is the toolkit's key; the bar takes it only for a pill",
    ));
    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeDelta, Utc};

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 15, 30, 0).unwrap()
    }

    // The design's "Search by" grid (screen 01), in its order.
    #[test]
    fn the_cheat_sheet_is_the_designs_eight_entries() {
        assert_eq!(
            cheat_sheet(),
            [
                ("from:", "person or address"),
                ("to:", "recipient"),
                ("subject:", "words in subject"),
                ("in:", "folder"),
                ("label:", "label name"),
                ("has:attachment", ""),
                ("after: before:", "dates"),
                ("-word", "exclude"),
            ]
        );
    }

    #[test]
    fn a_recent_search_says_when_it_ran_the_way_the_design_does() {
        // Wednesday 30 September 2026.
        let now = at(2026, 9, 30);
        assert_eq!(recent_when(now, now), "today");
        assert_eq!(recent_when(now - TimeDelta::days(1), now), "yesterday");
        // Within the week: the weekday, three letters.
        assert_eq!(recent_when(at(2026, 9, 28), now), "Mon");
        assert_eq!(recent_when(now - TimeDelta::days(6), now), "Thu");
        // A week or more: the day and month.
        assert_eq!(recent_when(at(2026, 9, 21), now), "21 Sep");
        assert_eq!(recent_when(at(2026, 9, 23), now), "23 Sep");
        // Another year says so.
        assert_eq!(recent_when(at(2025, 12, 3), now), "3 Dec 2025");
    }

    #[test]
    fn a_recent_search_that_ran_in_the_future_is_today() {
        let now = at(2026, 9, 30);
        assert_eq!(recent_when(now + TimeDelta::hours(30), now), "today");
    }

    #[test]
    fn the_footer_counts_matches_and_says_how_long_it_took() {
        let took = |ms| Duration::from_millis(ms);
        assert_eq!(footer_count(48, took(38), false), "48 matches · 38 ms");
        assert_eq!(footer_count(1, took(4), false), "1 match · 4 ms");
        assert_eq!(footer_count(0, took(2), false), "0 matches · 2 ms");
        assert_eq!(footer_count(214, took(12), false), "214 matches · 12 ms");
        assert_eq!(
            footer_count(10_000, took(41), true),
            "10,000+ matches · 41 ms"
        );
        assert_eq!(
            footer_count(18_204, took(9), false),
            "18,204 matches · 9 ms"
        );
    }

    #[test]
    fn a_search_that_took_under_a_millisecond_says_so_without_a_zero() {
        assert_eq!(
            footer_count(3, Duration::from_micros(400), false),
            "3 matches · <1 ms"
        );
    }

    #[test]
    fn a_narrow_to_pill_says_its_operator_its_value_and_its_count() {
        let pill = narrow_pill("from:", "Tomás Reyes", 9);
        assert_eq!(
            (pill.op.as_str(), pill.value.as_str(), pill.count.as_str()),
            ("from:", "Tomás Reyes", "9")
        );
        assert_eq!(pill.label(), "from: Tomás Reyes 9");

        let attachment = narrow_pill("has:", "attachment", 12);
        assert_eq!(attachment.label(), "has: attachment 12");

        let big = narrow_pill("label:", "Atlas", 1_204);
        assert_eq!(big.count, "1,204");
    }

    #[test]
    fn a_recent_search_counts_its_results() {
        assert_eq!(results_count(48), "48 results");
        assert_eq!(results_count(1), "1 result");
        assert_eq!(results_count(0), "0 results");
        assert_eq!(results_count(18_204), "18,204 results");
    }

    #[test]
    fn a_top_hit_says_its_day_and_month_and_another_years() {
        let now = at(2026, 9, 26);
        assert_eq!(hit_date(now, now), "26 Sep");
        assert_eq!(hit_date(at(2026, 8, 14), now), "14 Aug");
        assert_eq!(hit_date(at(2025, 12, 3), now), "3 Dec 2025");
    }

    #[test]
    fn a_hits_folder_is_written_as_its_operator() {
        assert_eq!(in_folder("Inbox"), "in:Inbox");
        assert_eq!(in_folder("Q3 close"), "in:\"Q3 close\"");
    }

    #[test]
    fn the_empty_dropdowns_footer_names_its_keys_from_the_keymap() {
        let keymap = postio_core::Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        let hints = empty_hints(&keymap);
        let said: Vec<(&str, &str)> = hints
            .iter()
            .map(|hint| (hint.key.as_str(), hint.label.as_str()))
            .collect();
        assert_eq!(
            said,
            [
                ("Up Down", "move"),
                ("Return", "run again"),
                ("alt+1\u{2013}4", "saved"),
                (">", "commands"),
            ]
        );
    }

    #[test]
    fn the_words_dropdowns_footer_names_its_keys_from_the_keymap() {
        let keymap = postio_core::Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        let said: Vec<(String, String)> = words_hints(&keymap)
            .into_iter()
            .map(|hint| (hint.key, hint.label))
            .collect();
        let show_all = keymap
            .binding(postio_core::CommandId::ShowAllResults)
            .expect("bound on the Mac")
            .to_owned();
        assert_eq!(
            said,
            [
                ("Up Down".to_owned(), "move".to_owned()),
                ("Return".to_owned(), "open message".to_owned()),
                (show_all, "all results".to_owned()),
                ("Tab".to_owned(), "add first filter".to_owned()),
            ]
        );
    }

    #[test]
    fn the_show_all_row_names_how_many_results_it_opens() {
        assert_eq!(show_all(48, false, None), "Show all 48 results");
        assert_eq!(
            show_all(214, false, Some("at")),
            "Show all 214 results for \u{201c}at\u{201d}"
        );
        assert_eq!(show_all(1, false, None), "Show 1 result");
        assert_eq!(show_all(10_000, true, None), "Show all 10,000+ results");
    }
}
