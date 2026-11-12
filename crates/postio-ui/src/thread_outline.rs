//! The thread menu and the outline (spec 013 US5, FR-019).
//!
//! Thread ▾ opens a menu in two groups, and `alt+t` an outline of every
//! message. Both are composed here, from the live keymap, so the Mac and GTK
//! say the same words and show the keys the person has.

use chrono::{DateTime, Local};
use postio_core::{CommandId, Keymap};
use postio_model::MessageId;
use postio_model::thread_view::ThreadView;

use crate::hints;
use crate::thread_list::ChildRowView;

/// One message of the outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineRow {
    /// Its place among the thread's members, from 0.
    pub index: usize,
    /// Its number as the person counts: `index + 1`.
    pub number: u32,
    /// The message.
    pub message: MessageId,
    /// The avatar's letters.
    pub initials: String,
    /// Who wrote it: "You" for the account's own.
    pub name: String,
    /// Drawn bold: not read yet.
    pub bold: bool,
    /// Its first line.
    pub line: String,
    /// When, as every row says it.
    pub date: String,
    /// The unread dot.
    pub unread: bool,
    /// The message on screen: ringed.
    pub current: bool,
}

/// Everything the outline popover draws.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutlineView {
    /// The rows the filter leaves, in thread order.
    pub rows: Vec<OutlineRow>,
    /// Person chips with their counts, over the whole thread.
    pub people: Vec<(String, u32)>,
    /// The field's text.
    pub filter: String,
    /// The chip chosen, if one is.
    pub person: Option<String>,
    /// The row the keyboard is on, among `rows`.
    pub cursor: usize,
    /// The footer's `u` keycap, as the keymap has it.
    pub first_unread_key: Option<String>,
}

/// Every message of `view` as an outline row, `current` the one on screen.
pub fn rows_of(
    view: &ThreadView,
    current: usize,
    me: &[postio_model::EmailAddress],
    keymap: &Keymap,
    now: DateTime<Local>,
) -> Vec<OutlineRow> {
    (0..view.members.len())
        .filter_map(|index| {
            let child = ChildRowView::member(view, index, me, keymap, now)?;
            Some(OutlineRow {
                index,
                number: index as u32 + 1,
                message: view.members[index].id,
                initials: child.initials,
                name: child.sender,
                bold: child.bold,
                line: child.line,
                date: child.time,
                unread: child.bold,
                current: index == current,
            })
        })
        .collect()
}

/// The rows whose name or first line holds `text` (case and accents
/// ignored) and, when `person` is given, that person's.
pub fn filter_rows(rows: &[OutlineRow], text: &str, person: Option<&str>) -> Vec<OutlineRow> {
    let needle = postio_model::fold::fold(text.trim());
    rows.iter()
        .filter(|row| person.is_none_or(|person| row.name == person))
        .filter(|row| {
            needle.is_empty()
                || postio_model::fold::fold(&row.name).contains(&needle)
                || postio_model::fold::fold(&row.line).contains(&needle)
        })
        .cloned()
        .collect()
}

/// The names in `rows` with how many messages each wrote, in order of first
/// appearance.
pub fn people(rows: &[OutlineRow]) -> Vec<(String, u32)> {
    let mut people: Vec<(String, u32)> = Vec::new();
    for row in rows {
        match people.iter_mut().find(|(name, _)| *name == row.name) {
            Some((_, count)) => *count += 1,
            None => people.push((row.name.clone(), 1)),
        }
    }
    people
}

/// The position, among `rows`, of the first unread message.
pub fn first_unread(rows: &[OutlineRow]) -> Option<usize> {
    rows.iter().position(|row| row.unread)
}

/// The outline over `all` the rows, narrowed by `filter` and `person`.
pub fn outline(
    all: &[OutlineRow],
    filter: &str,
    person: Option<&str>,
    cursor: usize,
    keymap: &Keymap,
) -> OutlineView {
    let rows = filter_rows(all, filter, person);
    OutlineView {
        cursor: cursor.min(rows.len().saturating_sub(1)),
        rows,
        people: people(all),
        filter: filter.to_owned(),
        person: person.map(str::to_owned),
        first_unread_key: hints::key(keymap, CommandId::OutlineFirstUnread),
    }
}

/// What a muted thread's row says: "muted", and the replies since.
pub fn muted_badge(count: u32) -> String {
    match count {
        0 => "muted".to_owned(),
        count => format!("muted \u{b7} {count}"),
    }
}

/// One entry of the thread menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    /// The command it runs.
    pub command: CommandId,
    /// What it says.
    pub title: String,
    /// The key, from the keymap: "alt+t", "f, 2".
    pub keycap: Option<String>,
    /// One line under it.
    pub note: Option<String>,
    /// Whether it can be chosen now.
    pub enabled: bool,
}

/// A group of entries under a heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuGroup {
    /// "Whole thread" or "This message only".
    pub title: String,
    /// Its entries, top to bottom.
    pub entries: Vec<MenuEntry>,
}

/// The thread menu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThreadMenuView {
    /// Two groups.
    pub groups: Vec<MenuGroup>,
}

/// The menu for a thread that is `muted` or not, with `can_split` false when
/// the message shown is the thread's first.
pub fn menu(keymap: &Keymap, muted: bool, can_split: bool) -> ThreadMenuView {
    let entry = |command, title: &str, note: Option<&str>| MenuEntry {
        command,
        title: title.to_owned(),
        keycap: hints::key(keymap, command),
        note: note.map(str::to_owned),
        enabled: true,
    };
    let mut forward = entry(
        CommandId::Forward,
        "Forward the whole thread",
        Some("As one message, or as attachments"),
    );
    // `f` then `2` on the chooser: both keys are the keymap's.
    forward.keycap = match (
        hints::key(keymap, CommandId::Forward),
        hints::key(keymap, CommandId::ForwardWhat2),
    ) {
        (Some(f), Some(two)) => Some(format!("{f}, {two}")),
        (one, other) => one.or(other),
    };
    let mut mute = entry(
        CommandId::MuteThread,
        "Mute thread",
        Some("New replies skip the inbox; still searchable"),
    );
    if muted {
        mute.title = "Unmute thread".to_owned();
        mute.note = Some("New replies reach the inbox again".to_owned());
    }
    let mut split = entry(
        CommandId::SplitThread,
        "Split into its own thread",
        Some("When an unrelated mail was grouped here"),
    );
    if !can_split {
        split.enabled = false;
        split.note = Some("The first message starts the thread; nothing to split".to_owned());
    }
    ThreadMenuView {
        groups: vec![
            MenuGroup {
                title: "Whole thread".to_owned(),
                entries: vec![
                    entry(
                        CommandId::ThreadOutline,
                        "Outline and filter",
                        Some("Every message, who wrote it, what's unread"),
                    ),
                    forward,
                    entry(
                        CommandId::ReplyToLatest,
                        "Reply to the latest message",
                        None,
                    ),
                    mute,
                ],
            },
            MenuGroup {
                title: "This message only".to_owned(),
                entries: vec![
                    split,
                    entry(CommandId::MoveThisMessage, "Move just this message", None),
                    entry(
                        CommandId::DeleteThisMessage,
                        "Delete just this message",
                        None,
                    ),
                    entry(
                        CommandId::SwitchTreatment,
                        "Always show its HTML as sent",
                        None,
                    ),
                ],
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use postio_model::thread_view::{Line, ThreadMember};
    use postio_model::{EmailAddress, ThreadId};

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap()
    }

    fn member(id: i64, name: &str, line: &str, seen: bool) -> ThreadMember {
        ThreadMember {
            id: MessageId::new(id),
            from: Some(EmailAddress::new(
                Some(name),
                format!("{}@example.com", name.to_lowercase().replace(' ', ".")),
            )),
            own: false,
            at: Utc.with_ymd_and_hms(2026, 9, 21, 9, 14, 0).unwrap(),
            seen,
            has_attachments: false,
            line: Line::Own(line.into()),
            answers: None,
            marker: false,
        }
    }

    fn harbor() -> ThreadView {
        ThreadView {
            thread: ThreadId::new(1),
            subject: Some("Harbor".into()),
            members: vec![
                member(1, "Ana Ruiz", "Plan for the export", true),
                member(2, "Ben Adeyemi", "Rate limit headers look right", true),
                member(3, "Ana Ruiz", "Thanks, merging", false),
                member(4, "Ben Adeyemi", "One more thing on José's draft", false),
            ],
            draft: None,
            muted: None,
            labels: Vec::new(),
        }
    }

    fn rows() -> Vec<OutlineRow> {
        rows_of(&harbor(), 1, &[], Keymap::defaults(), now())
    }

    #[test]
    fn a_row_says_number_initials_name_line_date_and_unread() {
        let rows = rows();
        assert_eq!(rows.len(), 4);
        let third = &rows[2];
        assert_eq!(third.number, 3);
        assert_eq!(third.initials, "AR");
        assert_eq!(third.name, "Ana Ruiz");
        assert!(third.bold && third.unread);
        assert_eq!(third.line, "Thanks, merging");
        assert!(!rows[0].bold && !rows[0].unread);
        assert!(rows[1].current && !rows[0].current);
    }

    #[test]
    fn the_filter_matches_name_and_line_ignoring_case_and_accents() {
        let rows = rows();
        let ben = filter_rows(&rows, "BEN", None);
        assert_eq!(ben.iter().map(|r| r.number).collect::<Vec<_>>(), [2, 4]);
        let line = filter_rows(&rows, "rate limit", None);
        assert_eq!(line.iter().map(|r| r.number).collect::<Vec<_>>(), [2]);
        let accent = filter_rows(&rows, "jose", None);
        assert_eq!(accent.iter().map(|r| r.number).collect::<Vec<_>>(), [4]);
        assert_eq!(filter_rows(&rows, "", None).len(), 4);
    }

    #[test]
    fn a_person_chip_narrows_to_that_person_and_combines_with_the_text() {
        let rows = rows();
        let ana = filter_rows(&rows, "", Some("Ana Ruiz"));
        assert_eq!(ana.iter().map(|r| r.number).collect::<Vec<_>>(), [1, 3]);
        let both = filter_rows(&rows, "merging", Some("Ana Ruiz"));
        assert_eq!(both.iter().map(|r| r.number).collect::<Vec<_>>(), [3]);
        assert!(filter_rows(&rows, "merging", Some("Ben Adeyemi")).is_empty());
    }

    #[test]
    fn the_chips_count_each_person_over_the_whole_thread() {
        assert_eq!(
            people(&rows()),
            [("Ana Ruiz".to_owned(), 2), ("Ben Adeyemi".to_owned(), 2)]
        );
    }

    #[test]
    fn u_goes_to_the_first_unread_among_the_rows_shown() {
        let rows = rows();
        assert_eq!(first_unread(&rows), Some(2));
        let ben = filter_rows(&rows, "", Some("Ben Adeyemi"));
        assert_eq!(first_unread(&ben), Some(1), "its place among those shown");
        assert_eq!(first_unread(&filter_rows(&rows, "plan", None)), None);
    }

    #[test]
    fn the_view_keeps_every_chip_while_the_rows_narrow() {
        let view = outline(&rows(), "plan", None, 0, Keymap::defaults());
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.people.len(), 2, "chips count the whole thread");
        assert_eq!(view.filter, "plan");
        assert_eq!(view.first_unread_key.as_deref(), Some("u"));
    }

    #[test]
    fn the_menu_has_two_groups_with_the_keymaps_keys() {
        let keymap = Keymap::defaults();
        // The one key that differs by platform: the keymap's own.
        let delete_this = keymap
            .binding(CommandId::DeleteThisMessage)
            .map(str::to_owned);
        let menu = menu(keymap, false, true);
        let groups: Vec<_> = menu.groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(groups, ["Whole thread", "This message only"]);
        let titles = |at: usize| {
            menu.groups[at]
                .entries
                .iter()
                .map(|e| (e.title.as_str(), e.keycap.as_deref()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            titles(0),
            [
                ("Outline and filter", Some("alt+t")),
                ("Forward the whole thread", Some("f, 2")),
                ("Reply to the latest message", Some("alt+e")),
                ("Mute thread", Some("m")),
            ]
        );
        assert_eq!(
            titles(1),
            [
                ("Split into its own thread", Some("alt+shift+s")),
                ("Move just this message", Some("alt+v")),
                ("Delete just this message", delete_this.as_deref()),
                ("Always show its HTML as sent", Some("O")),
            ]
        );
    }

    #[test]
    fn a_muted_thread_offers_unmute_and_a_first_message_cannot_split() {
        let menu = menu(Keymap::defaults(), true, false);
        let mute = &menu.groups[0].entries[3];
        assert_eq!(mute.title, "Unmute thread");
        let split = &menu.groups[1].entries[0];
        assert!(!split.enabled);
        assert!(split.note.is_some(), "it says why");
        assert!(menu.groups[1].entries[1].enabled);
    }

    #[test]
    fn a_rebound_key_shows_in_the_menu() {
        let mut overrides = postio_config::KeyBindings::default();
        overrides.overrides_mut().insert(
            CommandId::MuteThread.as_str().to_owned(),
            "ctrl+m".to_owned(),
        );
        let keymap = Keymap::resolve(&overrides);
        let menu = menu(&keymap, false, true);
        assert_eq!(menu.groups[0].entries[3].keycap.as_deref(), Some("ctrl+m"));
    }

    #[test]
    fn a_muted_row_reads_muted_and_the_count() {
        assert_eq!(muted_badge(1), "muted \u{b7} 1");
        assert_eq!(muted_badge(0), "muted");
    }
}
