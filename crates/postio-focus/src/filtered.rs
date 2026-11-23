//! Filtered: what Focus filed away, a surface over the list. Nothing in it is deleted
//! automatically: `R` restores, the sweep asks first. Selection is the list's `Cursor` over its
//! rows: messages, never conversations or the "everything" predicate.

use postio_client::protocol::FilteredRow;
use postio_core::state::Selection;
use postio_core::{Command, CommandId};
use postio_model::MessageId;
use postio_ui::filtered as words;
use postio_ui::focus_list::FocusRow;
use postio_ui::hints;
use postio_ui::thread_list::{ChildRowView, Slot};

use crate::cursor::{Cursor, RowFacts, Rows};
use crate::feed::Step;
use crate::surfaces::Origin;
use crate::thread_expand::{Expansion, Pending};
use crate::verbs::Verbs;
use crate::{FocusController, Intent, Reply, Request, SurfaceKind, ToastKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredView {
    pub title: String,
    pub subtitle: String,
    pub note: String,
    pub sweep: String,
    pub sweep_key: Option<String>,
    pub restore: String,
    pub restore_key: Option<String>,
    pub tabs: Vec<FilteredTab>,
    pub rows: Vec<FilteredLine>,
    pub focused: Option<u32>,
    pub more: bool,
    pub footer: Vec<hints::Hint>,
    pub bar: Option<FilteredBar>,
    pub empty: Option<String>,
    /// When it is, `focused` is a display position, the children counted.
    pub expanded: Option<FilteredSplice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredSplice {
    pub anchor: u32,
    pub children: Vec<ChildRowView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredBar {
    pub summary: String,
    pub actions: Vec<FilteredBarAction>,
    pub hints: Vec<hints::Hint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredBarAction {
    pub command: CommandId,
    pub label: String,
    pub key: Option<String>,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredTab {
    pub name: String,
    pub count: u32,
    pub key: Option<String>,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredLine {
    pub message: MessageId,
    pub sender: String,
    pub subject: String,
    pub preview: Option<String>,
    pub pill: String,
    pub time: String,
    pub heading: Option<String>,
    pub selected: bool,
    pub messages: u32,
    /// The message as the store has it, for a frontend that opens it for itself.
    pub summary: postio_model::listing::MessageSummary,
}

#[derive(Debug)]
pub(crate) struct Filtered {
    tab: usize,
    counts: [u32; 7],
    counted: bool,
    rows: Vec<FilteredRow>,
    cursor: Cursor,
    more: bool,
    stamp: u64,
    expansion: Option<Expansion>,
    pending: Option<Pending>,
}

/// `threaded` names each row's thread, which only an expansion needs: a verb must never reach the
/// rest of a conversation.
struct FilteredRows<'a>(&'a [FilteredRow], bool);

impl Rows for FilteredRows<'_> {
    fn len(&self) -> u32 {
        self.0.len() as u32
    }

    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.0.get(position as usize).map(|row| RowFacts {
            id: row.message.id,
            digest: false,
            threads: if self.1 {
                row.message.thread.into_iter().collect()
            } else {
                Vec::new()
            },
            writes: false,
        })
    }

    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.0
            .iter()
            .position(|row| row.message.id == message)
            .map(|at| at as u32)
    }

    fn row(&self, position: u32) -> Option<FocusRow> {
        let message = self.0.get(position as usize)?.message.clone();
        let count = message.thread_count;
        let mut summary = postio_ui::focus_list::lone(message);
        summary.message_count = count;
        Some(FocusRow::conversation(summary))
    }
}

impl Filtered {
    fn with_cursor<T>(&mut self, f: impl FnOnce(&mut Cursor, &dyn Rows, u32) -> T) -> T {
        let mut cursor = std::mem::take(&mut self.cursor);
        let raw = FilteredRows(&self.rows, false);
        let through = self
            .expansion
            .as_ref()
            .and_then(|expansion| expansion.through(&raw, true));
        let rows: &dyn Rows = through.as_ref().map_or(&raw, |through| through);
        let out = f(&mut cursor, rows, rows.len());
        self.cursor = cursor;
        out
    }

    pub(crate) fn focused(&self) -> Option<usize> {
        self.cursor.position().map(|at| at as usize)
    }

    fn row_at(&self, index: usize) -> Option<(usize, bool)> {
        let Some(expansion) = &self.expansion else {
            return Some((index, false)).filter(|(at, _)| *at < self.rows.len());
        };
        let anchor = self.index_of_row(expansion.anchor)?;
        match expansion.splice(anchor as u32).slot(index as u32) {
            Slot::Row(row) => Some((row as usize, false)),
            Slot::Child { row, .. } => Some((row as usize, true)),
        }
    }

    fn index_of_row(&self, message: MessageId) -> Option<usize> {
        self.rows.iter().position(|row| row.message.id == message)
    }

    pub(crate) fn facts(&self, index: usize) -> Option<RowFacts> {
        let (row, child) = self.row_at(index)?;
        if !child {
            let row = self.rows.get(row)?;
            return Some(RowFacts {
                id: row.message.id,
                digest: false,
                threads: row.message.thread.into_iter().collect(),
                writes: false,
            });
        }
        let raw = FilteredRows(&self.rows, true);
        let through = self.expansion.as_ref()?.through(&raw, true)?;
        through.facts(index as u32)
    }

    pub(crate) fn footer_at(
        &self,
        index: u32,
        keymap: &postio_core::Keymap,
    ) -> Option<postio_ui::scope::FooterView> {
        let raw = FilteredRows(&self.rows, true);
        crate::thread_expand::footer_at(self.expansion.as_ref(), index, &raw, keymap)
    }

    pub(crate) fn index_of(&self, message: MessageId) -> Option<usize> {
        let raw = FilteredRows(&self.rows, true);
        let through = self
            .expansion
            .as_ref()
            .and_then(|expansion| expansion.through(&raw, true));
        match through {
            Some(through) => through.position_of(message).map(|at| at as usize),
            None => self.index_of_row(message),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len() + self.expansion.as_ref().map_or(0, |e| e.children() as usize)
    }

    pub(crate) fn subject(&self, index: usize) -> Option<String> {
        self.rows.get(index)?.message.subject.clone()
    }

    fn reason(&self) -> Option<String> {
        words::tab_reason(self.tab).flatten().map(str::to_owned)
    }
}

impl FocusController {
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
            counted: false,
            rows: Vec::new(),
            cursor: Cursor::default(),
            more: false,
            stamp,
            expansion: None,
            pending: None,
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

    pub(crate) fn filtered_command(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let focused = self.filtered.as_ref()?.focused();
        if let Some(tab) = words::TAB_COMMANDS.iter().position(|tab| *tab == id) {
            let stamp = self.stamp();
            let filtered = self.filtered.as_mut()?;
            filtered.tab = tab;
            filtered.more = false;
            filtered.stamp = stamp;
            // Another place, as another folder is: what was selected is not shown any more.
            let _ = filtered.cursor.leave(0);
            filtered.cursor.moved_to(None);
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
            CommandId::ToggleSelection
            | CommandId::ExtendSelectionDown
            | CommandId::ExtendSelectionUp => {
                let filtered = self.filtered.as_mut()?;
                let steps = filtered
                    .with_cursor(|cursor, rows, total| cursor.command(id, rows, None, total, None));
                self.filtered_drawn(steps.unwrap_or_default())
            }
            CommandId::SelectAll => {
                let filtered = self.filtered.as_mut()?;
                let steps =
                    filtered.with_cursor(|cursor, rows, total| cursor.select_rows(rows, total));
                self.filtered_drawn(steps)
            }
            CommandId::OpenMessage => match focused {
                Some(index) => self.open_filtered(index),
                None => Vec::new(),
            },
            CommandId::RestoreFiltered => self.restore_selected(),
            CommandId::SweepInbox => vec![Step::Ask(Request::SweepPreview)],
            // The selected messages, or the focused one alone: Filtered
            // lists messages, and the rest of a conversation may be in the
            // inbox, so the rows' conversations are not reached.
            CommandId::Delete
            | CommandId::DeleteThisMessage
            | CommandId::Flag
            | CommandId::ToggleRead => {
                let filtered = self.filtered.as_mut()?;
                let verbs = &mut self.verbs;
                let steps = filtered
                    .with_cursor(|cursor, rows, total| verbs.command(id, cursor, rows, total));
                self.filtered_drawn(steps.unwrap_or_default())
            }
            CommandId::Back
                if self.filtered.as_ref()?.cursor.selection() != Selection::default() =>
            {
                let filtered = self.filtered.as_mut()?;
                let steps = filtered.with_cursor(|cursor, _, total| cursor.clear(total));
                self.filtered_drawn(steps)
            }
            CommandId::Back | CommandId::GoToInbox => self.leave_filtered(rows),
            CommandId::GoToFiltered => Vec::new(),
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
            CommandId::Undo
            | CommandId::CheatSheet
            | CommandId::Search
            | CommandId::CommandPalette
            | CommandId::SavedSearch1
            | CommandId::SavedSearch2
            | CommandId::SavedSearch3
            | CommandId::SavedSearch4
            | CommandId::GoToFolders => return None,
            // Nobody can see the rows of a verb meant for the list behind; Filtered mail is
            // archived already.
            _ => Vec::new(),
        };
        Some(steps)
    }

    pub(crate) fn filtered_answers(id: CommandId) -> bool {
        words::TAB_COMMANDS.contains(&id)
            || matches!(
                id,
                CommandId::RestoreFiltered
                    | CommandId::SweepInbox
                    | CommandId::GoToFiltered
                    | CommandId::ToggleSelection
                    | CommandId::ExtendSelectionDown
                    | CommandId::ExtendSelectionUp
                    | CommandId::SelectAll
            )
    }

    fn step_filtered(&mut self, by: i32) -> Vec<Step> {
        let Some(filtered) = self.filtered.as_mut() else {
            return Vec::new();
        };
        let (steps, moved) = filtered.with_cursor(|cursor, rows, _| cursor.step(by, rows));
        if !moved {
            return Vec::new();
        }
        self.filtered_drawn(steps)
    }

    fn filtered_drawn(&self, steps: Vec<Step>) -> Vec<Step> {
        steps
            .into_iter()
            .map(|step| match step {
                Step::Show(Intent::Cursor { position, .. }) => {
                    Step::Show(Intent::FilteredFocus(Some(position)))
                }
                Step::Show(Intent::Selection { selection, summary }) => {
                    Step::Show(Intent::FilteredSelection {
                        selected: selection.ids().map(<[_]>::to_vec).unwrap_or_default(),
                        bar: summary.map(|summary| self.filtered_bar(summary)),
                    })
                }
                Step::Show(Intent::Splice { anchor, children }) => {
                    Step::Show(Intent::FilteredSplice { anchor, children })
                }
                Step::Show(Intent::Unsplice { anchor }) => {
                    Step::Show(Intent::FilteredUnsplice { anchor })
                }
                other => other,
            })
            .collect()
    }

    fn filtered_bar(&self, summary: String) -> FilteredBar {
        let keymap = self.keymap();
        FilteredBar {
            summary,
            actions: words::BAR_VERBS
                .iter()
                .enumerate()
                .map(|(index, verb)| FilteredBarAction {
                    command: verb.command,
                    label: verb.label.to_owned(),
                    key: hints::key(keymap, verb.command),
                    primary: index == 0,
                })
                .collect(),
            hints: words::bar_hints(keymap),
        }
    }

    /// One command, so one Undo takes them all back. The keyboard steps to the first row below
    /// that stays.
    fn restore_selected(&mut self) -> Vec<Step> {
        let Some(filtered) = self.filtered.as_mut() else {
            return Vec::new();
        };
        let (aims, steps) = filtered.with_cursor(|cursor, rows, total| {
            let aims = Verbs::aimed(cursor, rows, CommandId::Archive).aims;
            let gone = match cursor.selection() {
                Selection::These(ids) if !ids.is_empty() => ids,
                _ => cursor.row(rows).map(|row| row.id).into_iter().collect(),
            };
            let mut steps = Verbs::past_removed(&Selection::These(gone), cursor, rows);
            steps.extend(cursor.clear(total));
            (aims, steps)
        });
        if aims.is_empty() {
            return Vec::new();
        }
        let mut steps = self.filtered_drawn(steps);
        steps.extend(aims.into_iter().map(|target| {
            Step::Ask(Request::Post(Command::RestoreFiltered {
                target,
                restored: true,
            }))
        }));
        steps
    }

    pub(crate) fn open_filtered(&mut self, index: usize) -> Vec<Step> {
        let Some(filtered) = self.filtered.as_mut() else {
            return Vec::new();
        };
        let Some(row) = filtered.facts(index) else {
            return Vec::new();
        };
        filtered.cursor.moved_to(Some(index as u32));
        let total = filtered.len() as u32;
        let mut steps = self
            .surfaces
            .open(&row, index as u32, total, self.policy.caps.stacking);
        self.surfaces.set_origin(Origin::Filtered);
        steps.insert(0, Step::Show(Intent::FilteredFocus(Some(index as u32))));
        steps
    }

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

    /// The list's own thread input (`thread_expand.rs`) run over Filtered's rows, cursor and
    /// expansion, which stand in for the list's; `None` when the input is not one.
    pub(crate) fn filtered_thread_input(&mut self, input: &crate::Input) -> Option<Vec<Step>> {
        use crate::Input;
        use postio_core::Event;
        if self.surfaces.top() != Some(SurfaceKind::Filtered) {
            return None;
        }
        let filtered = self.filtered.as_ref()?;
        let open = |thread: &postio_model::ThreadId| {
            filtered
                .expansion
                .as_ref()
                .is_some_and(|expansion| expansion.thread == *thread)
        };
        match input {
            Input::Command(CommandId::ExpandThread | CommandId::CollapseThread) => {}
            Input::Reply(_, Reply::ThreadRead { thread, .. })
                if open(thread)
                    || filtered
                        .pending
                        .is_some_and(|pending| pending.thread() == *thread) => {}
            Input::Event(Event::ThreadChanged { thread, .. }) if open(thread) => {
                return Some(vec![Step::Ask(Request::ReadThread(*thread))]);
            }
            _ => return None,
        }
        let mut rows = std::mem::take(&mut self.filtered.as_mut()?.rows);
        self.swap_filtered_state();
        let steps = {
            let raw = FilteredRows(&rows, true);
            let through = self.through(&raw);
            let seen: &dyn Rows = through.as_ref().map_or(&raw, |through| through);
            match input {
                Input::Command(CommandId::ExpandThread) => self.expand(&raw),
                Input::Command(_) => self.collapse(&raw),
                Input::Reply(ticket, Reply::ThreadRead { thread, view }) => {
                    self.thread_replied(*ticket, *thread, view, &raw, seen)
                }
                _ => Vec::new(),
            }
        };
        self.swap_filtered_state();
        if let Some(filtered) = self.filtered.as_mut() {
            std::mem::swap(&mut filtered.rows, &mut rows);
        }
        Some(self.filtered_drawn(steps))
    }

    fn swap_filtered_state(&mut self) {
        if let Some(filtered) = self.filtered.as_mut() {
            std::mem::swap(&mut self.cursor, &mut filtered.cursor);
            std::mem::swap(&mut self.expansion, &mut filtered.expansion);
            std::mem::swap(&mut self.pending, &mut filtered.pending);
        }
    }

    pub(crate) fn filtered_point(&mut self, index: u32) -> Vec<Step> {
        let Some(filtered) = self.filtered.as_mut() else {
            return Vec::new();
        };
        if index as usize >= filtered.len() || filtered.focused() == Some(index as usize) {
            return Vec::new();
        }
        filtered.cursor.moved_to(Some(index));
        vec![Step::Show(Intent::FilteredFocus(Some(index)))]
    }

    pub(crate) fn filtered_pick(&mut self, position: u32, range: bool) -> Vec<Step> {
        let Some(filtered) = self.filtered.as_mut() else {
            return Vec::new();
        };
        let steps =
            filtered.with_cursor(|cursor, rows, total| cursor.pick(position, range, rows, total));
        self.filtered_drawn(steps)
    }

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

    pub(crate) fn filtered_reply(&mut self, reply: Reply) -> Vec<Step> {
        match reply {
            Reply::FilteredTabs(Ok(reasons)) => {
                let Some(filtered) = self.filtered.as_mut() else {
                    return Vec::new();
                };
                filtered.counts = words::tab_counts(&reasons);
                filtered.counted = true;
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
                let on_child = filtered
                    .focused()
                    .and_then(|at| filtered.row_at(at))
                    .is_some_and(|(_, child)| child);
                let kept = filtered
                    .focused()
                    .and_then(|at| filtered.row_at(at))
                    .and_then(|(row, _)| filtered.rows.get(row))
                    .map(|row| row.message.id);
                let anchor_at = filtered
                    .expansion
                    .as_ref()
                    .and_then(|expansion| filtered.index_of_row(expansion.anchor));
                let was_focused = filtered.focused();
                if offset == 0 {
                    filtered.rows.clear();
                }
                filtered.rows.extend(page);
                if filtered.expansion.as_ref().is_some_and(|expansion| {
                    filtered.index_of_row(expansion.anchor) != anchor_at || anchor_at.is_none()
                }) {
                    filtered.expansion = None;
                }
                let focused = match kept.and_then(|message| filtered.index_of_row(message)) {
                    Some(_) if on_child && filtered.expansion.is_some() => was_focused,
                    Some(at) => Some(
                        filtered
                            .expansion
                            .as_ref()
                            .and_then(|expansion| {
                                let anchor = filtered.index_of_row(expansion.anchor)?;
                                Some(
                                    expansion
                                        .splice(anchor as u32)
                                        .display(Slot::Row(at as u32))
                                        as usize,
                                )
                            })
                            .unwrap_or(at),
                    ),
                    None if offset > 0 => filtered.focused(),
                    None => (!filtered.rows.is_empty()).then_some(0),
                };
                filtered.cursor.moved_to(focused.map(|at| at as u32));
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
        let selection = filtered.cursor.selection();
        let bar = postio_ui::selection::summary(&selection, Some(filtered.rows.len() as u32), &[])
            .map(|summary| self.filtered_bar(summary));
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
                    selected: selection.contains(row.message.id),
                    messages: row.message.thread_count.max(1),
                    summary: row.message.clone(),
                })
                .collect(),
            focused: filtered.focused().map(|at| at as u32),
            more: filtered.more,
            footer: words::footer(keymap, !filtered.rows.is_empty()),
            bar,
            empty: (filtered.counted && filtered.rows.is_empty() && !filtered.more)
                .then(|| words::EMPTY.to_owned()),
            expanded: filtered.expansion.as_ref().and_then(|expansion| {
                Some(FilteredSplice {
                    anchor: filtered.index_of_row(expansion.anchor)? as u32,
                    children: self.child_views(&expansion.view),
                })
            }),
        })
    }
}
