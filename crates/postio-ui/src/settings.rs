//! The settings sections: which they are, what they are called, which group
//! they sit under, and where each one starts in the file.
//!
//! Canvas 3f's contract is that `config.toml` *is* the settings UI -- one
//! store, no OK/Cancel, navigation that switches one pane rather than opening
//! a sub-screen. That contract is the same on both platforms, so the model of
//! it belongs here rather than in either frontend.
//!
//! Everything here is a pure function. [`header_key`] looks at a line's
//! brackets and parses no TOML at all, which is what lets the nav stay live
//! while the buffer is mid-edit and syntactically broken. Reading the file's
//! *values* is `postio_config`'s job, and writing them is its `patch_*`
//! functions'.
//!
//! What does **not** live here is anything a toolkit names: `postio-gtk`
//! keeps its own `icon` beside this, because a GTK symbolic icon name is not
//! an SF Symbol and neither frontend should carry the other's.

/// One of the eight sections the nav lists, in canvas order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// One row per account, and the form for the selected one.
    Accounts,
    /// `[filters]` — named saved queries.
    Filters,
    /// `[compose]` — signatures, and where one goes above a quote.
    Composing,
    /// `[ui]` — theme, row density, what the message list shows.
    Appearance,
    /// `[keys]` — command id to binding.
    Keyboard,
    /// `[sync]` — IDLE, polling, what is fetched and when.
    Sync,
    /// The remote-image allow-list (#871) and what has been unsubscribed
    /// from — never a `config.toml` table at all, unlike every other pane
    /// here: it is view state, kept in its own `$XDG_STATE_HOME` key-file
    /// (see `postio_gtk::reader::RemoteImageAllowList`, which owns the file;
    /// not linkable from here — this crate is below the frontends, not beside
    /// them).
    Privacy,
    /// The file itself, as text — the raw `TextView` every pane used to
    /// share. It is not a fallback: `config.toml` *is* the settings store,
    /// and a pane that shows it whole is how a person reaches a key no form
    /// has grown a control for yet.
    ConfigFile,
}

/// Which heading a pane sits under in the sidebar.
///
/// Two groups, because the drawing has two and because the split is real:
/// `Mail` is about the accounts and the messages in them, `Application` is
/// about this program. A person looking for "how big is my index" is not
/// looking under the same heading as one looking for "what is my address".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Accounts, Filters, Composing.
    Mail,
    /// Appearance, Keyboard, Sync & storage, Privacy, Config file.
    Application,
}

impl Group {
    /// Every group, in sidebar order.
    pub const ALL: [Group; 2] = [Group::Mail, Group::Application];

    /// The sidebar heading, already upper-cased.
    ///
    /// Upper here rather than in CSS: GTK's own `text-transform` does not
    /// apply reliably across every face this application loads, and a
    /// kicker that is capitalised in one place and not the next is the
    /// drift `widgets::kicker` exists to stop.
    pub fn label(self) -> &'static str {
        match self {
            Group::Mail => "MAIL",
            Group::Application => "APPLICATION",
        }
    }
}

impl Section {
    /// Every section, in nav order — the drawing's order, grouped.
    pub const ALL: [Section; 8] = [
        Section::Accounts,
        Section::Filters,
        Section::Composing,
        Section::Appearance,
        Section::Keyboard,
        Section::Sync,
        Section::Privacy,
        Section::ConfigFile,
    ];

    /// Which sidebar heading this pane sits under.
    pub fn group(self) -> Group {
        match self {
            Section::Accounts | Section::Filters | Section::Composing => Group::Mail,
            Section::Appearance
            | Section::Keyboard
            | Section::Sync
            | Section::Privacy
            | Section::ConfigFile => Group::Application,
        }
    }

    /// The top-level TOML key this section's headers start with.
    ///
    /// `[accounts]` and `[filters]` never appear as a bare header — every
    /// account and filter is its own dotted table, `[accounts.personal]` —
    /// so matching is by prefix, not by literal line. `Privacy` never
    /// appears at all, the same as `Accounts` since #470: the nav item
    /// stays and points at a structured widget instead of any text.
    pub fn key(self) -> &'static str {
        match self {
            Section::Appearance => "ui",
            Section::Keyboard => "keys",
            Section::Accounts => "accounts",
            Section::Sync => "sync",
            Section::Filters => "filters",
            Section::Composing => "compose",
            Section::Privacy => "privacy",
            // Not a table: the pane shows every table there is.
            Section::ConfigFile => "",
        }
    }

    /// The nav label, and the pane's own title.
    ///
    /// The pane repeats its sidebar name as its heading on purpose: with one
    /// pane on screen at a time, the title is the only thing that says which
    /// of the eight you are looking at without moving your eyes back to the
    /// sidebar.
    pub fn label(self) -> &'static str {
        match self {
            Section::Accounts => "Accounts",
            Section::Filters => "Filters",
            Section::Composing => "Composing",
            Section::Appearance => "Appearance",
            Section::Keyboard => "Keyboard",
            Section::Sync => "Sync & storage",
            Section::Privacy => "Privacy",
            Section::ConfigFile => "Config file",
        }
    }

    /// The one line under the pane's title, saying what it is for.
    pub fn description(self) -> &'static str {
        match self {
            Section::Accounts => "Every account this installation signs in to.",
            Section::Filters => "Saved searches, and which of them the sidebar shows.",
            Section::Composing => "Signatures, and where one goes when a quote sits under it.",
            Section::Appearance => "How the message list is drawn, and how much of it fits.",
            Section::Keyboard => "Every command and the key that runs it.",
            Section::Sync => "When mail is fetched, and what the local store keeps.",
            Section::Privacy => "What Postio will not do without being asked.",
            Section::ConfigFile => "The whole file, as text. Everything above writes here.",
        }
    }

    /// What this pane is about, for the find-a-setting field.
    ///
    /// The words a person would type looking for something on this pane,
    /// including the ones the pane's own title does not contain — somebody
    /// hunting for "dark mode" is looking for Appearance, which says
    /// neither word. Deliberately not generated from the controls: a pane
    /// gains and loses controls, and a search that silently stopped
    /// matching would be very hard to notice.
    pub fn keywords(self) -> &'static str {
        match self {
            Section::Accounts => "account address imap smtp password oauth signature server remove",
            Section::Filters => "saved search query pinned sidebar filter",
            Section::Composing => "signature reply forward quote compose",
            Section::Appearance => "theme dark light density row height avatars hover font",
            Section::Keyboard => "key binding shortcut rebind keys chord",
            Section::Sync => "sync idle poll interval storage index attachments notify size",
            Section::Privacy => "remote images trackers unsubscribe read receipts connections",
            Section::ConfigFile => "toml file text editor raw",
        }
    }

    /// The `config.toml` table this pane owns, for the footer line.
    ///
    /// `None` for the two panes that own no table: `Privacy` keeps its
    /// state outside the file entirely, and `Config file` is the file.
    pub fn table(self) -> Option<&'static str> {
        match self {
            Section::Accounts => Some("[accounts]"),
            Section::Filters => Some("[filters]"),
            Section::Composing => Some("[compose]"),
            Section::Appearance => Some("[ui]"),
            Section::Keyboard => Some("[keys]"),
            Section::Sync => Some("[sync]"),
            Section::Privacy | Section::ConfigFile => None,
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

/// `300` → `5 min`, `90` → `90s`, `3600` → `1 h`.
///
/// Pure, and tested as such: this is the sentence under the Check-for-mail
/// control, and it is the only thing on that pane that still says what the
/// interval in the file actually is once the spin button is gone.
pub fn humanize_interval(seconds: u64) -> String {
    match seconds {
        0 => "never".to_owned(),
        s if s % 3600 == 0 => {
            let hours = s / 3600;
            format!("{hours} h")
        }
        s if s % 60 == 0 => {
            let minutes = s / 60;
            format!("{minutes} min")
        }
        s => format!("{s}s"),
    }
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
    fn an_interval_reads_as_the_unit_it_was_written_in() {
        assert_eq!(humanize_interval(300), "5 min");
        assert_eq!(humanize_interval(3600), "1 h");
        assert_eq!(humanize_interval(120), "2 min");
    }

    #[test]
    fn an_interval_that_is_not_a_round_minute_says_seconds_rather_than_rounding() {
        // The spin button is gone, so this line is the only thing that can
        // tell somebody their interval is 90 seconds. Rounding it to
        // "1 min" here would make the pane lie about the file.
        assert_eq!(humanize_interval(90), "90s");
        assert_eq!(humanize_interval(45), "45s");
    }

    #[test]
    fn every_section_that_owns_a_table_names_one_and_the_two_that_do_not_say_so() {
        for section in Section::ALL {
            match section {
                Section::Privacy | Section::ConfigFile => assert_eq!(
                    section.table(),
                    None,
                    "{} owns no config.toml table",
                    section.label()
                ),
                other => {
                    let table = other.table().expect("a table");
                    assert!(
                        table.starts_with('[') && table.ends_with(']'),
                        "{} names its table the way the file writes it: {table}",
                        other.label()
                    );
                    assert!(
                        table.contains(other.key()),
                        "{}'s footer label and its header key must agree: \
                         {table} vs {}",
                        other.label(),
                        other.key()
                    );
                }
            }
        }
    }

    #[test]
    fn the_sidebar_groups_are_contiguous() {
        // The list draws a heading wherever the group changes, so a section
        // filed out of order would draw MAIL twice and read as two lists.
        let groups: Vec<Group> = Section::ALL.iter().map(|s| s.group()).collect();
        let mut seen = Vec::new();
        for group in groups {
            if seen.last() != Some(&group) {
                assert!(
                    !seen.contains(&group),
                    "{group:?} appears twice in nav order, so its heading would too"
                );
                seen.push(group);
            }
        }
        assert_eq!(seen, Group::ALL.to_vec());
    }

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
        assert_eq!(find_section(SAMPLE, Section::Appearance), Some(1));
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
        assert_eq!(section_at_line(SAMPLE, 2), Some(Section::Appearance));
        assert_eq!(section_at_line(SAMPLE, 3), Some(Section::Appearance));
        assert_eq!(section_at_line(SAMPLE, 6), Some(Section::Keyboard));
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
}
