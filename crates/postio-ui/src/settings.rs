//! The settings sections: which they are, what they are called, and where
//! each one starts in the file.
//!
//! Canvas 3f's contract is that `config.toml` *is* the settings UI — one
//! store, no OK/Cancel, navigation that jumps to a section rather than
//! opening a sub-screen. That contract is the same on both platforms, so the
//! model of it belongs here rather than in either frontend, and ADR 0029 is
//! where the reasoning lives.
//!
//! Everything in this module is a pure function over the file's text. It
//! parses no TOML — [`header_key`] looks at a line's brackets and nothing
//! else, which is what lets the nav stay live while the buffer is mid-edit
//! and syntactically broken. Reading the file's *values* is
//! `postio_config`'s job, and writing them is its `patch_*` functions'.
//!
//! # Two names for a section, deliberately
//!
//! [`Section::label`] is the bracketed table name (`[ui]`) and
//! [`Section::title`] is the human one ("Appearance"). GTK shows the first,
//! because its panel is a text view over the real file and a nav item that
//! did not match the header it scrolls to would be a lie. A structured pane
//! has no such text to agree with, so it shows the second.
//!
//! [`Section::Privacy`] has no bracketed name at all: its allow-list has
//! never been a `config.toml` table (#871), so [`Section::label`] gives the
//! human name there too rather than claiming a table that does not exist.

/// One of the six sections the nav lists, in canvas order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// `[ui]` — density, theme, hover actions, thread drill-in.
    Ui,
    /// `[keys]` — command id to binding.
    Keys,
    /// `[accounts]` — one table per account.
    Accounts,
    /// `[sync]` — IDLE, polling, connection budget.
    Sync,
    /// `[filters]` — named saved queries.
    Filters,
    /// The remote-image allow-list (#871) — never a `config.toml` table at
    /// all, unlike every other section here: it is view state, kept in its
    /// own `$XDG_STATE_HOME` key-file (see
    /// [`crate::reader::RemoteImageAllowList`]'s own module doc for why).
    Privacy,
}

impl Section {
    /// Every section, in nav order.
    pub const ALL: [Section; 6] = [
        Section::Ui,
        Section::Keys,
        Section::Accounts,
        Section::Sync,
        Section::Filters,
        Section::Privacy,
    ];

    /// The top-level TOML key this section's headers start with.
    ///
    /// `[accounts]` and `[filters]` never appear as a bare header — every
    /// account and filter is its own dotted table, `[accounts.personal]` —
    /// so matching is by prefix, not by literal line. `Privacy` never
    /// appears at all, the same as `Accounts` since #470: the nav item
    /// stays and points at a structured widget instead of any text.
    fn key(self) -> &'static str {
        match self {
            Section::Ui => "ui",
            Section::Keys => "keys",
            Section::Accounts => "accounts",
            Section::Sync => "sync",
            Section::Filters => "filters",
            Section::Privacy => "privacy",
        }
    }

    /// The nav label. `Privacy` deliberately drops the `[table]` bracket
    /// style the others use — see [`key`](Self::key): it would claim a
    /// `config.toml` table this section has never had.
    pub fn label(self) -> &'static str {
        match self {
            Section::Ui => "[ui]",
            Section::Keys => "[keys]",
            Section::Accounts => "[accounts]",
            Section::Sync => "[sync]",
            Section::Filters => "[filters]",
            Section::Privacy => "Privacy",
        }
    }

    /// The nav label a structured pane shows: the section's name in words.
    ///
    /// [`label`](Self::label) is the bracketed table name, which GTK's panel
    /// shows because it is a text view over the real file and a nav item has
    /// to agree with the header it scrolls to. A structured pane has no such
    /// text to agree with, and "[ui]" is a filename shown to someone who
    /// wanted to change the theme.
    ///
    /// `Privacy` answers the same either way — it owns no table for a
    /// bracketed name to refer to (#871).
    pub fn title(self) -> &'static str {
        match self {
            Section::Ui => "Appearance",
            Section::Keys => "Keyboard",
            Section::Accounts => "Accounts",
            Section::Sync => "Sync",
            Section::Filters => "Filters",
            Section::Privacy => "Privacy",
        }
    }
}

/// The first path segment of a `[...]` header line, if `line` is one.
///
/// `[accounts.personal.imap]` yields `Some("accounts")`; `density = "x"` and
/// `[[array_of_tables]]` (unused by this schema, guarded anyway) yield `None`.
fn header_key(line: &str) -> Option<&str> {
    let line = line.trim();
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    if inner.starts_with('[') || inner.is_empty() {
        return None;
    }
    inner.split('.').next()
}

/// Every section header in `text`, as a zero-based line number, top to
/// bottom.
fn header_lines(text: &str) -> impl Iterator<Item = (usize, Section)> + '_ {
    text.lines().enumerate().filter_map(|(line, content)| {
        let key = header_key(content)?;
        Section::ALL
            .into_iter()
            .find(|section| section.key() == key)
            .map(|section| (line, section))
    })
}

/// The first line `section` is written at, if it appears in `text` at all.
pub fn find_section(text: &str, section: Section) -> Option<usize> {
    header_lines(text)
        .find(|(_, found)| *found == section)
        .map(|(line, _)| line)
}

/// Which section a cursor on `cursor_line` sits inside — the nearest header
/// at or above it. `None` above the first header, or in a file with none.
pub fn section_at_line(text: &str, cursor_line: usize) -> Option<Section> {
    header_lines(text)
        .filter(|(line, _)| *line <= cursor_line)
        .last()
        .map(|(_, section)| section)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# edits here and in the panel are the same file
[ui]
density = \"compact\"
theme = \"system\"

[keys]
archive = \"a\"

[accounts.personal]
email = \"ada@example.com\"

[accounts.personal.imap]
host = \"imap.example.com\"

[sync]
idle = true
";

    // -- header parsing -----------------------------------------------------

    #[test]
    fn a_bare_section_header_names_its_key() {
        assert_eq!(header_key("[ui]"), Some("ui"));
        assert_eq!(header_key("  [sync]  "), Some("sync"));
    }

    #[test]
    fn a_dotted_header_is_matched_by_its_first_segment() {
        assert_eq!(header_key("[accounts.personal.imap]"), Some("accounts"));
        assert_eq!(header_key("[filters.urgent]"), Some("filters"));
    }

    #[test]
    fn a_key_value_line_has_no_header() {
        assert_eq!(header_key("density = \"compact\""), None);
        assert_eq!(header_key(""), None);
    }

    #[test]
    fn an_array_of_tables_header_is_not_mistaken_for_a_section() {
        // Unused by this schema today; guarded so a future one does not
        // silently misfile under the wrong section.
        assert_eq!(header_key("[[items]]"), None);
    }

    // -- find_section ---------------------------------------------------

    #[test]
    fn find_section_locates_a_bare_header() {
        assert_eq!(find_section(SAMPLE, Section::Ui), Some(1));
        assert_eq!(find_section(SAMPLE, Section::Sync), Some(14));
    }

    #[test]
    fn find_section_locates_the_first_dotted_table_for_a_prefix_section() {
        // `[accounts]` never appears bare; the first `[accounts.*]` table is
        // where a click on "[accounts]" has to land.
        assert_eq!(find_section(SAMPLE, Section::Accounts), Some(8));
    }

    #[test]
    fn find_section_is_none_for_a_section_not_written_yet() {
        assert_eq!(find_section(SAMPLE, Section::Filters), None);
    }

    #[test]
    fn find_section_is_none_for_privacy_no_matter_what_the_file_says() {
        // Privacy never has a `[privacy]` header to find, in any file --
        // it is not a table at all, unlike `Filters` above which simply
        // has not been written yet.
        assert_eq!(find_section(SAMPLE, Section::Privacy), None);
    }

    // -- section_at_line --------------------------------------------------

    #[test]
    fn section_at_line_is_none_above_the_first_header() {
        assert_eq!(section_at_line(SAMPLE, 0), None);
    }

    #[test]
    fn section_at_line_finds_the_nearest_header_at_or_above() {
        assert_eq!(section_at_line(SAMPLE, 2), Some(Section::Ui));
        assert_eq!(section_at_line(SAMPLE, 3), Some(Section::Ui));
        assert_eq!(section_at_line(SAMPLE, 6), Some(Section::Keys));
    }

    #[test]
    fn section_at_line_treats_every_line_of_a_dotted_table_as_its_section() {
        // Line 11, `host = "imap.example.com"`, sits under
        // `[accounts.personal.imap]`, which is still `accounts`.
        assert_eq!(section_at_line(SAMPLE, 11), Some(Section::Accounts));
    }

    #[test]
    fn section_at_line_past_the_end_of_the_file_is_the_last_section() {
        assert_eq!(section_at_line(SAMPLE, 999), Some(Section::Sync));
    }

    // -- the human names a structured pane shows (#1156) ---------------------

    #[test]
    fn every_section_has_a_human_title_distinct_from_every_other() {
        let titles: Vec<&str> = Section::ALL.iter().map(|s| s.title()).collect();
        let mut sorted = titles.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            titles.len(),
            "two sections share a title: {titles:?}"
        );
        assert!(
            titles.iter().all(|t| !t.starts_with('[')),
            "a title is the human name, not the table name: {titles:?}"
        );
    }

    #[test]
    fn appearance_is_the_human_name_for_the_ui_table() {
        // The pane a Mac user opens first (ADR 0029 Q3) must not be called
        // "[ui]" at them.
        assert_eq!(Section::Ui.title(), "Appearance");
        assert_eq!(Section::Ui.label(), "[ui]");
    }

    #[test]
    fn privacy_reads_the_same_either_way_because_it_owns_no_table() {
        // #871: the allow-list is not in `config.toml` at all, so there is no
        // bracketed name for `label` to be honest about.
        assert_eq!(Section::Privacy.label(), Section::Privacy.title());
    }
}
