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
//! Nothing in it is deleted automatically (C4): `R` restores, a tab
//! narrows, the sweep asks first and moves mail *into* Filtered. Delete,
//! Flag and read act on the selection, or on the focused message when
//! nothing is selected, as the person asked; a deleted one leaves the view
//! because the store no longer lists it.
//!
//! Selecting is the list's, not a second model of it: Filtered holds a
//! [`Cursor`] of its own over its rows, so `x`, `J`/`K`, select all, the
//! modified clicks and Back's first rung follow the same rules as in a
//! folder, and the verbs aim through the same [`Verbs`]. A selection is the
//! rows' messages, not their conversations -- Filtered lists messages --
//! and never the "everything" predicate: the rows read are all it means.

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
    /// The selection bar, while anything is selected.
    pub bar: Option<FilteredBar>,
    /// What to say in place of the rows when the tab has none, once it has
    /// been counted.
    pub empty: Option<String>,
    /// The thread open under one of the rows (spec 013 FR-009). When it
    /// is, `focused` is a display position, the children counted.
    pub expanded: Option<FilteredSplice>,
}

/// The children of Filtered's expanded thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredSplice {
    /// The expanded row, an index into the view's `rows`.
    pub anchor: u32,
    /// One per member, oldest first, then the unsent reply.
    pub children: Vec<ChildRowView>,
}

/// What the bar over Filtered says while anything is selected: the count,
/// the verbs with their keys, and the selection's own keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredBar {
    /// "3 selected".
    pub summary: String,
    /// The verbs, the first the one a selection is usually for.
    pub actions: Vec<FilteredBarAction>,
    /// "x toggle", "J/K extend", "Esc clear".
    pub hints: Vec<hints::Hint>,
}

/// One verb on the selection bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredBarAction {
    /// The command a press runs.
    pub command: CommandId,
    /// Its words.
    pub label: String,
    /// Its key, as the keymap in force spells it.
    pub key: Option<String>,
    /// Whether it is the verb a selection is usually for.
    pub primary: bool,
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
    /// Whether it is in the selection: its box is checked.
    pub selected: bool,
    /// How many messages its conversation holds: more than one and `Right`
    /// opens it in place (spec 013 FR-009).
    pub messages: u32,
}

/// What the controller keeps of Filtered.
#[derive(Debug)]
pub(crate) struct Filtered {
    /// The tab showing, `0` for All.
    tab: usize,
    /// Each tab's count, as last read.
    counts: [u32; 7],
    /// Whether the counts have been read once.
    counted: bool,
    /// The rows read, in order.
    rows: Vec<FilteredRow>,
    /// The row with the keyboard, and what is selected: the list's own
    /// rules over these rows.
    cursor: Cursor,
    /// Whether the last page read was full.
    more: bool,
    /// The read current now: a page for another is dropped.
    stamp: u64,
    /// The one thread open in place (spec 013 FR-009), with the read in
    /// flight for it: the controller's `expansion` and `pending` while
    /// Filtered has the keyboard.
    expansion: Option<Expansion>,
    pending: Option<Pending>,
}

/// The list's cursor and selection rules over rows that are only
/// messages, for a frontend that draws Filtered itself and keeps its own
/// rows (GTK's Filtered reads its pages directly). It is [`Cursor`] and
/// nothing else: the controller's Filtered holds the same one, so `x`,
/// `J`/`K`, select all, the modified clicks and Back's first rung cannot
/// differ between the two.
#[derive(Debug, Default)]
pub struct RowPicker {
    cursor: Cursor,
    ids: Vec<MessageId>,
}

/// What a [`RowPicker`] call changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PickChange {
    /// The row the keyboard went to.
    pub focus: Option<u32>,
    /// The messages selected now, when the selection changed.
    pub selection: Option<Vec<MessageId>>,
}

struct IdRows<'a>(&'a [MessageId]);

impl Rows for IdRows<'_> {
    fn len(&self) -> u32 {
        self.0.len() as u32
    }

    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.0.get(position as usize).map(|id| RowFacts {
            id: *id,
            digest: false,
            threads: Vec::new(),
            writes: false,
        })
    }

    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.0
            .iter()
            .position(|id| *id == message)
            .map(|at| at as u32)
    }
}

impl RowPicker {
    /// A picker over no rows.
    pub fn new() -> Self {
        Self::default()
    }

    /// The rows are these now. The keyboard stays on its message, or goes
    /// to the first row when that one is gone; the selection is kept.
    pub fn set_rows(&mut self, ids: Vec<MessageId>) {
        let kept = self
            .focused()
            .and_then(|at| self.ids.get(at as usize).copied());
        self.ids = ids;
        let at = kept
            .and_then(|message| IdRows(&self.ids).position_of(message))
            .or((!self.ids.is_empty()).then_some(0));
        self.cursor.moved_to(at);
    }

    /// The row with the keyboard.
    pub fn focused(&self) -> Option<u32> {
        self.cursor.position()
    }

    /// The frontend's own list put the keyboard on `position`.
    pub fn focus(&mut self, position: Option<u32>) {
        self.cursor.moved_to(position);
    }

    /// The messages selected, in the order they were taken.
    pub fn selected(&self) -> Vec<MessageId> {
        self.cursor
            .selection()
            .ids()
            .map(<[_]>::to_vec)
            .unwrap_or_default()
    }

    /// What a verb is about: the selection, or the focused row alone.
    pub fn aimed(&self) -> Vec<MessageId> {
        let selected = self.selected();
        if !selected.is_empty() {
            return selected;
        }
        self.focused()
            .and_then(|at| self.ids.get(at as usize).copied())
            .into_iter()
            .collect()
    }

    /// `j`, `k`, `x`, `J`, `K`, select all, and Back's first rung: the
    /// list's rules. Any other command changes nothing.
    pub fn run(&mut self, id: CommandId) -> PickChange {
        let total = self.ids.len() as u32;
        let steps = match id {
            CommandId::SelectAll => self.cursor.select_rows(&IdRows(&self.ids), total),
            CommandId::Back => self.cursor.clear(total),
            _ => self
                .cursor
                .command(id, &IdRows(&self.ids), None, total, None)
                .unwrap_or_default(),
        };
        Self::changed(steps)
    }

    /// A modified click: `range` for Shift, a toggle otherwise.
    pub fn pick(&mut self, position: u32, range: bool) -> PickChange {
        let total = self.ids.len() as u32;
        let steps = self.cursor.pick(position, range, &IdRows(&self.ids), total);
        Self::changed(steps)
    }

    /// Drop the selection; the keyboard stays.
    pub fn clear(&mut self) -> PickChange {
        self.run(CommandId::Back)
    }

    fn changed(steps: Vec<Step>) -> PickChange {
        let mut change = PickChange::default();
        for step in steps {
            match step {
                Step::Show(Intent::Cursor { position, .. }) => change.focus = Some(position),
                Step::Show(Intent::Selection { selection, .. }) => {
                    change.selection = Some(selection.ids().map(<[_]>::to_vec).unwrap_or_default());
                }
                _ => {}
            }
        }
        change
    }
}

/// Filtered's rows as the cursor reads them: messages, never conversations.
/// `threaded` names each row's thread, which only an expansion needs: a
/// verb must never reach the rest of a conversation.
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
    /// Run `f` with the cursor and the rows it reads (an expanded
    /// thread's children among them), and the count a selection's words
    /// use.
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

    /// The row with the keyboard, a display position.
    pub(crate) fn focused(&self) -> Option<usize> {
        self.cursor.position().map(|at| at as usize)
    }

    /// The row the display position `index` stands for, and whether it is a
    /// child's.
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

    /// The message at display position `index`, as a verb or the open
    /// message sees it.
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

    /// The footer for the row or child at display `index`.
    pub(crate) fn footer_at(
        &self,
        index: u32,
        keymap: &postio_core::Keymap,
    ) -> Option<postio_ui::scope::FooterView> {
        let raw = FilteredRows(&self.rows, true);
        crate::thread_expand::footer_at(self.expansion.as_ref(), index, &raw, keymap)
    }

    /// Where `message` is among the rows read, children counted.
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

    /// How many rows are read, children counted.
    pub(crate) fn len(&self) -> usize {
        self.rows.len() + self.expansion.as_ref().map_or(0, |e| e.children() as usize)
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

    /// A command while Filtered is on top, or `None` when the list's rules
    /// apply to it (the bar, the key map, Undo).
    pub(crate) fn filtered_command(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let focused = self.filtered.as_ref()?.focused();
        if let Some(tab) = words::TAB_COMMANDS.iter().position(|tab| *tab == id) {
            let stamp = self.stamp();
            let filtered = self.filtered.as_mut()?;
            filtered.tab = tab;
            filtered.more = false;
            filtered.stamp = stamp;
            // Another place, as another folder is: what was selected is not
            // shown any more. The view drawn next says so; nothing else
            // needs telling.
            let _ = filtered.cursor.leave(0);
            // And it opens on its first row, as every place does.
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
            CommandId::Delete | CommandId::Flag | CommandId::ToggleRead => {
                let filtered = self.filtered.as_mut()?;
                let verbs = &mut self.verbs;
                let steps = filtered
                    .with_cursor(|cursor, rows, total| verbs.command(id, cursor, rows, total));
                self.filtered_drawn(steps.unwrap_or_default())
            }
            // Back's first rung drops the selection, as in a folder; the
            // next one leaves.
            CommandId::Back
                if self.filtered.as_ref()?.cursor.selection() != Selection::default() =>
            {
                let filtered = self.filtered.as_mut()?;
                let steps = filtered.with_cursor(|cursor, _, total| cursor.clear(total));
                self.filtered_drawn(steps)
            }
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
            // Archive among them: Filtered mail is archived already.
            _ => Vec::new(),
        };
        Some(steps)
    }

    /// Whether `id` is Filtered's own while it is on top.
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

    /// `j`/`k` in Filtered: the keyboard to the next or previous row.
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

    /// Cursor and selection steps, as Filtered's own intents: the list's
    /// `Cursor` and `Selection` are about the rows behind it.
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
                // Filtered has its own table: the list's splice is not it.
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

    /// The bar over Filtered, for a selection `summary` counts.
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

    /// `R`: every selected row back to the inbox -- the focused one with
    /// nothing selected -- as one command, so one Undo takes them all
    /// back. What was selected has been acted on, so it goes, and the
    /// keyboard steps to the first row below that stays.
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

    /// Open Filtered's message at `index` over it, to be read.
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

    /// The thread inputs while Filtered has the keyboard: `Right`, `Left`,
    /// the read's answer, a change to the open thread. They are the list's
    /// own (`thread_expand.rs`) run over Filtered's rows, cursor and
    /// expansion, which stand in for the list's while it does; what comes
    /// out is Filtered's intents. `None` when the input is not one.
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
        // Filtered's state stands in for the list's for this one input.
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

    /// Exchange the list's cursor, expansion and pending read with
    /// Filtered's.
    fn swap_filtered_state(&mut self) {
        if let Some(filtered) = self.filtered.as_mut() {
            std::mem::swap(&mut self.cursor, &mut filtered.cursor);
            std::mem::swap(&mut self.expansion, &mut filtered.expansion);
            std::mem::swap(&mut self.pending, &mut filtered.pending);
        }
    }

    /// A click on Filtered's row at `index`.
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

    /// A modified click on Filtered's row at `position`.
    pub(crate) fn filtered_pick(&mut self, position: u32, range: bool) -> Vec<Step> {
        let Some(filtered) = self.filtered.as_mut() else {
            return Vec::new();
        };
        let steps =
            filtered.with_cursor(|cursor, rows, total| cursor.pick(position, range, rows, total));
        self.filtered_drawn(steps)
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
                // The keyboard's row, children left out: the message it
                // names, or the thread it is under.
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
                // The thread stays open if its row is where it was;
                // otherwise it folds with the list that held it.
                if filtered.expansion.as_ref().is_some_and(|expansion| {
                    filtered.index_of_row(expansion.anchor) != anchor_at || anchor_at.is_none()
                }) {
                    filtered.expansion = None;
                }
                let focused = match kept.and_then(|message| filtered.index_of_row(message)) {
                    // A child keeps its place under a row that did not move.
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
