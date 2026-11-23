//! A thread expanded in place. The children are only a splice over the frontend's paged rows;
//! `Through` makes them part of the list so the cursor, `j`/`k`, `G` and the selection walk them.
//! Everything the controller says about positions is a display position, children counted.

use chrono::Local;
use postio_core::{CommandId, Event};
use postio_model::{MessageId, ThreadId, ThreadView};
use postio_ui::focus_list::FocusRow;
use postio_ui::focus_row::open_at;
use postio_ui::scope::{self, FooterAbout, FooterView, ScopeFocus};
use postio_ui::thread_list::{self, ChildRowView, Slot, Splice};

use crate::cursor::{RowFacts, Rows};
use crate::feed::Step;
use crate::{Effect, FocusController, Input, Intent, Reply, Request};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Expansion {
    pub(crate) thread: ThreadId,
    pub(crate) anchor: MessageId,
    pub(crate) view: ThreadView,
    pub(crate) focused: Option<MessageId>,
    pub(crate) at: u32,
    pub(crate) reads: u8,
    pub(crate) probe: Probe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Probe {
    #[default]
    Idle,
    Flying,
    Again,
}

impl Expansion {
    pub(crate) fn children(&self) -> u32 {
        count(&self.view)
    }

    pub(crate) fn settling(&self) -> bool {
        self.reads > 0 || self.probe != Probe::Idle
    }

    pub(crate) fn splice(&self, at: u32) -> Splice {
        Splice {
            anchor: at,
            children: self.children(),
        }
    }

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

fn locate(rows: &dyn Rows, anchor: MessageId, view: &ThreadView) -> Option<(u32, MessageId)> {
    if let Some(at) = rows.position_of(anchor) {
        return Some((at, anchor));
    }
    view.members
        .iter()
        .rev()
        .find_map(|member| Some((rows.position_of(member.id)?, member.id)))
}

fn page_absent(rows: &dyn Rows, at: u32) -> bool {
    rows.facts(at).is_none() && (rows.is_empty() || at < rows.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pending {
    Expand { thread: ThreadId, anchor: MessageId },
    Open { thread: ThreadId, row: MessageId },
}

impl Pending {
    pub(crate) fn thread(self) -> ThreadId {
        match self {
            Pending::Expand { thread, .. } | Pending::Open { thread, .. } => thread,
        }
    }
}

pub(crate) fn footer_at(
    expansion: Option<&Expansion>,
    position: u32,
    raw: &dyn Rows,
    keymap: &postio_core::Keymap,
) -> Option<FooterView> {
    let splice = expansion.and_then(|x| Some(x.splice(raw.position_of(x.anchor)?)));
    match thread_list::slot(splice, position) {
        Slot::Row(row) => {
            let facts = raw.facts(row)?;
            if facts.digest {
                // Always a footer on a digest: none would leave the row
                // before it up, with keys for a message the cursor left
                // (#2203). A row not read yet says nothing.
                return Some(match raw.row(row) {
                    Some(FocusRow::Digest(digest)) => {
                        scope::digest_footer(&digest.rule, digest.count as usize, keymap)
                    }
                    _ => FooterView::default(),
                });
            }
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
                expanded: expansion.is_some_and(|x| x.anchor == facts.id),
            };
            Some(scope::footer(focus, &about, keymap))
        }
        Slot::Child { index, .. } => {
            let view = &expansion?.view;
            let subject = view.subject.as_deref().unwrap_or(NO_SUBJECT);
            let Some(member) = view.members.get(index as usize) else {
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
            let sender = sender_of(member);
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

pub(crate) fn sender_of(member: &postio_model::ThreadMember) -> &str {
    match (&member.from, member.own) {
        (_, true) => "You",
        (Some(from), false) => from.display(),
        (None, false) => "Unknown sender",
    }
}

const NO_SUBJECT: &str = "(no subject)";

pub(crate) struct Through<'a> {
    inner: &'a dyn Rows,
    splice: Splice,
    thread: ThreadId,
    members: Vec<MessageId>,
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

    fn position_of_thread(&self, thread: ThreadId) -> Option<u32> {
        self.inner
            .position_of_thread(thread)
            .map(|row| self.splice.display(Slot::Row(row)))
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
    pub(crate) fn through<'a>(&self, rows: &'a dyn Rows) -> Option<Through<'a>> {
        self.expansion.as_ref()?.through(rows, true)
    }

    fn splice_over(&self, rows: &dyn Rows) -> Option<Splice> {
        let expansion = self.expansion.as_ref()?;
        Some(expansion.splice(rows.position_of(expansion.anchor)?))
    }

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

    fn cursor_row(&self, rows: &dyn Rows) -> Option<(u32, Option<u32>)> {
        let position = self.cursor.position()?;
        Some(match thread_list::slot(self.splice_over(rows), position) {
            Slot::Row(row) => (row, None),
            Slot::Child { row, index } => (row, Some(index)),
        })
    }

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

    fn put_cursor(&mut self, position: u32) -> Step {
        self.cursor.moved_to(Some(position));
        Step::Show(Intent::Cursor {
            position,
            to_top: position == 0,
        })
    }

    pub(crate) fn child_views(&self, view: &ThreadView) -> Vec<ChildRowView> {
        let now = self.pickers.clock.unwrap_or_else(Local::now);
        let keymap = self.keymap();
        (0..view.members.len())
            .filter_map(|index| ChildRowView::member(view, index, &[], keymap, now))
            .chain(ChildRowView::draft(view, now))
            .collect()
    }

    fn is_thread(rows: &dyn Rows, position: u32) -> Option<bool> {
        let row = rows.row(position)?;
        Some(
            row.as_conversation()
                .is_some_and(|row| row.summary.message_count > 1),
        )
    }

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
                self.open_draft_child(raw)
                    .or_else(|| self.open_thread_row(raw, rows))
            }
            Input::Reply(ticket, Reply::ThreadRead { thread, view }) => {
                Some(self.thread_replied(*ticket, *thread, view, raw, rows))
            }
            Input::Event(Event::ThreadChanged { thread, .. }) => {
                let expansion = self.expansion.as_mut()?;
                if expansion.thread == *thread {
                    expansion.reads = expansion.reads.saturating_add(1);
                    Some(vec![Step::Ask(Request::ReadThread(*thread))])
                } else {
                    // Another thread changed. It may have swallowed this
                    // one (a late parent links two), and only the store
                    // knows: the anchor is asked where it is now.
                    Some(self.probe_anchor())
                }
            }
            Input::Reply(_, Reply::ThreadOf { message, thread }) => {
                self.anchor_probed(*message, *thread, raw)
            }
            _ => None,
        }
    }

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
                if let Some(expansion) = self.expansion.as_mut() {
                    expansion.reads = 0;
                    expansion.probe = Probe::Idle;
                }
                return self.walk_unread(thread);
            }
        };
        let mut steps = self.result_thread_read(thread, view.as_ref());
        steps.extend(match &view {
            Some(view) => self.walk_read(thread, view),
            None => self.walk_gone(thread),
        });
        steps.extend(self.thread_read(thread, view, raw, rows));
        steps
    }

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
            if self
                .expansion
                .as_ref()
                .is_some_and(|expansion| expansion.thread == thread)
            {
                if let Some(expansion) = self.expansion.as_mut() {
                    expansion.reads = expansion.reads.saturating_sub(1);
                }
                return self.probe_anchor();
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
                if self.cursor.position() != Some(position) {
                    return Vec::new();
                }
                self.walk_seed = Some(view);
                self.open_target(position, target, rows)
            }
            None => self.reread(thread, view, raw),
        }
    }

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
            reads: 0,
            probe: Probe::Idle,
        });
        if let Some(row) = kept {
            let display = thread_list::display(self.splice_over(raw), Slot::Row(row));
            if self.cursor.position() != Some(display) {
                steps.push(self.put_cursor(display));
            }
        }
        steps
    }

    /// The row it hangs from may have become another message's (a reply is the thread's newest, a
    /// merge makes a new thread), and is found again among the new members.
    fn reread(&mut self, thread: ThreadId, view: ThreadView, raw: &dyn Rows) -> Vec<Step> {
        let Some(expansion) = self.expansion.as_mut().filter(|x| x.thread == thread) else {
            return Vec::new();
        };
        expansion.reads = expansion.reads.saturating_sub(1);
        let Some((at, anchor)) = locate(raw, expansion.anchor, &view) else {
            if page_absent(raw, expansion.at) {
                expansion.view = view;
                return Vec::new();
            }
            return self.fold_gone(raw);
        };
        let old_at = expansion.at;
        let position = self.cursor.position();
        let slot = position.map(|position| expansion.splice(old_at).slot(position));
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
        let mut steps = Vec::new();
        if at != old_at {
            steps.push(Step::Show(Intent::Unsplice { anchor: old_at }));
        }
        if view.members.len() < 2 {
            self.expansion = None;
            steps.push(Step::Show(Intent::Unsplice { anchor: at }));
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
            anchor,
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
                None => match slot {
                    Some(Slot::Child { index, .. }) if members > 0 => child(index.min(members - 1)),
                    _ => at,
                },
            }),
            Some(Slot::Row(row)) => Some(after.display(Slot::Row(row))),
            None => None,
        };
        self.expansion = Some(next);
        steps.push(Step::Show(Intent::Splice {
            anchor: at,
            children,
        }));
        if let Some(target) = target
            && Some(target) != position
        {
            steps.push(self.put_cursor(target));
        }
        steps
    }

    fn fold_gone(&mut self, raw: &dyn Rows) -> Vec<Step> {
        let Some(expansion) = &self.expansion else {
            return Vec::new();
        };
        let old = expansion.splice(expansion.at);
        let position = self.cursor.position();
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

    fn probe_anchor(&mut self) -> Vec<Step> {
        let Some(expansion) = self.expansion.as_mut() else {
            return Vec::new();
        };
        match expansion.probe {
            Probe::Idle => {
                expansion.probe = Probe::Flying;
                vec![Step::Ask(Request::ThreadOf(expansion.anchor))]
            }
            Probe::Flying | Probe::Again => {
                expansion.probe = Probe::Again;
                Vec::new()
            }
        }
    }

    fn anchor_probed(
        &mut self,
        message: MessageId,
        thread: Option<ThreadId>,
        raw: &dyn Rows,
    ) -> Option<Vec<Step>> {
        let expansion = self.expansion.as_mut()?;
        if expansion.anchor != message || expansion.probe == Probe::Idle {
            return None;
        }
        let again = expansion.probe == Probe::Again;
        expansion.probe = Probe::Idle;
        let mut steps = Vec::new();
        match thread {
            Some(thread) if thread != expansion.thread => {
                expansion.thread = thread;
                expansion.reads = expansion.reads.saturating_add(1);
                steps.push(Step::Ask(Request::ReadThread(thread)));
            }
            _ => steps.extend(self.expansion_followed(raw)),
        }
        if again {
            steps.extend(self.probe_anchor());
        }
        Some(steps)
    }

    /// A row that is not in the pages in hand is not a row that left (a refresh lets go of its
    /// pages for a moment), and one that a change in flight may yet explain is waited for.
    pub(crate) fn expansion_followed(&mut self, raw: &dyn Rows) -> Vec<Step> {
        let Some(expansion) = &self.expansion else {
            return Vec::new();
        };
        let found = match raw.position_of(expansion.anchor) {
            Some(at) => Some((at, expansion.anchor)),
            None => locate(raw, expansion.anchor, &expansion.view),
        };
        let Some((at, anchor)) = found else {
            if page_absent(raw, expansion.at) || expansion.settling() {
                return Vec::new();
            }
            return self.fold_gone(raw);
        };
        let old = expansion.splice(expansion.at);
        let position = self.cursor.position();
        if at == expansion.at {
            if let Some(expansion) = &mut self.expansion {
                expansion.anchor = anchor;
            }
            return Vec::new();
        }
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
            let display = new.display(slot);
            if display != position {
                steps.push(self.put_cursor(display));
            }
        }
        if let Some(expansion) = &mut self.expansion {
            expansion.at = at;
            expansion.anchor = anchor;
        }
        steps
    }

    fn open_draft_child(&mut self, raw: &dyn Rows) -> Option<Vec<Step>> {
        let expansion = self.expansion.as_ref()?;
        let (_, Some(index)) = self.cursor_row(raw)? else {
            return None;
        };
        if index as usize != expansion.view.members.len() {
            return None;
        }
        let draft = expansion.view.draft.as_ref()?.id;
        Some(self.write(crate::ComposerKind::ThreadDraft { draft }, None))
    }

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

    pub(crate) fn opened_for(&self, row: MessageId, reading: Option<MessageId>) -> bool {
        reading.is_some() && self.opened_from == reading.map(|reading| (row, reading))
    }
}
