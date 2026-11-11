//! Walking a thread in the message window (spec 013 US3, research R8).
//!
//! The window shows one message at a time; the controller holds the thread
//! it belongs to, so `[` `]` `{` `}` and `g u` are decided here and both
//! frontends only draw. A walk is made when a message opens, from the thread
//! read that opened it (or one asked for), and ends with the window.
//! Showing a message by stepping marks it read (FR-017); the message a
//! window opens on is marked by the frontend's dwell, as opening always was.

use chrono::Local;
use postio_core::{Command, CommandId};
use postio_model::{MessageId, ThreadId, ThreadView};
use postio_ui::thread_bar;

use crate::cursor::Rows;
use crate::feed::Step;
use crate::surfaces::{Host, Origin};
use crate::{Effect, FocusController, Intent, Request, SurfaceKind};

/// How many chips the bar is composed for until the window says (a 72-point
/// chip each, in the narrowest window).
pub(crate) const DEFAULT_ROOM: u32 = 8;

/// The thread of the message on screen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Walk {
    /// What the last read of the thread said, with what was read since.
    pub(crate) view: ThreadView,
    /// The message on screen.
    pub(crate) shown: MessageId,
    /// The list row the window was opened from: how the cursor's row is
    /// known not to be a different message.
    pub(crate) row: MessageId,
    /// The `(index, total)` the window was opened at in its list.
    pub(crate) place: (u32, u32),
}

impl Walk {
    /// Where the message on screen is among the members.
    fn at(&self) -> Option<usize> {
        self.view
            .members
            .iter()
            .position(|member| member.id == self.shown)
    }
}

/// A window opened, and the thread read it waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Opening {
    pub(crate) thread: ThreadId,
    pub(crate) shown: MessageId,
    pub(crate) row: MessageId,
    pub(crate) place: (u32, u32),
}

impl FocusController {
    /// A message was shown in the window: begin its walk. `effects` are what
    /// the input just produced, `rows` the list with any expansion's children
    /// among them. Steps asking for the thread read when it is not in hand.
    pub(crate) fn walk_after(&mut self, effects: &[Effect], rows: &dyn Rows) -> Vec<Step> {
        if !self.surfaces.has(SurfaceKind::Message) {
            self.walk = None;
            self.walk_asked = None;
            self.walk_seed = None;
            return Vec::new();
        }
        let opened = effects.iter().rev().find_map(|effect| match effect {
            Effect::Show(Intent::OpenMessage {
                message,
                index,
                total,
                host,
            }) => Some((*message, (*index, *total), *host)),
            _ => None,
        });
        let Some((message, place, host)) = opened else {
            return Vec::new();
        };
        let seed = self.walk_seed.take();
        // The step that put this message up is its own walk's.
        if self.walk.as_ref().is_some_and(|walk| walk.shown == message) {
            return Vec::new();
        }
        self.walk = None;
        self.walk_asked = None;
        if host != Host::Own || self.surfaces.origin() != Origin::List {
            return Vec::new();
        }
        // The row the cursor is on, when it is the message shown (or the
        // row a thread opened it from), says which thread it is of.
        let facts = self
            .cursor
            .position()
            .and_then(|position| rows.facts(position))
            .filter(|facts| facts.id == message || self.opened_for(facts.id, Some(message)))
            .or_else(|| self.hit_facts(message));
        let Some(facts) = facts else {
            return Vec::new();
        };
        let Some(&thread) = facts.threads.first() else {
            return Vec::new();
        };
        match seed.filter(|view| view.thread == thread) {
            Some(view) => self.begin_walk(
                Opening {
                    thread,
                    shown: message,
                    row: facts.id,
                    place,
                },
                view,
            ),
            None => {
                self.walk_asked = Some(Opening {
                    thread,
                    shown: message,
                    row: facts.id,
                    place,
                });
                vec![Step::Ask(Request::ReadThread(thread))]
            }
        }
    }

    /// A thread was read: the walk waiting for it begins.
    pub(crate) fn walk_read(&mut self, thread: ThreadId, view: &ThreadView) -> Vec<Step> {
        match self.walk_asked.filter(|asked| asked.thread == thread) {
            Some(opening) if self.surfaces.reading() == Some(opening.shown) => {
                self.walk_asked = None;
                self.begin_walk(opening, view.clone())
            }
            _ => Vec::new(),
        }
    }

    fn begin_walk(&mut self, opening: Opening, view: ThreadView) -> Vec<Step> {
        let walk = Walk {
            view,
            shown: opening.shown,
            row: opening.row,
            place: opening.place,
        };
        if walk.at().is_none() {
            return Vec::new();
        }
        self.walk = Some(walk);
        self.walk_intents()
    }

    /// The bar, kicker, banner and Reply to row of the message on screen.
    fn walk_intents(&self) -> Vec<Step> {
        let Some(walk) = &self.walk else {
            return Vec::new();
        };
        let Some(at) = walk.at() else {
            return Vec::new();
        };
        let many = walk.view.members.len() > 1;
        let keymap = self.keymap();
        let latest = at + 1 == walk.view.members.len();
        vec![
            Step::Show(Intent::ThreadBar(self.walk_bar(walk, at))),
            Step::Show(Intent::Kicker(if many {
                thread_bar::kicker(at, walk.view.members.len(), keymap)
            } else {
                String::new()
            })),
            Step::Show(Intent::OlderBanner(
                (many && !latest).then(|| thread_bar::older_banner(keymap)),
            )),
            Step::Show(Intent::ReplyTo(thread_bar::reply_to_row(&walk.view, at))),
        ]
    }

    fn walk_bar(&self, walk: &Walk, at: usize) -> thread_bar::ThreadBarView {
        let now = self.pickers.clock.unwrap_or_else(Local::now);
        thread_bar::bar(&walk.view, at, &[], self.bar_room, now)
    }

    /// The window's width allows `room` chips: draw the bar again.
    pub(crate) fn bar_room(&mut self, room: u32) -> Vec<Step> {
        self.bar_room = room;
        let Some(walk) = &self.walk else {
            return Vec::new();
        };
        let Some(at) = walk.at() else {
            return Vec::new();
        };
        vec![Step::Show(Intent::ThreadBar(self.walk_bar(walk, at)))]
    }

    /// A walking key in the message window, or `None` when there is no walk
    /// (the frontend's own stepping is still its own then).
    pub(crate) fn walk_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let walk = self.walk.as_ref()?;
        let at = walk.at()?;
        let last = walk.view.members.len().checked_sub(1)?;
        let target = match id {
            CommandId::NextInConversation => at.checked_add(1).filter(|next| *next <= last),
            CommandId::PrevInConversation => at.checked_sub(1),
            CommandId::FirstInConversation => Some(0),
            CommandId::LatestInConversation => Some(last),
            CommandId::FirstUnreadInConversation => {
                walk.view.members.iter().position(|member| !member.seen)
            }
            _ => return None,
        };
        // Past an end, or already there, nothing moves.
        let Some(target) = target.filter(|target| *target != at) else {
            return Some(Vec::new());
        };
        Some(self.walk_show(target))
    }

    /// Show the member at `target`, marking it read.
    fn walk_show(&mut self, target: usize) -> Vec<Step> {
        let Some(walk) = self.walk.as_mut() else {
            return Vec::new();
        };
        let member = &mut walk.view.members[target];
        let (message, unread) = (member.id, !member.seen);
        member.seen = true;
        walk.shown = message;
        let (row, (index, total)) = (walk.row, walk.place);
        self.surfaces.show(message);
        // The row the window came from stays the cursor's: it is not a
        // different message to follow.
        self.opened_from = Some((row, message));
        let mut steps = vec![Step::Show(Intent::OpenMessage {
            message,
            index,
            total,
            host: Host::Own,
        })];
        if unread {
            steps.push(Step::Ask(Request::Post(Command::MarkReadOnDwell {
                message,
            })));
        }
        steps.extend(self.walk_intents());
        steps
    }
}
