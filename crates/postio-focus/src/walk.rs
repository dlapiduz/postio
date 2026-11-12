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
use crate::surfaces::{Host, Origin, ReaderVerb};
use crate::{Effect, FocusController, Intent, Request, SurfaceKind, ToastKind};

/// How many chips the bar is composed for until the window says (a 72-point
/// chip each, in the narrowest window).
pub(crate) const DEFAULT_ROOM: u32 = 8;

/// The matches of the search a window was opened from (spec 013 US6): the
/// messages of the thread that matched, in date order, and which one the
/// window opened on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ThreadMatches {
    /// The search.
    pub(crate) query: postio_search::ParsedQuery,
    /// The matching messages, oldest first.
    pub(crate) messages: Vec<MessageId>,
    /// The one the window last showed, an index into `messages`.
    pub(crate) at: usize,
}

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
    /// Replies that arrived while the window was open and have not been
    /// shown: the banner names the latest (FR-018).
    pub(crate) pending_new: Vec<MessageId>,
    /// Messages deleted from the window: a read that still lists them (the
    /// store has not caught up) does not bring them back.
    pub(crate) gone: Vec<MessageId>,
    /// The search's matches in this thread, when the window was opened on
    /// one of them.
    pub(crate) matches: Option<ThreadMatches>,
    /// The message on screen was missing from a re-read: where it went is
    /// being asked, and the thread as it was read waits for the answer.
    pub(crate) follow: Option<Follow>,
}

/// A re-read that no longer holds the message on screen, while its new
/// thread is looked for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Follow {
    /// The thread as the re-read said.
    pub(crate) view: ThreadView,
    /// The thread the message is in now, once known.
    pub(crate) to: Option<ThreadId>,
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
    /// The shown message has no more of its own find matches in this
    /// direction (spec 013 US6): on to the next matched message, or say it
    /// is the last.
    pub(crate) fn match_edge(&mut self, forward: bool) -> Vec<Step> {
        let Some(walk) = self.walk.as_ref() else {
            return Vec::new();
        };
        let (Some(matches), Some(shown)) = (walk.matches.as_ref(), walk.at()) else {
            return Vec::new();
        };
        // Where the matched messages are among the members, whichever one
        // is on screen now.
        let places: Vec<usize> = walk
            .view
            .members
            .iter()
            .enumerate()
            .filter(|(_, member)| matches.messages.contains(&member.id))
            .map(|(at, _)| at)
            .collect();
        let target = if forward {
            places.iter().copied().find(|at| *at > shown)
        } else {
            places.iter().copied().rev().find(|at| *at < shown)
        };
        match target {
            Some(target) => self.walk_show(target),
            None => vec![Step::Show(Intent::Toast {
                text: thread_bar::no_more_matches(forward).to_owned(),
                kind: ToastKind::Notice,
            })],
        }
    }

    /// Whether `message` is the one a walk of the open thread is showing:
    /// an `OpenMessage` for it is a step in place, and the frontend does not
    /// begin the window again.
    pub fn walking(&self, message: MessageId, row: Option<MessageId>) -> bool {
        self.walk
            .as_ref()
            .is_some_and(|walk| walk.shown == message && Some(walk.row) == row)
    }

    /// A message was shown in the window: begin its walk. `effects` are what
    /// the input just produced, `rows` the list with any expansion's children
    /// among them. Steps asking for the thread read when it is not in hand.
    pub(crate) fn walk_after(&mut self, effects: &[Effect], rows: &dyn Rows) -> Vec<Step> {
        if !self.surfaces.has(SurfaceKind::Message) {
            let from_a_match = self.walk.take().is_some_and(|walk| walk.matches.is_some());
            self.walk_asked = None;
            self.walk_seed = None;
            // The window is gone, and its bar with it.
            self.bar_up = false;
            // A window opened on a match is closed: the ring goes back to
            // the child it came from.
            return if from_a_match {
                self.results_returned()
            } else {
                Vec::new()
            };
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
            return self.walk_unbarred();
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
            return self.walk_unbarred();
        };
        let Some(&thread) = facts.threads.first() else {
            return self.walk_unbarred();
        };
        // A conversation of one message has no thread to walk: nothing is
        // asked for it.
        if self
            .cursor
            .position()
            .and_then(|position| rows.row(position))
            .filter(|row| row.id() == facts.id)
            .and_then(|row| row.as_conversation().map(|row| row.summary.message_count))
            .is_some_and(|count| count <= 1)
            && seed.is_none()
        {
            return self.walk_unbarred();
        }
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
            _ if self.walk.as_ref().is_some_and(|walk| {
                walk.view.thread == thread
                    || walk
                        .follow
                        .as_ref()
                        .is_some_and(|follow| follow.to == Some(thread))
            }) =>
            {
                self.walk_refresh(view)
            }
            _ => Vec::new(),
        }
    }

    /// The bar, kicker and banners of a walk that is over, taken away. Said
    /// once, and only when one is up: a window whose walk never began has
    /// nothing to clear (spec 013 T085).
    pub(crate) fn walk_unbarred(&mut self) -> Vec<Step> {
        if !std::mem::take(&mut self.bar_up) {
            return Vec::new();
        }
        vec![
            Step::Show(Intent::ThreadBar(thread_bar::ThreadBarView::empty())),
            Step::Show(Intent::Kicker(String::new())),
            Step::Show(Intent::OlderBanner(None)),
            Step::Show(Intent::NewReply(None)),
            Step::Show(Intent::ReplyTo(None)),
            Step::Show(Intent::MatchBanner(None)),
        ]
    }

    /// The thread a window waits on could not be read: its walk will not
    /// begin, and the bar of the window before it must not stay.
    pub(crate) fn walk_unread(&mut self, thread: ThreadId) -> Vec<Step> {
        if self.walk_asked.is_some_and(|asked| asked.thread == thread) {
            self.walk_asked = None;
            return self.walk_unbarred();
        }
        Vec::new()
    }

    /// A thread was read and is gone: the window walking it has nothing left
    /// to show.
    pub(crate) fn walk_gone(&mut self, thread: ThreadId) -> Vec<Step> {
        let unread = self.walk_unread(thread);
        let Some(walk) = self.walk.as_mut() else {
            return unread;
        };
        // The thread the window was sent to is gone as well.
        let sent = walk
            .follow
            .as_ref()
            .is_some_and(|follow| follow.to == Some(thread));
        if sent {
            return self.walk_close();
        }
        if walk.view.thread != thread {
            return Vec::new();
        }
        // The thread may be gone because its last message moved (an undone
        // split): ask where the one on screen is before closing.
        let shown = walk.shown;
        if walk.follow.is_some() || walk.gone.contains(&shown) {
            return self.walk_close();
        }
        let mut view = walk.view.clone();
        view.members.clear();
        walk.follow = Some(Follow { view, to: None });
        vec![Step::Ask(Request::ThreadOf(shown))]
    }

    /// A thread the open window walks changed: its arrivals are pending
    /// until shown, and it is read again. The message on screen never
    /// changes here (FR-018).
    pub(crate) fn walk_heard(&mut self, event: &postio_core::Event) -> Vec<Step> {
        let walk = self.walk.as_mut();
        match event {
            postio_core::Event::ThreadChanged {
                thread,
                members_added,
                ..
            } => {
                let Some(walk) = walk.filter(|walk| walk.view.thread == *thread) else {
                    return Vec::new();
                };
                for added in members_added {
                    if *added != walk.shown && !walk.pending_new.contains(added) {
                        walk.pending_new.push(*added);
                    }
                }
                vec![Step::Ask(Request::ReadThread(*thread))]
            }
            // A message of the thread left a folder: it may have left the
            // thread (deleted elsewhere), so the thread is read again. One
            // that is only filed elsewhere is still a member, and nothing
            // moves.
            postio_core::Event::MessagesRemoved { messages, .. } => {
                let Some(walk) = walk.filter(|walk| {
                    walk.view
                        .members
                        .iter()
                        .any(|member| messages.contains(&member.id))
                }) else {
                    return Vec::new();
                };
                vec![Step::Ask(Request::ReadThread(walk.view.thread))]
            }
            _ => Vec::new(),
        }
    }

    /// The open walk's thread, read again: the bar, kicker and banners are
    /// drawn from it, with `shown` where it was.
    fn walk_refresh(&mut self, view: &ThreadView) -> Vec<Step> {
        let Some(walk) = self.walk.as_mut() else {
            return Vec::new();
        };
        let mut view = view.clone();
        // What was shown since stays read, whether or not the store has
        // caught up.
        for member in &mut view.members {
            if let Some(old) = walk.view.members.iter().find(|old| old.id == member.id) {
                member.seen |= old.seen;
            }
        }
        view.members
            .retain(|member| !walk.gone.contains(&member.id));
        if !view.members.iter().any(|member| member.id == walk.shown) {
            // Not deleted from here: ask where it went before saying it is
            // gone. One question at a time.
            if walk.gone.contains(&walk.shown) {
                return self.walk_displaced(view);
            }
            let shown = walk.shown;
            if walk.follow.is_none() {
                walk.follow = Some(Follow { view, to: None });
                return vec![Step::Ask(Request::ThreadOf(shown))];
            }
            return Vec::new();
        }
        walk.follow = None;
        walk.view = view;
        let members = &walk.view.members;
        walk.pending_new
            .retain(|id| members.iter().any(|member| member.id == *id));
        let mut steps = self.walk_intents();
        steps.push(Step::Show(Intent::NewReply(self.new_reply())));
        steps
    }

    /// Where the message on screen is now (spec 013 US5): in another thread
    /// (a split moved it, an undo brought it back) the walk goes there and
    /// nothing is said; in none (deleted) the window moves on as for any
    /// message gone.
    pub(crate) fn walk_followed(
        &mut self,
        message: MessageId,
        thread: Option<ThreadId>,
    ) -> Vec<Step> {
        let Some(walk) = self.walk.as_mut() else {
            return Vec::new();
        };
        let Some(follow) = walk.follow.as_mut().filter(|_| walk.shown == message) else {
            return Vec::new();
        };
        match thread.filter(|thread| *thread != walk.view.thread) {
            Some(thread) => {
                follow.to = Some(thread);
                vec![Step::Ask(Request::ReadThread(thread))]
            }
            None => {
                let view = walk.follow.take().expect("checked above").view;
                self.walk_displaced(view)
            }
        }
    }

    /// The message on screen is not in the thread any more (`view` is the
    /// thread as it is): show the nearest member that remains, the next
    /// newer one before the next older, and say so; with none left, the
    /// window closes (spec 013, edge cases).
    fn walk_displaced(&mut self, view: ThreadView) -> Vec<Step> {
        let Some(walk) = self.walk.as_mut() else {
            return Vec::new();
        };
        let kept = |id: MessageId| view.members.iter().any(|member| member.id == id);
        let at = walk.at();
        let old = &walk.view.members;
        let newer = at.and_then(|at| old[at + 1..].iter().find(|member| kept(member.id)));
        let older = at.and_then(|at| old[..at].iter().rev().find(|member| kept(member.id)));
        let (target, is_newer) = match (newer, older) {
            (Some(member), _) => (Some(member.id), true),
            (None, Some(member)) => (Some(member.id), false),
            // Nothing of the old thread is left; whatever it holds now.
            (None, None) => (view.members.last().map(|member| member.id), true),
        };
        let Some(target) = target else {
            return self.walk_close();
        };
        let index = view
            .members
            .iter()
            .position(|member| member.id == target)
            .expect("the target is a member");
        walk.view = view;
        walk.pending_new
            .retain(|id| walk.view.members.iter().any(|member| member.id == *id));
        let mut steps = self.walk_show(index);
        steps.push(Step::Show(Intent::Toast {
            text: thread_bar::gone_notice(is_newer).to_owned(),
            kind: ToastKind::Notice,
        }));
        steps
    }

    /// The thread has no message left to show: the window closes.
    fn walk_close(&mut self) -> Vec<Step> {
        self.walk = None;
        self.walk_asked = None;
        self.bar_up = false;
        self.surfaces.dismiss(SurfaceKind::Message);
        let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Message))];
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
    }

    /// The message on screen was deleted from the window: the window moves
    /// at once, as a deleted row does (local-first), and the thread is read
    /// again to confirm.
    pub(crate) fn walk_deleted_shown(&mut self) -> Vec<Step> {
        let Some(walk) = self.walk.as_mut() else {
            return Vec::new();
        };
        let (shown, thread) = (walk.shown, walk.view.thread);
        walk.gone.push(shown);
        let mut view = walk.view.clone();
        view.members.retain(|member| member.id != shown);
        let mut steps = self.walk_displaced(view);
        if self.walk.is_some() {
            steps.push(Step::Ask(Request::ReadThread(thread)));
        }
        steps
    }

    /// The banner for the latest reply not yet shown.
    fn new_reply(&self) -> Option<thread_bar::BannerView> {
        let walk = self.walk.as_ref()?;
        let latest = walk.pending_new.last()?;
        let member = walk
            .view
            .members
            .iter()
            .find(|member| member.id == *latest)?;
        Some(thread_bar::new_reply_banner(member, self.keymap()))
    }

    fn begin_walk(&mut self, opening: Opening, view: ThreadView) -> Vec<Step> {
        let walk = Walk {
            view,
            shown: opening.shown,
            row: opening.row,
            place: opening.place,
            pending_new: Vec::new(),
            gone: Vec::new(),
            follow: None,
            matches: self
                .results
                .as_mut()
                .and_then(|results| results.opening.take())
                .filter(|found| found.messages.get(found.at) == Some(&opening.shown)),
        };
        if walk.at().is_none() {
            // The thread does not hold the message on screen: no walk.
            return self.walk_unbarred();
        }
        self.walk = Some(walk);
        self.bar_up = true;
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
        let mut steps = vec![
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
        ];
        if let Some(banner) = self.match_banner(walk) {
            steps.push(Step::Show(Intent::MatchBanner(Some(banner))));
        }
        steps
    }

    /// The banner of a window opened from a search: where the shown match
    /// is among the thread's.
    fn match_banner(&self, walk: &Walk) -> Option<thread_bar::MatchBannerView> {
        let matches = walk.matches.as_ref()?;
        let numbers: Vec<u32> = matches
            .messages
            .iter()
            .filter_map(|id| walk.view.members.iter().position(|member| member.id == *id))
            .map(|at| at as u32 + 1)
            .collect();
        Some(thread_bar::match_banner(
            matches.query.input(),
            matches.at,
            &numbers,
            self.keymap(),
        ))
    }

    fn walk_bar(&self, walk: &Walk, at: usize) -> thread_bar::ThreadBarView {
        let now = self.pickers.clock.unwrap_or_else(Local::now);
        let matched: &[MessageId] = walk
            .matches
            .as_ref()
            .map_or(&[], |matches| matches.messages.as_slice());
        thread_bar::bar(&walk.view, at, matched, self.bar_room, now)
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
        // From a search, find first steps the message's own matches; the
        // frontend says when they run out.
        if walk.matches.is_some() {
            match id {
                CommandId::FindNext => {
                    return Some(vec![Step::Show(Intent::Reader(ReaderVerb::NextMatch))]);
                }
                CommandId::FindPrevious => {
                    return Some(vec![Step::Show(Intent::Reader(ReaderVerb::PrevMatch))]);
                }
                _ => {}
            }
        }
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

    /// A click on the chip of the member at `index`: shown as `]` and `[`
    /// would have shown it. The one on screen, a member that is not there,
    /// and a window with no walk do nothing.
    pub(crate) fn chip_clicked(&mut self, index: u32) -> Vec<Step> {
        let Some(walk) = self.walk.as_ref() else {
            return Vec::new();
        };
        let target = index as usize;
        if target >= walk.view.members.len() || walk.at() == Some(target) {
            return Vec::new();
        }
        self.walk_show(target)
    }

    /// Show the member at `target`, marking it read.
    pub(crate) fn walk_show(&mut self, target: usize) -> Vec<Step> {
        let Some(walk) = self.walk.as_mut() else {
            return Vec::new();
        };
        let member = &mut walk.view.members[target];
        let (message, unread) = (member.id, !member.seen);
        member.seen = true;
        walk.shown = message;
        if let Some(matches) = walk.matches.as_mut()
            && let Some(at) = matches.messages.iter().position(|id| *id == message)
        {
            matches.at = at;
        }
        let had_pending = !walk.pending_new.is_empty();
        if target + 1 == walk.view.members.len() {
            walk.pending_new.clear();
        } else {
            walk.pending_new.retain(|id| *id != message);
        }
        let banner = had_pending.then(|| self.new_reply());
        let Some(walk) = self.walk.as_ref() else {
            return Vec::new();
        };
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
        if let Some(banner) = banner {
            steps.push(Step::Show(Intent::NewReply(banner)));
        }
        steps
    }
}
