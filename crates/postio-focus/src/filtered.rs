//! Filtered: what Focus filed away, as a view of its own (research R2,
//! slice 12; spec 007 US9, screen 21).
//!
//! Moved from `postio-gtk`'s `filtered.rs` and the window's Filtered
//! handlers (`show_filtered`, `leave_filtered`, `filtered_action`,
//! `restore_filtered`, `ask_sweep`). The words are `postio_ui::filtered`'s;
//! the controller keeps which tab is showing, the rows read so far, the row
//! with the keyboard, and which read is current. It is a surface over the
//! list ([`SurfaceKind::Filtered`]) that the controller puts on the stack at
//! `g f` and takes off at Back or `g i`.
//!
//! Nothing in it is deleted (C4): `R` restores, a tab narrows, the sweep
//! asks first and moves mail *into* Filtered.

use postio_client::protocol::FilteredRow;
use postio_core::{Command, CommandId, MessageTarget};
use postio_model::MessageId;
use postio_ui::filtered as words;
use postio_ui::hints;

use crate::cursor::{RowFacts, Rows};
use crate::feed::Step;
use crate::surfaces::Origin;
use crate::{FocusController, Intent, Reply, Request, SurfaceKind, ToastKind};

/// Filtered, drawable as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredView {
    /// "Filtered".
    pub title: String,
    /// The line under the title.
    pub subtitle: String,
    /// The line right of the tabs: nothing here is deleted (C4).
    pub note: String,
    /// The sweep button's words.
    pub sweep: String,
    /// Its key, as the keymap in force spells it.
    pub sweep_key: Option<String>,
    /// The focused row's restore button's words.
    pub restore: String,
    /// Its key.
    pub restore_key: Option<String>,
    /// The tabs, `1` to `7`.
    pub tabs: Vec<FilteredTab>,
    /// The rows read so far, newest first.
    pub rows: Vec<FilteredLine>,
    /// The row with the keyboard, an index into `rows`.
    pub focused: Option<u32>,
    /// Whether there may be more: [`crate::Input::FilteredMore`] reads them.
    pub more: bool,
    /// The footer's hints.
    pub footer: Vec<hints::Hint>,
}

/// One of Filtered's tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredTab {
    /// "All", "Spam", "Promotions".
    pub name: String,
    /// How many messages it holds.
    pub count: u32,
    /// The number key that shows it.
    pub key: Option<String>,
    /// Whether it is the tab showing.
    pub on: bool,
}

/// One filtered message, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredLine {
    /// The message: what Return opens and `R` restores.
    pub message: MessageId,
    /// Who sent it.
    pub sender: String,
    /// Its subject.
    pub subject: String,
    /// Its first line.
    pub preview: Option<String>,
    /// The reason pill: "notification · Forge".
    pub pill: String,
    /// When it was filed: "09:12", "Tue".
    pub time: String,
    /// The day heading drawn above it, for the first row of each day.
    pub heading: Option<String>,
}

/// What the controller keeps of Filtered.
#[derive(Debug)]
pub(crate) struct Filtered {
    /// The tab showing, `0` for All.
    tab: usize,
    /// Each tab's count, as last read.
    counts: [u32; 7],
    /// The rows read, in order.
    rows: Vec<FilteredRow>,
    /// The row with the keyboard.
    focused: Option<usize>,
    /// Whether the last page read was full.
    more: bool,
    /// The read current now: a page for another is dropped.
    stamp: u64,
}

impl Filtered {
    /// The message at `index`, as a verb or the open message sees it.
    pub(crate) fn facts(&self, index: usize) -> Option<RowFacts> {
        self.rows.get(index).map(|row| RowFacts {
            id: row.message.id,
            digest: false,
            threads: row.message.thread.into_iter().collect(),
            writes: false,
        })
    }

    /// Where `message` is among the rows read.
    pub(crate) fn index_of(&self, message: MessageId) -> Option<usize> {
        self.rows.iter().position(|row| row.message.id == message)
    }

    /// How many rows are read.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// The subject of the row at `index`.
    pub(crate) fn subject(&self, index: usize) -> Option<String> {
        self.rows.get(index)?.message.subject.clone()
    }

    /// The reason the tab showing narrows to.
    fn reason(&self) -> Option<String> {
        words::tab_reason(self.tab).flatten().map(str::to_owned)
    }
}

impl FocusController {
    /// `g f`: Filtered over the list, its tabs counted and its first page
    /// of All read.
    pub(crate) fn show_filtered(&mut self) -> Vec<Step> {
        if self.filtered.is_some() {
            return Vec::new();
        }
        let stamp = self.stamp();
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Filtered, self.policy.caps.stacking);
        self.filtered = Some(Filtered {
            tab: 0,
            counts: [0; 7],
            rows: Vec::new(),
            focused: None,
            more: false,
            stamp,
        });
        steps.push(Step::Show(Intent::ShowFiltered));
        steps.push(Step::Ask(Request::FilteredTabs));
        steps.push(Step::Ask(Request::Filtered {
            reason: None,
            offset: 0,
            stamp,
        }));
        steps
    }

    /// A command while Filtered is on top, or `None` when the list's rules
    /// apply to it (the bar, the key map, Undo).
    pub(crate) fn filtered_command(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let focused = self.filtered.as_ref()?.focused;
        if let Some(tab) = words::TAB_COMMANDS.iter().position(|tab| *tab == id) {
            let stamp = self.stamp();
            let filtered = self.filtered.as_mut()?;
            filtered.tab = tab;
            filtered.more = false;
            filtered.stamp = stamp;
            let reason = filtered.reason();
            return Some(vec![
                Step::Show(Intent::Filtered(Box::new(self.filtered_view()?))),
                Step::Ask(Request::Filtered {
                    reason,
                    offset: 0,
                    stamp,
                }),
            ]);
        }
        let steps = match id {
            CommandId::NextMessage | CommandId::PrevMessage => {
                let by = if id == CommandId::NextMessage { 1 } else { -1 };
                self.step_filtered(by)
            }
            CommandId::OpenMessage => match focused {
                Some(index) => self.open_filtered(index),
                None => Vec::new(),
            },
            CommandId::RestoreFiltered => match self
                .filtered
                .as_ref()
                .zip(focused)
                .and_then(|(filtered, at)| filtered.facts(at))
            {
                Some(row) => vec![Step::Ask(Request::Post(Command::RestoreFiltered {
                    target: MessageTarget::Messages(vec![row.id]),
                    restored: true,
                }))],
                None => Vec::new(),
            },
            CommandId::SweepInbox => vec![Step::Ask(Request::SweepPreview)],
            CommandId::Back | CommandId::GoToInbox => self.leave_filtered(rows),
            CommandId::GoToFiltered => Vec::new(),
            // Another place: Filtered goes, and the list goes there.
            CommandId::GoToDrafts
            | CommandId::GoToSent
            | CommandId::GoToArchive
            | CommandId::GoToSnoozed
            | CommandId::GoToFlagged
            | CommandId::GoToJunk
            | CommandId::GoToTrash
            | CommandId::GoToOutbox => {
                let mut steps = self.leave_filtered(rows);
                steps.extend(self.going(id, rows).unwrap_or_default());
                steps
            }
            // What stays the app's wherever it is.
            CommandId::Undo
            | CommandId::CheatSheet
            | CommandId::Search
            | CommandId::CommandPalette
            | CommandId::SavedSearch1
            | CommandId::SavedSearch2
            | CommandId::SavedSearch3
            | CommandId::SavedSearch4
            | CommandId::GoToFolders => return None,
            // A verb meant for the list behind: nobody can see its rows.
            _ => Vec::new(),
        };
        Some(steps)
    }

    /// Whether `id` is Filtered's own while it is on top.
    pub(crate) fn filtered_answers(id: CommandId) -> bool {
        words::TAB_COMMANDS.contains(&id)
            || matches!(
                id,
                CommandId::RestoreFiltered | CommandId::SweepInbox | CommandId::GoToFiltered
            )
    }

    /// `j`/`k` in Filtered: the keyboard to the next or previous row.
    fn step_filtered(&mut self, by: i32) -> Vec<Step> {
        let Some(filtered) = self.filtered.as_mut() else {
            return Vec::new();
        };
        if filtered.rows.is_empty() {
            return Vec::new();
        }
        let last = filtered.rows.len() as i64 - 1;
        let next = filtered
            .focused
            .map_or(0, |at| (at as i64 + i64::from(by)).clamp(0, last)) as usize;
        if filtered.focused == Some(next) {
            return Vec::new();
        }
        filtered.focused = Some(next);
        vec![Step::Show(Intent::FilteredFocus(Some(next as u32)))]
    }

    /// Open Filtered's message at `index` over it, to be read.
    pub(crate) fn open_filtered(&mut self, index: usize) -> Vec<Step> {
        let Some(filtered) = self.filtered.as_mut() else {
            return Vec::new();
        };
        let Some(row) = filtered.facts(index) else {
            return Vec::new();
        };
        filtered.focused = Some(index);
        let total = filtered.rows.len() as u32;
        let mut steps = self
            .surfaces
            .open(&row, index as u32, total, self.policy.caps.stacking);
        self.surfaces.set_origin(Origin::Filtered);
        steps.insert(0, Step::Show(Intent::FilteredFocus(Some(index as u32))));
        steps
    }

    /// Back, or `g i`: Filtered goes, and the list has the keyboard on its
    /// first row (GTK's `leave_filtered`).
    fn leave_filtered(&mut self, rows: &dyn Rows) -> Vec<Step> {
        self.filtered = None;
        self.surfaces.dismiss(SurfaceKind::Filtered);
        let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Filtered))];
        if !rows.is_empty() {
            steps.extend(self.cursor.place(0, rows));
        }
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
    }

    /// A click on Filtered's row at `index`.
    pub(crate) fn filtered_point(&mut self, index: u32) -> Vec<Step> {
        let Some(filtered) = self.filtered.as_mut() else {
            return Vec::new();
        };
        if index as usize >= filtered.rows.len() || filtered.focused == Some(index as usize) {
            return Vec::new();
        }
        filtered.focused = Some(index as usize);
        vec![Step::Show(Intent::FilteredFocus(Some(index)))]
    }

    /// Filtered was scrolled to its end: its next page, when there may be
    /// one.
    pub(crate) fn filtered_more(&mut self) -> Vec<Step> {
        let Some(filtered) = self.filtered.as_mut().filter(|filtered| filtered.more) else {
            return Vec::new();
        };
        filtered.more = false;
        vec![Step::Ask(Request::Filtered {
            reason: filtered.reason(),
            offset: filtered.rows.len() as u32,
            stamp: filtered.stamp,
        })]
    }

    /// Mail moved -- a restore, its undo, mail filed while the view is up:
    /// what Filtered shows is read again, the keyboard kept on its message.
    pub(crate) fn filtered_event(&mut self, event: &postio_core::Event) -> Vec<Step> {
        use postio_core::Event;
        if self.filtered.is_none()
            || !matches!(
                event,
                Event::MessageListChanged { .. }
                    | Event::UndoPerformed { .. }
                    | Event::ActionCompleted { .. }
            )
        {
            return Vec::new();
        }
        let stamp = self.stamp();
        let Some(filtered) = self.filtered.as_mut() else {
            return Vec::new();
        };
        filtered.stamp = stamp;
        vec![
            Step::Ask(Request::FilteredTabs),
            Step::Ask(Request::Filtered {
                reason: filtered.reason(),
                offset: 0,
                stamp,
            }),
        ]
    }

    /// An answer for Filtered.
    pub(crate) fn filtered_reply(&mut self, reply: Reply) -> Vec<Step> {
        match reply {
            Reply::FilteredTabs(Ok(reasons)) => {
                let Some(filtered) = self.filtered.as_mut() else {
                    return Vec::new();
                };
                filtered.counts = words::tab_counts(&reasons);
                self.filtered_view()
                    .map(|view| vec![Step::Show(Intent::Filtered(Box::new(view)))])
                    .unwrap_or_default()
            }
            Reply::FilteredTabs(Err(error)) => {
                tracing::warn!(%error, "Focus could not count Filtered");
                Vec::new()
            }
            Reply::Filtered {
                stamp,
                offset,
                answer,
            } => {
                let Some(filtered) = self.filtered.as_mut().filter(|it| it.stamp == stamp) else {
                    return Vec::new();
                };
                let page = match answer {
                    Ok(page) => page,
                    Err(error) => {
                        tracing::warn!(%error, "Focus could not read Filtered");
                        return Vec::new();
                    }
                };
                filtered.more = words::page_is_full(page.len());
                let kept = filtered
                    .focused
                    .and_then(|at| filtered.rows.get(at))
                    .map(|row| row.message.id);
                if offset == 0 {
                    filtered.rows.clear();
                }
                filtered.rows.extend(page);
                filtered.focused = match kept.and_then(|message| filtered.index_of(message)) {
                    Some(at) => Some(at),
                    None if offset > 0 => filtered.focused,
                    None => (!filtered.rows.is_empty()).then_some(0),
                };
                self.filtered_view()
                    .map(|view| vec![Step::Show(Intent::Filtered(Box::new(view)))])
                    .unwrap_or_default()
            }
            Reply::SweepPreview(Ok(0)) => vec![Step::Show(Intent::Toast {
                text: words::SWEEP_NOTHING.to_owned(),
                kind: ToastKind::Notice,
            })],
            Reply::SweepPreview(Ok(count)) => {
                let undo = hints::key(self.keymap(), CommandId::Undo);
                vec![self.ask_first(
                    words::SWEEP_HEADING.to_owned(),
                    words::sweep_body(count, undo.as_deref()),
                    words::sweep_action(count),
                    false,
                    Command::SweepInbox,
                )]
            }
            Reply::SweepPreview(Err(error)) => vec![Step::Show(Intent::Toast {
                text: error,
                kind: ToastKind::Notice,
            })],
            _ => Vec::new(),
        }
    }

    /// Filtered in words, as it stands.
    pub(crate) fn filtered_view(&self) -> Option<FilteredView> {
        let filtered = self.filtered.as_ref()?;
        let keymap = self.keymap();
        let now = self.now();
        let days: Vec<_> = filtered
            .rows
            .iter()
            .map(|row| row.at.with_timezone(&chrono::Local).date_naive())
            .collect();
        let headings = words::day_headings(&days, now.date_naive());
        Some(FilteredView {
            title: words::TITLE.to_owned(),
            subtitle: words::SUBTITLE.to_owned(),
            note: words::NOTE.to_owned(),
            sweep: words::SWEEP_BUTTON.to_owned(),
            sweep_key: hints::key(keymap, CommandId::SweepInbox),
            restore: words::RESTORE.to_owned(),
            restore_key: hints::key(keymap, CommandId::RestoreFiltered),
            tabs: words::TABS
                .iter()
                .enumerate()
                .map(|(index, (_, name))| FilteredTab {
                    name: (*name).to_owned(),
                    count: filtered.counts[index],
                    key: hints::key(keymap, words::TAB_COMMANDS[index]),
                    on: index == filtered.tab,
                })
                .collect(),
            rows: filtered
                .rows
                .iter()
                .zip(headings)
                .map(|(row, heading)| FilteredLine {
                    message: row.message.id,
                    sender: row
                        .message
                        .from
                        .as_ref()
                        .map(|from| from.display().to_owned())
                        .unwrap_or_default(),
                    subject: row.message.subject.clone().unwrap_or_default(),
                    preview: row.message.preview.clone(),
                    pill: words::pill(&row.reason, row.source.as_deref()),
                    time: postio_ui::row::timestamp(row.at, now),
                    heading,
                })
                .collect(),
            focused: filtered.focused.map(|at| at as u32),
            more: filtered.more,
            footer: words::footer(keymap),
        })
    }
}
