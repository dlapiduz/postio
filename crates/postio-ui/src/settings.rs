//! The settings sections, their names, groups and where each starts in the file.
//! `header_key` reads brackets only and parses no TOML, so the nav stays live while the buffer is
//! mid-edit.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Accounts,
    /// Kept in the store, not `config.toml`; the terminal has no pane.
    Contacts,
    Filtering,
    Filters,
    Composing,
    Appearance,
    Keyboard,
    Sync,
    /// View state kept in its own `$XDG_STATE_HOME` key-file, not a `config.toml` table.
    Privacy,
    Agents,
    /// Not a fallback: `config.toml` is the settings store, and this reaches keys no form has a
    /// control for.
    ConfigFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Mail,
    Application,
}

impl Group {
    pub const ALL: [Group; 2] = [Group::Mail, Group::Application];

    /// Upper-cased here, not in CSS: GTK's `text-transform` does not apply reliably across faces.
    pub fn label(self) -> &'static str {
        match self {
            Group::Mail => "MAIL",
            Group::Application => "APPLICATION",
        }
    }
}

impl Section {
    pub const ALL: [Section; 11] = [
        Section::Accounts,
        Section::Contacts,
        Section::Filtering,
        Section::Filters,
        Section::Composing,
        Section::Appearance,
        Section::Keyboard,
        Section::Sync,
        Section::Privacy,
        Section::Agents,
        Section::ConfigFile,
    ];

    pub fn group(self) -> Group {
        match self {
            Section::Accounts
            | Section::Contacts
            | Section::Filtering
            | Section::Filters
            | Section::Composing => Group::Mail,
            Section::Appearance
            | Section::Keyboard
            | Section::Sync
            | Section::Privacy
            | Section::Agents
            | Section::ConfigFile => Group::Application,
        }
    }

    /// Matched by prefix: every account and filter is its own dotted table (`[accounts.personal]`).
    /// `Privacy` never appears.
    pub fn key(self) -> &'static str {
        match self {
            Section::Appearance => "ui",
            Section::Keyboard => "keys",
            Section::Accounts => "accounts",
            Section::Contacts => "contacts",
            Section::Filtering => "focus",
            Section::Sync => "sync",
            Section::Filters => "saved_searches",
            Section::Composing => "compose",
            Section::Privacy => "privacy",
            Section::Agents => "mcp",
            Section::ConfigFile => "",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Section::Accounts => "Accounts",
            Section::Contacts => crate::contacts_settings::TITLE,
            Section::Filtering => "Filtering",
            Section::Filters => "Saved searches",
            Section::Composing => "Composing",
            Section::Appearance => "Appearance",
            Section::Keyboard => "Keyboard",
            Section::Sync => "Sync & storage",
            Section::Privacy => "Privacy",
            Section::Agents => crate::agent_access::TITLE,
            Section::ConfigFile => "Config file",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Section::Accounts => "Every account this installation signs in to.",
            Section::Contacts => crate::contacts_settings::DESCRIPTION,
            Section::Filtering => "Spam and updates, archived as they arrive, with a reason.",
            Section::Filters => {
                "Saved searches, and which of them are pinned across the command bar."
            }
            Section::Composing => "Signatures, and where one goes when a quote sits under it.",
            Section::Appearance => "How the message list is drawn, and how much of it fits.",
            Section::Keyboard => "Every command and the key that runs it.",
            Section::Sync => "When mail syncs, and what the local store keeps.",
            Section::Privacy => "What Postio will not do without being asked.",
            Section::Agents => crate::agent_access::DESCRIPTION,
            Section::ConfigFile => "The whole file, as text. Everything above writes here.",
        }
    }

    /// Not generated from the controls: a search that silently stopped matching would be hard to
    /// notice.
    pub fn keywords(self) -> &'static str {
        match self {
            Section::Accounts => "account address imap smtp password oauth signature server remove",
            Section::Contacts => {
                "contacts people suggestions save automatically signature duplicates merge \
                 newsletter list automated kind address book sync"
            }
            Section::Filtering => {
                "filtering filtered spam promotion notification receipt shipping social \
                 automated updates archive never pinned restore sweep marker"
            }
            Section::Filters => "saved search query pinned command bar filter",
            Section::Composing => "signature reply forward quote compose",
            Section::Appearance => "theme dark light density row height avatars hover font",
            Section::Keyboard => "key binding shortcut rebind keys chord",
            Section::Sync => "sync idle poll interval storage index attachments notify size",
            Section::Privacy => "remote images trackers unsubscribe read receipts connections",
            Section::Agents => "agents ai mcp assistant model shared access revoke log activity",
            Section::ConfigFile => "toml file text editor raw",
        }
    }

    /// Focus honours none of Appearance's `[ui]` keys, so a control there would change nothing
    /// visible.
    pub fn shown_in(self, frontend: postio_core::Frontend) -> bool {
        use postio_core::Frontend;
        !matches!(
            (self, frontend),
            (Section::Appearance, Frontend::Focus)
                | (Section::Contacts, Frontend::Terminal)
                | (Section::Agents, Frontend::Terminal)
        )
    }

    /// `None` for panes owning no table: Accounts (encrypted store, #470), Privacy, and Config
    /// file.
    pub fn table(self) -> Option<&'static str> {
        match self {
            // Not `[accounts]`: #470 retired that table, and a footer naming it sends people to a
            // file that does not describe their account.
            Section::Accounts | Section::Contacts => None,
            Section::Filtering => Some("[focus]"),
            Section::Filters => Some("[saved_searches]"),
            Section::Composing => Some("[compose]"),
            Section::Appearance => Some("[ui]"),
            Section::Keyboard => Some("[keys]"),
            Section::Sync => Some("[sync]"),
            Section::Agents => Some("[mcp]"),
            Section::Privacy | Section::ConfigFile => None,
        }
    }

    /// Says the true thing for every pane, including the three with no table.
    pub fn stored_in(self) -> &'static str {
        match self {
            Section::Accounts => "accounts are in the encrypted store, not in config.toml",
            Section::Contacts => crate::contacts_settings::STORED_IN,
            Section::Filtering => "[focus] in config.toml · applied live",
            Section::Filters => "[saved_searches] in config.toml · applied live",
            Section::Composing => "[compose] in config.toml · applied live",
            Section::Appearance => "[ui] in config.toml · applied live",
            Section::Keyboard => "[keys] in config.toml · applied live",
            Section::Sync => "[sync] in config.toml · applied live",
            Section::Privacy => "kept outside config.toml, with the app's own state",
            Section::Agents => crate::agent_access::STORED_IN,
            Section::ConfigFile => "config.toml · applied live",
        }
    }
}

fn header_key(line: &str) -> Option<&str> {
    let line = line.trim();
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    if inner.starts_with('[') || inner.is_empty() {
        return None;
    }
    inner.split('.').next()
}

fn header_lines(text: &str) -> impl Iterator<Item = (usize, Section)> + '_ {
    text.lines().enumerate().filter_map(|(line, content)| {
        let key = header_key(content)?;
        Section::ALL
            .into_iter()
            .find(|section| section.key() == key)
            .map(|section| (line, section))
    })
}

pub fn find_section(text: &str, section: Section) -> Option<usize> {
    header_lines(text)
        .find(|(_, found)| *found == section)
        .map(|(line, _)| line)
}

/// `None` above the first header, or in a file with none.
pub fn section_at_line(text: &str, cursor_line: usize) -> Option<Section> {
    header_lines(text)
        .filter(|(line, _)| *line <= cursor_line)
        .last()
        .map(|(_, section)| section)
}

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

/// `Inbox` is excluded: RFC 3501 names it, and remapping it would disagree with every other client.
/// `Flagged` is a view, not a folder.
pub const MAPPABLE_ROLES: [(postio_model::mailbox::MailboxRole, &str); 5] = [
    (postio_model::mailbox::MailboxRole::Sent, "Sent"),
    (postio_model::mailbox::MailboxRole::Archive, "Archive"),
    (postio_model::mailbox::MailboxRole::Drafts, "Drafts"),
    (postio_model::mailbox::MailboxRole::Trash, "Trash"),
    (postio_model::mailbox::MailboxRole::Junk, "Junk"),
];

pub fn signature_heading(editing: bool, account: &str) -> String {
    let verb = if editing {
        "Edit signature"
    } else {
        "New signature"
    };
    format!("{verb} \u{b7} {account}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_signature_editor_names_what_it_makes_and_for_whom() {
        assert_eq!(
            super::signature_heading(false, "ada@example.com"),
            "New signature \u{b7} ada@example.com"
        );
        assert_eq!(
            super::signature_heading(true, "ada@example.com"),
            "Edit signature \u{b7} ada@example.com"
        );
    }

    use super::*;

    #[test]
    fn focus_shows_every_section_but_the_one_whose_keys_it_does_not_honour() {
        let focus: Vec<Section> = Section::ALL
            .into_iter()
            .filter(|section| section.shown_in(postio_core::Frontend::Focus))
            .collect();
        assert_eq!(
            focus,
            [
                Section::Accounts,
                Section::Contacts,
                Section::Filtering,
                Section::Filters,
                Section::Composing,
                Section::Keyboard,
                Section::Sync,
                Section::Privacy,
                Section::Agents,
                Section::ConfigFile,
            ]
        );
        assert!(
            Section::ALL
                .into_iter()
                .filter(|section| !matches!(section, Section::Contacts | Section::Agents))
                .all(|section| section.shown_in(postio_core::Frontend::Terminal)),
            "the terminal shows the other nine: filtering runs while it has the store"
        );
        assert!(
            !Section::Contacts.shown_in(postio_core::Frontend::Terminal),
            "Contacts is graphical: the registry offers a terminal none of its commands"
        );
    }

    #[test]
    fn agents_is_an_application_page_over_mcp_just_above_the_file() {
        assert_eq!(Section::Agents.label(), "Agents");
        assert_eq!(Section::Agents.group(), Group::Application);
        assert_eq!(Section::Agents.key(), "mcp");
        assert_eq!(Section::Agents.table(), Some("[mcp]"));
        let at = Section::ALL.iter().position(|s| *s == Section::Agents);
        assert_eq!(at, Some(Section::ALL.len() - 2), "the raw file stays last");
        for word in ["agent", "mcp", "revoke"] {
            assert!(
                Section::Agents.keywords().contains(word),
                "{word} finds Agents"
            );
        }
        assert!(!Section::Agents.shown_in(postio_core::Frontend::Terminal));
        let text = "[ui]\ntheme = \"dark\"\n\n[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"INBOX\"]\n";
        assert_eq!(find_section(text, Section::Agents), Some(3));
    }

    #[test]
    fn filtering_is_a_mail_page_over_the_focus_table() {
        assert_eq!(Section::Filtering.label(), "Filtering");
        assert_eq!(Section::Filtering.group(), Group::Mail);
        assert_eq!(Section::Filtering.table(), Some("[focus]"));
        let said = Section::Filtering.description();
        assert!(said.contains("reason"), "{said:?}");
        for word in ["spam", "promotion", "filtered", "never"] {
            assert!(
                Section::Filtering.keywords().contains(word),
                "{word} finds Filtering"
            );
        }
        let text = "[ui]\ndensity = \"compact\"\n\n[focus.filter]\nnever = []\n\n[focus]\nfiltering = false\n";
        assert_eq!(find_section(text, Section::Filtering), Some(3));
        assert_eq!(section_at_line(text, 7), Some(Section::Filtering));
    }

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

    #[test]
    fn an_interval_reads_as_the_unit_it_was_written_in() {
        assert_eq!(humanize_interval(300), "5 min");
        assert_eq!(humanize_interval(3600), "1 h");
        assert_eq!(humanize_interval(120), "2 min");
    }

    #[test]
    fn an_interval_that_is_not_a_round_minute_says_seconds_rather_than_rounding() {
        // The spin button is gone, so this line is the only one that states the interval; rounding
        // 90s to "1 min" would misstate the file.
        assert_eq!(humanize_interval(90), "90s");
        assert_eq!(humanize_interval(45), "45s");
    }

    #[test]
    fn every_pane_says_where_its_settings_actually_live() {
        for section in Section::ALL {
            let said = section.stored_in();
            assert!(!said.is_empty(), "{} says nothing", section.label());
            if let Some(table) = section.table() {
                assert!(
                    said.contains(table),
                    "{} owns {table} and does not say so: {said}",
                    section.label()
                );
            }
        }
        assert!(
            !Section::Accounts.stored_in().contains("[accounts]"),
            "accounts have not been in config.toml since #470"
        );
        assert!(Section::Accounts.stored_in().contains("encrypted store"));
    }

    #[test]
    fn every_section_that_owns_a_table_names_one_and_the_four_that_do_not_say_so() {
        for section in Section::ALL {
            match section {
                // Accounts and Contacts keep their state in the store, not in `config.toml` (#470).
                Section::Accounts | Section::Contacts | Section::Privacy | Section::ConfigFile => {
                    assert_eq!(
                        section.table(),
                        None,
                        "{} owns no config.toml table",
                        section.label()
                    )
                }
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
        assert_eq!(
            header_key("[saved_searches.urgent]"),
            Some("saved_searches")
        );
    }

    #[test]
    fn a_key_value_line_has_no_header() {
        assert_eq!(header_key("density = \"compact\""), None);
        assert_eq!(header_key(""), None);
    }

    #[test]
    fn an_array_of_tables_header_is_not_mistaken_for_a_section() {
        assert_eq!(header_key("[[items]]"), None);
    }

    #[test]
    fn find_section_locates_a_bare_header() {
        assert_eq!(find_section(SAMPLE, Section::Appearance), Some(1));
        assert_eq!(find_section(SAMPLE, Section::Sync), Some(14));
    }

    #[test]
    fn find_section_locates_the_first_dotted_table_for_a_prefix_section() {
        assert_eq!(find_section(SAMPLE, Section::Accounts), Some(8));
    }

    #[test]
    fn find_section_is_none_for_a_section_not_written_yet() {
        assert_eq!(find_section(SAMPLE, Section::Filters), None);
    }

    #[test]
    fn find_section_is_none_for_privacy_no_matter_what_the_file_says() {
        assert_eq!(find_section(SAMPLE, Section::Privacy), None);
    }

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
        assert_eq!(section_at_line(SAMPLE, 11), Some(Section::Accounts));
    }

    #[test]
    fn section_at_line_past_the_end_of_the_file_is_the_last_section() {
        assert_eq!(section_at_line(SAMPLE, 999), Some(Section::Sync));
    }
}

#[cfg(test)]
mod saved_search_words {
    use super::Section;

    #[test]
    fn the_saved_searches_page_names_no_sidebar_and_borrows_no_mailbox_name() {
        assert_eq!(Section::Filters.label(), "Saved searches");
        let said = Section::Filters.description();
        assert!(!said.contains("sidebar"), "{said:?}");
        assert!(said.contains("command bar"), "{said:?}");
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GnupgKeyring {
    Off,
    Read { keys: usize, skipped: u32 },
    Unreadable,
    Absent { home: String },
}

pub fn gnupg_line(state: &GnupgKeyring) -> String {
    match state {
        GnupgKeyring::Off => "Off: Postio reads no file outside its own store.".to_owned(),
        GnupgKeyring::Read { keys, skipped } => {
            let mut line = format!(
                "{keys} {} read from your GnuPG keyring",
                if *keys == 1 { "key" } else { "keys" }
            );
            if *skipped > 0 {
                line.push_str(&format!(" · {skipped} skipped"));
            }
            line
        }
        GnupgKeyring::Unreadable => "Postio can't read this GnuPG keyring. Export the keys you need with `gpg --export --armor <address> > keys.asc` and import the file.".to_owned(),
        GnupgKeyring::Absent { home } => format!("No GnuPG keyring was found in {home}."),
    }
}
