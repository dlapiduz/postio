//! An expanded thread as a splice over the windowed rows: the paged query never holds children, so
//! cursor, scope and footer ask the splice (spec 013 R7).

use chrono::{DateTime, Local};
use postio_core::{CommandId, Keymap};
use postio_model::EmailAddress;
use postio_model::thread_view::{Line, ThreadView};

use crate::{focus_row, hints, row};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Row(u32),
    Child { row: u32, index: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Splice {
    pub anchor: u32,
    pub children: u32,
}

impl Splice {
    /// A position past the end is the row it would be after the splice, so an anchor beyond the
    /// landed rows is harmless.
    pub fn slot(self, display: u32) -> Slot {
        let first = self.anchor.saturating_add(1);
        if display < first {
            Slot::Row(display)
        } else if display - first < self.children {
            Slot::Child {
                row: self.anchor,
                index: display - first,
            }
        } else {
            Slot::Row(display - self.children)
        }
    }

    pub fn display(self, slot: Slot) -> u32 {
        match slot {
            Slot::Row(row) if row <= self.anchor => row,
            Slot::Row(row) => row + self.children,
            Slot::Child { index, .. } => {
                self.anchor + 1 + index.min(self.children.saturating_sub(1))
            }
        }
    }

    pub fn len(self, rows: u32) -> u32 {
        if self.anchor < rows {
            rows + self.children
        } else {
            rows
        }
    }
}

pub fn slot(splice: Option<Splice>, display: u32) -> Slot {
    splice.map_or(Slot::Row(display), |splice| splice.slot(display))
}

pub fn display(splice: Option<Splice>, slot: Slot) -> u32 {
    match (splice, slot) {
        (Some(splice), slot) => splice.display(slot),
        (None, Slot::Row(row)) => row,
        (None, Slot::Child { row, index }) => row + 1 + index,
    }
}

pub fn len(splice: Option<Splice>, rows: u32) -> u32 {
    splice.map_or(rows, |splice| splice.len(rows))
}

/// Thread-row verbs act on the latest message (D2); a child narrows them to itself, and this is how
/// the row says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildHint {
    /// A command with no key is left out rather than drawn as a blank cap.
    pub keys: Vec<String>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildRowView {
    pub initials: String,
    pub sender: String,
    pub bold: bool,
    pub tag: Option<String>,
    pub line: String,
    pub placeholder: bool,
    pub attachment: bool,
    pub time: String,
    pub draft: bool,
    pub focused_hint: Option<ChildHint>,
}

pub const NO_PREVIEW: &str = "No preview";
pub const QUOTED_ONLY: &str = "Quoted reply only";

impl ChildRowView {
    pub fn member(
        view: &ThreadView,
        index: usize,
        me: &[EmailAddress],
        keymap: &Keymap,
        now: DateTime<Local>,
    ) -> Option<Self> {
        let member = view.members.get(index)?;
        let (line, placeholder) = match &member.line {
            Line::Own(text) if !text.trim().is_empty() => (text.clone(), false),
            Line::Quoted => (QUOTED_ONLY.to_owned(), true),
            Line::Own(_) | Line::Preview(_) => match &member.line {
                Line::Preview(text) if !text.trim().is_empty() => (text.clone(), false),
                _ => (NO_PREVIEW.to_owned(), true),
            },
        };
        let own = member.own
            || member
                .from
                .as_ref()
                .is_some_and(|f| me.iter().any(|m| m.same_address(f)));
        Some(Self {
            initials: row::initials(member.from.as_ref()),
            sender: match (&member.from, own) {
                (_, true) => "You".to_owned(),
                (Some(from), false) => from.display().to_owned(),
                (None, false) => "Unknown sender".to_owned(),
            },
            bold: !member.seen,
            tag: focus_row::child_tag(member.answers, &view.members),
            line,
            placeholder,
            attachment: member.has_attachments,
            time: row::timestamp(member.at, now),
            draft: false,
            focused_hint: Some(focused_hint(keymap)),
        })
    }

    pub fn draft(view: &ThreadView, now: DateTime<Local>) -> Option<Self> {
        let draft = view.draft.as_ref()?;
        let (line, placeholder) = if draft.first_words.trim().is_empty() {
            (NO_PREVIEW.to_owned(), true)
        } else {
            (draft.first_words.clone(), false)
        };
        Some(Self {
            initials: "\u{270e}".to_owned(),
            sender: "Draft \u{b7} you".to_owned(),
            bold: false,
            tag: Some("not sent".to_owned()),
            line,
            placeholder,
            attachment: false,
            time: row::timestamp(draft.updated_at, now),
            draft: true,
            focused_hint: None,
        })
    }
}

pub fn focused_hint(keymap: &Keymap) -> ChildHint {
    ChildHint {
        keys: [CommandId::Reply, CommandId::Forward]
            .into_iter()
            .filter_map(|command| hints::key(keymap, command))
            .collect(),
        label: "this one".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(anchor: u32, children: u32) -> Splice {
        Splice { anchor, children }
    }

    fn round_trips(splice: Splice, rows: u32) {
        let mut expected = Vec::new();
        for row in 0..rows {
            expected.push(Slot::Row(row));
            if row == splice.anchor {
                expected.extend((0..splice.children).map(|index| Slot::Child { row, index }));
            }
        }
        assert_eq!(splice.len(rows) as usize, expected.len());
        for (position, slot) in expected.into_iter().enumerate() {
            assert_eq!(splice.slot(position as u32), slot, "slot at {position}");
            assert_eq!(splice.display(slot), position as u32, "display of {slot:?}");
        }
    }

    #[test]
    fn an_anchor_at_the_top_pushes_every_other_row_down() {
        round_trips(at(0, 3), 5);
        assert_eq!(at(0, 3).slot(1), Slot::Child { row: 0, index: 0 });
        assert_eq!(at(0, 3).slot(4), Slot::Row(1));
    }

    #[test]
    fn an_anchor_in_the_middle_leaves_the_rows_above_it_alone() {
        round_trips(at(2, 4), 6);
        assert_eq!(at(2, 4).slot(2), Slot::Row(2));
        assert_eq!(at(2, 4).slot(7), Slot::Row(3));
    }

    #[test]
    fn an_anchor_at_the_end_ends_in_its_children() {
        round_trips(at(4, 2), 5);
        assert_eq!(at(4, 2).slot(6), Slot::Child { row: 4, index: 1 });
    }

    #[test]
    fn no_children_is_no_splice() {
        round_trips(at(1, 0), 4);
        assert_eq!(at(1, 0).len(4), 4);
    }

    #[test]
    fn an_anchor_beyond_the_landed_rows_adds_nothing_yet() {
        let splice = at(9, 3);
        assert_eq!(splice.len(5), 5);
        for position in 0..5 {
            assert_eq!(splice.slot(position), Slot::Row(position));
            assert_eq!(splice.display(Slot::Row(position)), position);
        }
        assert_eq!(splice.len(10), 13);
    }

    #[test]
    fn no_splice_is_the_identity() {
        assert_eq!(slot(None, 3), Slot::Row(3));
        assert_eq!(display(None, Slot::Row(3)), 3);
        assert_eq!(len(None, 8), 8);
        assert_eq!(len(Some(at(1, 2)), 8), 10);
        assert_eq!(slot(Some(at(1, 2)), 2), Slot::Child { row: 1, index: 0 });
        assert_eq!(display(Some(at(1, 2)), Slot::Row(5)), 7);
    }
}

#[cfg(test)]
mod child_tests {
    use chrono::{Local, TimeZone, Utc};
    use postio_model::thread_view::{Line, ThreadDraft, ThreadMember, ThreadView};
    use postio_model::{DraftId, EmailAddress, MessageId, ThreadId};

    use super::*;

    fn member(id: i64, from: &str, line: Line) -> ThreadMember {
        ThreadMember {
            id: MessageId::new(id),
            from: Some(EmailAddress::new(
                Some(from),
                format!("{}@example.com", from.to_lowercase().replace(' ', ".")),
            )),
            own: false,
            at: Utc.with_ymd_and_hms(2026, 9, 21, 9, 14, 0).unwrap(),
            seen: true,
            has_attachments: false,
            line,
            answers: None,
            marker: false,
        }
    }

    fn view(members: Vec<ThreadMember>) -> ThreadView {
        ThreadView {
            thread: ThreadId::new(1),
            subject: None,
            members,
            draft: Some(ThreadDraft {
                id: DraftId::new(4),
                first_words: "Thanks, I will send".into(),
                updated_at: Utc.with_ymd_and_hms(2026, 9, 21, 11, 0, 0).unwrap(),
            }),
            muted: None,
            labels: Vec::new(),
        }
    }

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap()
    }

    fn child(view: &ThreadView, index: usize) -> ChildRowView {
        ChildRowView::member(view, index, &[], Keymap::defaults(), now()).unwrap()
    }

    #[test]
    fn a_child_says_who_what_and_when() {
        let mut ben = member(2, "Ben Ortiz", Line::Own("Looks good".into()));
        ben.seen = false;
        ben.has_attachments = true;
        ben.answers = Some(0);
        let v = view(vec![member(1, "Ana Ruiz", Line::Own("Plan".into())), ben]);
        let row = child(&v, 1);
        assert_eq!(row.initials, "BO");
        assert_eq!(row.sender, "Ben Ortiz");
        assert!(row.bold);
        assert_eq!(row.tag.as_deref(), Some("\u{21b3} to Ana"));
        assert_eq!(row.line, "Looks good");
        assert!(!row.placeholder);
        assert!(row.attachment);
        assert!(!row.draft);
        assert_eq!(row.time, crate::row::timestamp(v.members[1].at, now()));
        assert!(!child(&v, 0).bold, "read is not bold");
        assert_eq!(child(&v, 0).tag, None);
    }

    #[test]
    fn a_quoted_only_child_says_so_quietly() {
        let v = view(vec![member(1, "Ana Ruiz", Line::Quoted)]);
        let row = child(&v, 0);
        assert_eq!(row.line, "Quoted reply only");
        assert!(row.placeholder);
    }

    #[test]
    fn an_unfetched_child_shows_its_preview_and_an_empty_one_a_placeholder() {
        let v = view(vec![
            member(1, "Ana Ruiz", Line::Preview("Short preview".into())),
            member(2, "Ben Ortiz", Line::Preview(String::new())),
        ]);
        assert_eq!(child(&v, 0).line, "Short preview");
        assert!(!child(&v, 0).placeholder);
        assert_eq!(child(&v, 1).line, NO_PREVIEW);
        assert!(child(&v, 1).placeholder);
    }

    #[test]
    fn the_accounts_own_message_is_you() {
        let mut mine = member(1, "Test User", Line::Own("Sent".into()));
        mine.own = true;
        assert_eq!(child(&view(vec![mine]), 0).sender, "You");
    }

    #[test]
    fn the_draft_child_is_italic_unsent_and_has_no_hint() {
        let v = view(vec![member(1, "Ana Ruiz", Line::Own("Plan".into()))]);
        let row = ChildRowView::draft(&v, now()).unwrap();
        assert_eq!(row.sender, "Draft \u{b7} you");
        assert_eq!(row.line, "Thanks, I will send");
        assert_eq!(row.tag.as_deref(), Some("not sent"));
        assert!(row.draft);
        assert_eq!(row.focused_hint, None);
        let mut none = v;
        none.draft = None;
        assert_eq!(ChildRowView::draft(&none, now()), None);
    }

    #[test]
    fn the_focused_hint_is_the_keymap_s_reply_and_forward() {
        let v = view(vec![member(1, "Ana Ruiz", Line::Own("Plan".into()))]);
        let hint = child(&v, 0).focused_hint.unwrap();
        assert_eq!(hint.keys, ["r", "f"]);
        assert_eq!(hint.label, "this one");

        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert(CommandId::Reply.as_str().to_owned(), "w".to_owned());
        let rebound = Keymap::resolve(&overrides);
        assert_eq!(
            focused_hint(&rebound).keys,
            ["w", "f"],
            "a rebind reaches it"
        );
    }
}
