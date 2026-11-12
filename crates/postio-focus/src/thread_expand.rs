//! A thread expanded in place (spec 013 US1, research R7).
//!
//! The list is paged and the frontend holds its rows, so the children of an
//! expanded thread are only a [`Splice`] over them: `children` display
//! positions that follow the anchor row. The controller reads the frontend's
//! rows through [`Through`], which makes the children part of the list, so
//! the cursor, `j`/`k`, `G` and the selection walk them with no code of
//! their own. **Everything the controller says about positions is a display
//! position**, children counted; the rows the frontend hands in are row
//! positions, and an [`Intent::Splice`] names its anchor by row.

use chrono::Local;
use postio_core::{CommandId, Event};
use postio_model::{MessageId, ThreadId, ThreadView};
use postio_ui::focus_row::open_at;
use postio_ui::scope::{self, FooterAbout, FooterView, ScopeFocus};
use postio_ui::thread_list::{self, ChildRowView, Slot, Splice};

use crate::cursor::{RowFacts, Rows};
use crate::feed::Step;
use crate::{Effect, FocusController, Input, Intent, Reply, Request};

/// The one expanded thread of a list (at most one per list surface).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Expansion {
    pub(crate) thread: ThreadId,
    /// The expanded row's message: how the row is found again.
    pub(crate) anchor: MessageId,
    /// What the last read of the thread said.
    pub(crate) view: ThreadView,
    /// The child the cursor was on when the thread last changed.
    pub(crate) focused: Option<MessageId>,
    /// Where the anchor row was when the children were last drawn.
    pub(crate) at: u32,
}

impl Expansion {
    /// How many children follow the anchor: the members and the draft.
    pub(crate) fn children(&self) -> u32 {
        count(&self.view)
    }

    pub(crate) fn splice(&self, at: u32) -> Splice {
        Splice {
            anchor: at,
            children: self.children(),
        }
    }

    /// `rows` with this expansion's children among them, when its anchor
    /// is among them.
    pub(crate) fn through<'a>(&self, rows: &'a dyn Rows, by_thread: bool) -> Option<Through<'a>> {
        let at = rows.position_of(self.anchor)?;
        Some(Through {
            inner: rows,
            splice: self.splice(at),
            thread: self.thread,
            members: self.view.members.iter().map(|m| m.id).collect(),
            by_thread,
        })
    }
}

fn count(view: &ThreadView) -> u32 {
    (view.members.len() + usize::from(view.draft.is_some())) as u32
}

/// A read in flight that something waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pending {
    /// `Right` on a row: splice its children when the thread is read.
    Expand { thread: ThreadId, anchor: MessageId },
    /// `Return` on a row: open the message `open_at` names when the thread
    /// is read.
    Open { thread: ThreadId, row: MessageId },
}

impl Pending {
    pub(crate) fn thread(self) -> ThreadId {
        match self {
            Pending::Expand { thread, .. } | Pending::Open { thread, .. } => thread,
        }
    }
}

/// What the footer says about the row or child at display `position` of
/// `raw`, the frontend's rows (spec 013 US2): the thread's subject and size
/// on a row, the message's place and sender on a child.
pub(crate) fn footer_at(
    expansion: Option<&Expansion>,
    position: u32,
    raw: &dyn Rows,
    keymap: &postio_core::Keymap,
) -> Option<FooterView> {
    let splice = expansion.and_then(|x| Some(x.splice(raw.position_of(x.anchor)?)));
    match thread_list::slot(splice, position) {
        Slot::Row(row) => {
            let facts = raw.facts(row).filter(|facts| !facts.digest)?;
            let thread = facts.threads.first().copied().unwrap_or(ThreadId::new(0));
            let summary = raw.row(row)?;
            let summary = &summary.as_conversation()?.summary;
            let subject = summary
                .representative
                .subject
                .as_deref()
                .unwrap_or(NO_SUBJECT);
            let about = FooterAbout {
                subject,
                count: summary.message_count as usize,
                index: 0,
                sender: "",
            };
            let focus = ScopeFocus::Row {
                thread,
                latest: facts.id,
            };
            Some(scope::footer(focus, &about, keymap))
        }
        Slot::Child { index, .. } => {
            let view = &expansion?.view;
            let subject = view.subject.as_deref().unwrap_or(NO_SUBJECT);
            let Some(member) = view.members.get(index as usize) else {
                // The unsent reply: it has no message to act on.
                let mut footer = FooterView {
                    title: format!("Draft \u{b7} you \u{b7} {subject}"),
                    ..FooterView::default()
                };
                footer.keys.extend(
                    postio_ui::hints::pair(
                        keymap,
                        CommandId::NextMessage,
                        CommandId::PrevMessage,
                        "move",
                    )
                    .map(|hint| (postio_ui::hints::short(&hint.key), hint.label)),
                );
                return Some(footer);
            };
            let sender = match (&member.from, member.own) {
                (_, true) => "You",
                (Some(from), false) => from.display(),
                (None, false) => "Unknown sender",
            };
            let about = FooterAbout {
                subject,
                count: view.members.len(),
                index: index as usize,
                sender,
            };
            let focus = ScopeFocus::Child {
                thread: view.thread,
                message: member.id,
            };
            Some(scope::footer(focus, &about, keymap))
        }
    }
}

/// What a thread without a subject is called in the footer.
const NO_SUBJECT: &str = "(no subject)";

/// The frontend's rows with an expanded thread's children among them.
pub(crate) struct Through<'a> {
    inner: &'a dyn Rows,
    splice: Splice,
    thread: ThreadId,
    members: Vec<MessageId>,
    /// Whether a child's facts name its thread, so a thread verb aims the
    /// whole conversation (spec 013 D2, through `postio_ui::scope::target`).
    by_thread: bool,
}

impl Rows for Through<'_> {
    fn len(&self) -> u32 {
        self.splice.len(self.inner.len())
    }

    fn facts(&self, position: u32) -> Option<RowFacts> {
        match self.splice.slot(position) {
            Slot::Row(row) => self.inner.facts(row),
            // The draft has no message: nothing to open or select.
            Slot::Child { index, .. } => {
                let id = *self.members.get(index as usize)?;
                Some(RowFacts {
                    id,
                    digest: false,
                    threads: if self.by_thread {
                        vec![self.thread]
                    } else {
                        Vec::new()
                    },
                    writes: false,
                })
            }
        }
    }

    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.inner
            .position_of(message)
            .map(|row| self.splice.display(Slot::Row(row)))
            .or_else(|| {
                let index = self.members.iter().position(|id| *id == message)?;
                Some(self.splice.display(Slot::Child {
                    row: self.splice.anchor,
                    index: index as u32,
                }))
            })
    }

    fn is_child(&self, position: u32) -> bool {
        matches!(self.splice.slot(position), Slot::Child { .. })
    }

    fn said(&self, position: u32) -> Option<(String, String)> {
        match self.splice.slot(position) {
            Slot::Row(row) => self.inner.said(row),
            Slot::Child { .. } => None,
        }
    }

    fn row(&self, position: u32) -> Option<postio_ui::focus_list::FocusRow> {
        match self.splice.slot(position) {
            Slot::Row(row) => self.inner.row(row),
            Slot::Child { .. } => None,
        }
    }
}

impl FocusController {
    /// `rows` with the expansion's children among them, or `rows` as they
    /// are when nothing is expanded (or its anchor is not in a landed page).
    pub(crate) fn through<'a>(&self, rows: &'a dyn Rows) -> Option<Through<'a>> {
        self.expansion.as_ref()?.through(rows, true)
    }

    /// The expansion's splice over `rows`, when its anchor is among them.
    fn splice_over(&self, rows: &dyn Rows) -> Option<Splice> {
        let expansion = self.expansion.as_ref()?;
        Some(expansion.splice(rows.position_of(expansion.anchor)?))
    }

    /// The footer for the cursor's last move in `effects`, if it moved:
    /// the list's `Cursor` over its rows, Filtered's focus over its own.
    pub(crate) fn footer_after(&self, effects: &[Effect], raw: &dyn Rows) -> Option<FooterView> {
        let keymap = self.keymap();
        effects.iter().rev().find_map(|effect| match effect {
            Effect::Show(Intent::Cursor { position, .. }) => {
                footer_at(self.expansion.as_ref(), *position, raw, keymap)
            }
            Effect::Show(Intent::FilteredFocus(Some(position))) => {
                self.filtered.as_ref()?.footer_at(*position, keymap)
            }
            _ => None,
        })
    }

    /// The cursor's row position, and whether it is on a child, over the
    /// raw `rows`.
    fn cursor_row(&self, rows: &dyn Rows) -> Option<(u32, Option<u32>)> {
        let position = self.cursor.position()?;
        Some(match thread_list::slot(self.splice_over(rows), position) {
            Slot::Row(row) => (row, None),
            Slot::Child { row, index } => (row, Some(index)),
        })
    }

    /// The cursor's `(index, total)` among the messages, children counted
    /// as their row.
    pub(crate) fn place_of(&self, position: u32, rows: &dyn Rows) -> (u32, u32) {
        let Some(expansion) = &self.expansion else {
            return self.feed.message_place(position, rows.len());
        };
        let Some(anchor) = rows.position_of(expansion.anchor) else {
            return self.feed.message_place(position, rows.len());
        };
        let splice = expansion.splice(anchor);
        let row = match splice.slot(position) {
            Slot::Row(row) | Slot::Child { row, .. } => row,
        };
        self.feed
            .message_place(row, rows.len().saturating_sub(splice.children))
    }

    /// The cursor moved to `position`, which is drawn.
    fn put_cursor(&mut self, position: u32) -> Step {
        self.cursor.moved_to(Some(position));
        Step::Show(Intent::Cursor {
            position,
            to_top: position == 0,
        })
    }

    /// The children of `view`, as the live keymap and clock say them.
    pub(crate) fn child_views(&self, view: &ThreadView) -> Vec<ChildRowView> {
        let now = self.pickers.clock.unwrap_or_else(Local::now);
        let keymap = self.keymap();
        (0..view.members.len())
            .filter_map(|index| ChildRowView::member(view, index, &[], keymap, now))
            .chain(ChildRowView::draft(view, now))
            .collect()
    }

    /// Whether the row at `position` holds more than one message, as far as
    /// the frontend can say.
    fn is_thread(rows: &dyn Rows, position: u32) -> Option<bool> {
        let row = rows.row(position)?;
        Some(
            row.as_conversation()
                .is_some_and(|row| row.summary.message_count > 1),
        )
    }

    /// The inputs an expansion owns, or `None` when it has no say. `raw` is
    /// the frontend's rows, `rows` those with the children among them.
    pub(crate) fn thread_input(
        &mut self,
        input: &Input,
        raw: &dyn Rows,
        rows: &dyn Rows,
    ) -> Option<Vec<Step>> {
        match input {
            Input::Command(id @ (CommandId::ExpandThread | CommandId::CollapseThread))
                if self.surfaces.top().is_none()
                    && self.results.is_none()
                    && self.filtered.is_none() =>
            {
                Some(if *id == CommandId::ExpandThread {
                    self.expand(raw)
                } else {
                    self.collapse(raw)
                })
            }
            Input::Command(CommandId::OpenMessage)
                if self.surfaces.top().is_none()
                    && self.results.is_none()
                    && self.filtered.is_none()
                    && self.shown_apart(rows).is_none() =>
            {
                self.open_thread_row(raw, rows)
            }
            Input::Reply(ticket, Reply::ThreadRead { thread, view }) => {
                Some(self.thread_replied(*ticket, *thread, view, raw, rows))
            }
            Input::Event(Event::ThreadChanged { thread, .. }) => {
                let expanded = self
                    .expansion
                    .as_ref()
                    .is_some_and(|expansion| expansion.thread == *thread);
                // The event still goes on to the rest of the controller.
                expanded.then(|| vec![Step::Ask(Request::ReadThread(*thread))])
            }
            _ => None,
        }
    }

    /// The answer to a [`Request::ReadThread`].
    pub(crate) fn thread_replied(
        &mut self,
        ticket: crate::Ticket,
        thread: ThreadId,
        view: &Result<Option<ThreadView>, String>,
        raw: &dyn Rows,
        rows: &dyn Rows,
    ) -> Vec<Step> {
        if ticket.generation != self.generation {
            return Vec::new();
        }
        let view = match view {
            Ok(view) => view.clone(),
            Err(error) => {
                tracing::warn!(%error, "Focus could not read a thread");
                self.pending = None;
                return Vec::new();
            }
        };
        // A window waiting for this thread begins its walk with it.
        let mut steps = match &view {
            Some(view) => self.walk_read(thread, view),
            None => Vec::new(),
        };
        steps.extend(self.thread_read(thread, view, raw, rows));
        steps
    }

    /// `Right`: read the thread the cursor's row stands for, and splice its
    /// children when it comes. A child, a digest, a row of one message, or
    /// a thread already open does nothing.
    pub(crate) fn expand(&mut self, raw: &dyn Rows) -> Vec<Step> {
        let Some((row, None)) = self.cursor_row(raw) else {
            return Vec::new();
        };
        let Some(facts) = raw
            .facts(row)
            .filter(|facts| !facts.digest && !facts.writes)
        else {
            return Vec::new();
        };
        let Some(&thread) = facts.threads.first() else {
            return Vec::new();
        };
        if self
            .expansion
            .as_ref()
            .is_some_and(|expansion| expansion.anchor == facts.id)
            || Self::is_thread(raw, row) == Some(false)
        {
            return Vec::new();
        }
        self.pending = Some(Pending::Expand {
            thread,
            anchor: facts.id,
        });
        vec![Step::Ask(Request::ReadThread(thread))]
    }

    /// `Left`: fold the open thread, from its row or from a child, and put
    /// the cursor on its row.
    pub(crate) fn collapse(&mut self, raw: &dyn Rows) -> Vec<Step> {
        let Some(expansion) = &self.expansion else {
            return Vec::new();
        };
        let Some((row, _)) = self.cursor_row(raw) else {
            return Vec::new();
        };
        if raw.facts(row).map(|facts| facts.id) != Some(expansion.anchor) {
            return Vec::new();
        }
        self.expansion = None;
        let mut steps = vec![Step::Show(Intent::Unsplice { anchor: row })];
        if self.cursor.position() != Some(row) {
            steps.push(self.put_cursor(row));
        }
        steps
    }

    /// The thread arrived: splice it, open it, or redraw it.
    fn thread_read(
        &mut self,
        thread: ThreadId,
        view: Option<ThreadView>,
        raw: &dyn Rows,
        rows: &dyn Rows,
    ) -> Vec<Step> {
        let Some(view) = view else {
            if self
                .pending
                .is_some_and(|pending| pending.thread() == thread)
            {
                self.pending = None;
            }
            return Vec::new();
        };
        match self.pending.filter(|pending| pending.thread() == thread) {
            Some(Pending::Expand { anchor, .. }) => {
                self.pending = None;
                self.expanded(thread, anchor, view, raw)
            }
            Some(Pending::Open { row, .. }) => {
                self.pending = None;
                let Some(target) = open_at(&view) else {
                    return Vec::new();
                };
                let Some(position) = rows.position_of(row) else {
                    return Vec::new();
                };
                // Only if the person is still on the row they pressed it on.
                if self.cursor.position() != Some(position) {
                    return Vec::new();
                }
                self.walk_seed = Some(view);
                self.open_target(position, target, rows)
            }
            None => self.reread(thread, view, raw),
        }
    }

    /// Splice `view`'s children under the row of `anchor`, folding the
    /// thread open before it.
    fn expanded(
        &mut self,
        thread: ThreadId,
        anchor: MessageId,
        view: ThreadView,
        raw: &dyn Rows,
    ) -> Vec<Step> {
        let Some(at) = raw.position_of(anchor) else {
            return Vec::new();
        };
        if view.members.len() < 2 {
            return Vec::new();
        }
        // The row the cursor is on, whether or not it is under a child.
        let kept = self.cursor_row(raw).map(|(row, _)| row);
        let mut steps = Vec::new();
        if let Some(old) = self.expansion.take() {
            let was = raw.position_of(old.anchor).unwrap_or(old.at);
            steps.push(Step::Show(Intent::Unsplice { anchor: was }));
        }
        steps.push(Step::Show(Intent::Splice {
            anchor: at,
            children: self.child_views(&view),
        }));
        self.expansion = Some(Expansion {
            thread,
            anchor,
            view,
            focused: None,
            at,
        });
        if let Some(row) = kept {
            let display = thread_list::display(self.splice_over(raw), Slot::Row(row));
            if self.cursor.position() != Some(display) {
                steps.push(self.put_cursor(display));
            }
        }
        steps
    }

    /// The expanded thread changed and was read again: redraw its children
    /// and keep the cursor on the same message.
    fn reread(&mut self, thread: ThreadId, view: ThreadView, raw: &dyn Rows) -> Vec<Step> {
        let Some(expansion) = self.expansion.as_ref().filter(|x| x.thread == thread) else {
            return Vec::new();
        };
        let Some(at) = raw.position_of(expansion.anchor) else {
            return Vec::new();
        };
        let position = self.cursor.position();
        // Where the cursor was among the old children, by what it named.
        let slot = position.map(|position| expansion.splice(at).slot(position));
        let old_members = expansion.view.members.len();
        let focused = match slot {
            Some(Slot::Child { index, .. }) => {
                expansion.view.members.get(index as usize).map(|m| m.id)
            }
            _ => None,
        };
        let on_draft =
            matches!(slot, Some(Slot::Child { index, .. }) if index as usize == old_members);
        let on_child = matches!(slot, Some(Slot::Child { .. }));
        if view.members.len() < 2 {
            self.expansion = None;
            let mut steps = vec![Step::Show(Intent::Unsplice { anchor: at })];
            if on_child {
                steps.push(self.put_cursor(at));
            }
            return steps;
        }
        let children = self.child_views(&view);
        let members = view.members.len() as u32;
        let next = Expansion {
            focused,
            at,
            view,
            ..self.expansion.take().expect("checked above")
        };
        let after = next.splice(at);
        let found = focused.and_then(|id| next.view.members.iter().position(|m| m.id == id));
        let child = |index: u32| after.display(Slot::Child { row: at, index });
        let target = match slot {
            Some(Slot::Child { .. }) => Some(match found {
                Some(index) => child(index as u32),
                None if on_draft && next.view.draft.is_some() => child(members),
                // A message that left: back to the thread's row.
                None => at,
            }),
            Some(Slot::Row(row)) => Some(after.display(Slot::Row(row))),
            None => None,
        };
        self.expansion = Some(next);
        let mut steps = vec![Step::Show(Intent::Splice {
            anchor: at,
            children,
        })];
        if let Some(target) = target
            && Some(target) != position
        {
            steps.push(self.put_cursor(target));
        }
        steps
    }

    /// The list landed: if the expanded row left it, fold the children and
    /// leave the cursor on what slid into its place; if it moved, draw the
    /// children where it went.
    pub(crate) fn expansion_followed(&mut self, raw: &dyn Rows) -> Vec<Step> {
        let Some(expansion) = &self.expansion else {
            return Vec::new();
        };
        let old = expansion.splice(expansion.at);
        let position = self.cursor.position();
        match raw.position_of(expansion.anchor) {
            None => {
                let at = expansion.at;
                self.expansion = None;
                let mut steps = vec![Step::Show(Intent::Unsplice { anchor: at })];
                if let Some(position) = position {
                    let next = if position < at {
                        position
                    } else if position <= at + old.children {
                        at
                    } else {
                        position - old.children
                    };
                    match raw.len().checked_sub(1) {
                        Some(last) => {
                            let next = next.min(last);
                            if next != position {
                                steps.push(self.put_cursor(next));
                            }
                        }
                        None => self.cursor.moved_to(None),
                    }
                }
                steps
            }
            Some(at) if at != expansion.at => {
                let old_at = expansion.at;
                let children = self.child_views(&expansion.view);
                let new = expansion.splice(at);
                let mut steps = vec![
                    Step::Show(Intent::Unsplice { anchor: old_at }),
                    Step::Show(Intent::Splice {
                        anchor: at,
                        children,
                    }),
                ];
                if let Some(position) = position {
                    let slot = match old.slot(position) {
                        Slot::Child { index, .. } => Slot::Child { row: at, index },
                        slot => slot,
                    };
                    // A row that was not the anchor's keeps its place among
                    // the rows; the anchor moved, so the rows around it may
                    // have too, and the frontend's landing says which.
                    let display = new.display(slot);
                    if display != position {
                        steps.push(self.put_cursor(display));
                    }
                }
                if let Some(expansion) = &mut self.expansion {
                    expansion.at = at;
                }
                steps
            }
            Some(_) => Vec::new(),
        }
    }

    /// `Return` on a row of a thread: open the message `open_at` names, the
    /// thread read first when it is not loaded. `None` when the cursor is
    /// not on such a row, and the row opens as it always did.
    fn open_thread_row(&mut self, raw: &dyn Rows, rows: &dyn Rows) -> Option<Vec<Step>> {
        let (row, None) = self.cursor_row(raw)? else {
            return None;
        };
        let facts = raw.facts(row)?;
        if facts.digest || facts.writes || Self::is_thread(raw, row) != Some(true) {
            return None;
        }
        let &thread = facts.threads.first()?;
        let position = self.cursor.position()?;
        match &self.expansion {
            Some(expansion) if expansion.anchor == facts.id => {
                let target = open_at(&expansion.view)?;
                self.walk_seed = Some(expansion.view.clone());
                Some(self.open_target(position, target, rows))
            }
            _ => {
                self.pending = Some(Pending::Open {
                    thread,
                    row: facts.id,
                });
                Some(vec![Step::Ask(Request::ReadThread(thread))])
            }
        }
    }

    /// Open `target`, a message of the thread the row at `position` stands
    /// for.
    pub(crate) fn open_target(
        &mut self,
        position: u32,
        target: MessageId,
        rows: &dyn Rows,
    ) -> Vec<Step> {
        let Some(mut row) = rows.facts(position) else {
            return Vec::new();
        };
        self.opened_from = Some((row.id, target));
        row.id = target;
        let (index, total) = self.place_of(position, rows);
        self.surfaces
            .open(&row, index, total, self.policy.caps.stacking)
    }

    /// Whether the open message is the one `row`'s thread opened at.
    pub(crate) fn opened_for(&self, row: MessageId, reading: Option<MessageId>) -> bool {
        reading.is_some() && self.opened_from == reading.map(|reading| (row, reading))
    }
}
