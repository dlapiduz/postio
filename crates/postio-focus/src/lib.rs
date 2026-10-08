//! Focus's behaviour, decided once (ADR 0045, `specs/009-focus-macos`).
//!
//! A frontend turns what happens to it — a key, an engine event, a client's
//! reply, a fact about its own window — into an [`Input`], hands it to the
//! [`FocusController`], and does what the returned [`Effect`]s say: draw an
//! [`Intent`], run a [`Request`] through [`perform()`] and feed the [`Reply`]
//! back, or set a timer. The controller does no I/O and never awaits, so the
//! GTK app can drive it from glib's main loop and the FFI from tokio, and
//! every rule in it is a unit test that runs on either host (research R1).
//!
//! Rules arrive here a slice at a time, moved out of `postio-gtk`'s window
//! (research R2); each slice brings its inputs, intents and requests with it.

use std::time::Duration;

use postio_client::protocol::FocusCounts;
use postio_config::paths::Platform;
use postio_core::state::Selection;
use postio_core::{Command, CommandId, Event, MessageTarget};
use postio_model::listing::{PageRequest, Surfaced};
use postio_model::{AccountId, ListScope, MailboxId, MessageId};
use postio_ui::focus_list::FocusRow;

mod cursor;
mod feed;
mod perform;
mod verbs;

pub use cursor::{NoRows, RowFacts, Rows};
pub use feed::{Opened, PageAnswer};
pub use perform::{perform, perform_now};
pub use verbs::{Everything, ToastKind};

/// What differs between platforms, as policy rather than as a fork
/// (ADR 0045 rule 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Which platform's layout and conventions apply.
    pub platform: Platform,
    /// What this frontend can draw.
    pub caps: Capabilities,
}

/// What a frontend can draw that another cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// A message can be read beside the list (spec 007 FR-038). Linux has
    /// it; the Mac comes to it after parity (spec 009 M4).
    pub reading_pane: bool,
    /// Secondary surfaces stack over each other: Linux's dialogs do, the
    /// Mac's windows replace each other (spec 009 M4).
    pub stacking: bool,
}

impl Policy {
    /// The policy each platform's Focus app runs with.
    pub const fn for_platform(platform: Platform) -> Self {
        let linux = matches!(platform, Platform::Freedesktop);
        Policy {
            platform,
            caps: Capabilities {
                reading_pane: linux,
                stacking: linux,
            },
        }
    }
}

/// One request in flight. The generation is the controller's when it asked;
/// a reply carrying an older one changes nothing (contract invariant 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticket {
    /// Unique among this controller's requests.
    pub id: u64,
    /// The controller's generation when it asked.
    pub generation: u64,
}

/// Something that happened, told to the controller.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Input {
    /// A command, from a key, a button, the bar or a menu.
    Command(CommandId),
    /// The answer to an earlier [`Effect::Ask`].
    Reply(Ticket, Reply),
    /// Something the engine said, from the client's one event stream.
    Event(Event),
    /// The accounts Focus's inbox is made of: what "select everything"
    /// reaches.
    Accounts(Vec<AccountId>),
    /// Whether the list stands scrolled to its very top: where it goes back
    /// to after an undo brings rows in above.
    AtTop(bool),
}

/// What the frontend does next.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Effect {
    /// Draw this.
    Show(Intent),
    /// Run this through [`perform()`] and hand the answer back as
    /// [`Input::Reply`] with the same ticket.
    Ask(Ticket, Request),
    /// Call back with [`Input`] once `after` has passed. (Arrives with the
    /// first slice that needs one.)
    Timer {
        /// Handed back when the timer fires.
        token: u64,
        /// How long to wait.
        after: Duration,
    },
}

/// What the frontend draws. Each one is drawable without asking the
/// controller anything else.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Intent {
    /// The header strip's counts changed.
    Counts(FocusCounts),
    /// The cursor is on `position`: draw it there and bring it into view.
    Cursor {
        /// The row.
        position: u32,
        /// Whether the list scrolls back to its very top: the first row.
        to_top: bool,
    },
    /// The selection changed: redraw the rows' boxes, and the bar's words.
    Selection {
        /// What is selected now.
        selection: Selection,
        /// "3 selected", or nothing with nothing selected.
        summary: Option<String>,
    },
    /// The list's one heading, while `!` narrows it ("Has action · 7"), or
    /// back to the day headings with `None`.
    SingleHeading(Option<String>),
    /// Scroll the list back to its very top.
    ListToTop,
    /// Say something in the toast.
    Toast {
        /// What to say: the host's words.
        text: String,
        /// How to draw it, and how long it stays.
        kind: ToastKind,
    },
    /// The place opened has been counted: the list changes over to it,
    /// keeping the rows on screen until its first page lands, and asks for
    /// that page ([`FocusController::page_wanted`]).
    ReplaceSource {
        /// How many rows the new place draws.
        total: u32,
    },
    /// A page has been asked for under the list's `stamp`.
    PagePending {
        /// The list's own stamp, echoed.
        stamp: u64,
        /// Which page.
        page: u32,
    },
    /// A page's rows, for the list to take if `stamp` is still its own.
    DeliverPage {
        /// The stamp the page was asked under.
        stamp: u64,
        /// Which page.
        page: u32,
        /// How many rows the place draws now.
        total: u32,
        /// The rows, conversations and surfaced rows in list order.
        rows: Vec<FocusRow>,
    },
    /// A page could not be read and is being asked for again.
    AbandonPage {
        /// The stamp it was asked under.
        stamp: u64,
        /// Which page.
        page: u32,
    },
    /// A page could not be read and will not be asked for again.
    GiveUp {
        /// The stamp it was asked under.
        stamp: u64,
        /// Which page.
        page: u32,
    },
    /// Re-read the pages on screen, keeping the scroll position.
    RefreshList,
    /// Rows landed: a first page, or a re-read.
    Filled,
    /// Leave the app.
    Quit,
}

/// What the controller needs from the engine. [`perform()`] is the one place
/// each becomes a client call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Request {
    /// The header strip's counts.
    FocusCounts,
    /// Open a place: which folders are inboxes, how many rows it holds, and
    /// -- when it `splices` -- what Focus surfaces among them.
    OpenScope {
        /// The place.
        scope: ListScope,
        /// Whether to read the surfaced rows too.
        splices: bool,
    },
    /// One page of the place in view, with its label pills.
    Page {
        /// Which page of the list.
        page: u32,
        /// The list's stamp, echoed in the answer.
        stamp: u64,
        /// The page's first position, in the list's terms.
        start: u32,
        /// How many positions the page spans.
        count: u32,
        /// The run of conversations to read, in the store's terms.
        wanted: PageRequest,
    },
    /// What Focus surfaces in its inbox, read again.
    Surfaced,
    /// Send a verb, aimed: at each of `aims` in turn, or -- with
    /// `everything` -- at a whole-view selection the host resolves.
    Send {
        /// The verb.
        command: Command,
        /// Where it goes, in the order to send it.
        aims: Vec<MessageTarget>,
        /// A whole-view selection, instead of `aims`.
        everything: Option<Everything>,
    },
    /// Send a command as it is: one that aims at nothing (`Undo`).
    Post(Command),
    /// Mail left `mailbox`: said to the store before the list re-reads.
    NoteRemoved {
        /// The folder it left.
        mailbox: MailboxId,
        /// What left.
        messages: Vec<MessageId>,
    },
}

/// The engine's answer to a [`Request`]. A failure is carried as its
/// sentence: the controller decides what to show, not how to recover.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Reply {
    /// The answer to [`Request::FocusCounts`].
    FocusCounts(Result<FocusCounts, String>),
    /// The answer to [`Request::OpenScope`].
    Opened(Opened),
    /// The answer to [`Request::Page`], with the request's own terms.
    Page {
        /// Which page.
        page: u32,
        /// The stamp it was asked under.
        stamp: u64,
        /// The page's first position.
        start: u32,
        /// How many positions it spans.
        count: u32,
        /// The rows, or why there are none.
        answer: Result<PageAnswer, String>,
    },
    /// The answer to [`Request::Surfaced`].
    Surfaced(Result<Vec<Surfaced>, String>),
    /// A post was made; nothing to answer.
    Noted,
    /// A command was sent, or why it could not be.
    Sent(Result<(), String>),
}

/// Focus's behaviour for one window. `Send`, and plain data: no toolkit
/// object, no executor, no client.
#[derive(Debug)]
pub struct FocusController {
    policy: Policy,
    generation: u64,
    next_ticket: u64,
    feed: feed::Feed,
    cursor: cursor::Cursor,
    verbs: verbs::Verbs,
    /// The strip's counts, as the host last said.
    counts: Option<FocusCounts>,
}

impl FocusController {
    /// A controller for one window, under its platform's policy.
    pub fn new(policy: Policy) -> Self {
        FocusController {
            policy,
            generation: 0,
            next_ticket: 0,
            feed: feed::Feed::default(),
            cursor: cursor::Cursor::default(),
            verbs: verbs::Verbs::default(),
            counts: None,
        }
    }

    /// The policy this controller was made with.
    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// Tell the controller what happened; get back what to do. For inputs
    /// that need no rows; [`handle_on`](Self::handle_on) for the rest.
    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        self.handle_on(input, &NoRows)
    }

    /// Tell the controller what happened, over the list as the frontend
    /// holds it; get back what to do.
    pub fn handle_on(&mut self, input: Input, rows: &dyn Rows) -> Vec<Effect> {
        match input {
            Input::Command(CommandId::Quit) => vec![Effect::Show(Intent::Quit)],
            // Back's last rung: the surfaces above the list are still the
            // frontend's to close first.
            Input::Command(CommandId::Back) => {
                let steps = self.cursor.clear(self.feed.total());
                self.effects(steps)
            }
            Input::Command(id) => {
                let has_action = self.counts.map(|counts| counts.has_action);
                let total = self.feed.total();
                let steps = self
                    .cursor
                    .command(id, rows, self.feed.scope(), total, has_action)
                    .or_else(|| self.verbs.command(id, &mut self.cursor, rows, total));
                steps.map(|steps| self.effects(steps)).unwrap_or_default()
            }
            Input::Accounts(accounts) => {
                self.cursor.set_accounts(accounts);
                Vec::new()
            }
            Input::AtTop(at_top) => {
                self.verbs.set_at_top(at_top);
                Vec::new()
            }
            Input::Reply(ticket, _) if ticket.generation != self.generation => Vec::new(),
            Input::Reply(_, Reply::FocusCounts(Ok(counts))) => {
                self.counts = Some(counts);
                vec![Effect::Show(Intent::Counts(counts))]
            }
            Input::Reply(_, Reply::FocusCounts(Err(error))) => {
                tracing::debug!(%error, "focus counts unavailable");
                Vec::new()
            }
            Input::Reply(_, Reply::Opened(opened)) => {
                let steps = self.feed.opened(opened);
                self.effects(steps)
            }
            Input::Reply(
                _,
                Reply::Page {
                    page,
                    stamp,
                    start,
                    count,
                    answer,
                },
            ) => {
                let steps = self.feed.page(page, stamp, start, count, answer);
                self.effects(steps)
            }
            Input::Reply(_, Reply::Surfaced(surfaced)) => {
                let steps = self.feed.resurfaced(surfaced);
                self.effects(steps)
            }
            Input::Reply(_, Reply::Noted) => Vec::new(),
            Input::Reply(_, Reply::Sent(result)) => {
                if let Err(error) = result {
                    tracing::warn!(%error, "Focus could not send a command");
                }
                Vec::new()
            }
            Input::Event(event) => {
                let mut steps = self.verbs.event(&event);
                steps.extend(self.feed.event(&event));
                self.effects(steps)
            }
        }
    }

    /// The list has landed or moved (the frontend's "filled"), `opened` when
    /// this is the first landing of a place just opened: the cursor goes to
    /// the first row, or back to the message `!` kept, and the strip's
    /// counts are asked for again.
    pub fn landed(&mut self, rows: &dyn Rows, opened: bool) -> Vec<Effect> {
        let mut steps = self.cursor.landed(rows, opened);
        steps.extend(self.verbs.landed(&mut self.cursor, rows));
        let mut effects = self.effects(steps);
        effects.push(self.refresh_counts());
        effects
    }

    /// The cursor's row, if it has one.
    pub fn cursor(&self) -> Option<u32> {
        self.cursor.position()
    }

    /// What is selected: what `a` would act on.
    pub fn selection(&self) -> Selection {
        self.cursor.selection()
    }

    /// Whether the list is narrowed to the rows with a marker (`!`).
    pub fn has_action(&self) -> bool {
        self.cursor.has_action()
    }

    /// The strip's counts, as the host last said.
    pub fn counts(&self) -> Option<FocusCounts> {
        self.counts
    }

    /// Show `scope` in the list: counted first, then the list changes over.
    pub fn open(&mut self, scope: ListScope) -> Vec<Effect> {
        let request = self.feed.open(scope);
        vec![Effect::Ask(self.ticket(), request)]
    }

    /// The list wants `page`, under its own `stamp` (GTK's model generation,
    /// the Mac's window generation): every answer for it carries the stamp
    /// back for the list to check.
    pub fn page_wanted(&mut self, page: u32, stamp: u64) -> Vec<Effect> {
        let steps = self.feed.wanted(page, stamp);
        self.effects(steps)
    }

    /// The place in view, once one is open.
    pub fn scope(&self) -> Option<ListScope> {
        self.feed.scope()
    }

    /// How many rows the place in view draws, as the store last said.
    pub fn list_total(&self) -> u32 {
        self.feed.total()
    }

    /// Every page asked of the store, in order.
    pub fn pages_asked(&self) -> &[u32] {
        self.feed.pages_asked()
    }

    /// Whether the first page of the place in view has landed.
    pub fn has_landed(&self) -> bool {
        self.feed.has_landed()
    }

    /// True once per opening, after its first page lands.
    pub fn take_opened(&mut self) -> bool {
        self.feed.take_opened()
    }

    /// Whether `mailbox` is one of the inboxes Focus's inbox is made of.
    pub fn is_inbox(&self, mailbox: MailboxId) -> bool {
        self.feed.is_inbox(mailbox)
    }

    /// The cursor's `index` among the messages of a list of `total` rows,
    /// and how many messages it holds, digests not counted.
    pub fn message_place(&self, index: u32, total: u32) -> (u32, u32) {
        self.feed.message_place(index, total)
    }

    fn effects(&mut self, steps: Vec<feed::Step>) -> Vec<Effect> {
        steps
            .into_iter()
            .map(|step| match step {
                feed::Step::Show(intent) => Effect::Show(intent),
                feed::Step::Ask(request) => Effect::Ask(self.ticket(), request),
                feed::Step::Open(scope) => {
                    let request = self.feed.open(scope);
                    Effect::Ask(self.ticket(), request)
                }
            })
            .collect()
    }

    /// Ask for the header strip's counts.
    pub fn refresh_counts(&mut self) -> Effect {
        Effect::Ask(self.ticket(), Request::FocusCounts)
    }

    /// Forget every request in flight: their replies, when they come, change
    /// nothing. Called when what they were for is gone, a place left or a
    /// list replaced.
    pub fn invalidate(&mut self) {
        self.generation += 1;
    }

    fn ticket(&mut self) -> Ticket {
        self.next_ticket += 1;
        Ticket {
            id: self.next_ticket,
            generation: self.generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(has_action: u32) -> FocusCounts {
        FocusCounts {
            has_action,
            ..FocusCounts::default()
        }
    }

    #[test]
    fn the_controller_can_cross_threads() {
        fn send<T: Send>() {}
        send::<FocusController>();
    }

    #[test]
    fn linux_stacks_and_reads_beside_and_the_mac_does_neither() {
        let linux = Policy::for_platform(Platform::Freedesktop);
        let mac = Policy::for_platform(Platform::Apple);
        assert!(linux.caps.stacking && linux.caps.reading_pane);
        assert!(!mac.caps.stacking && !mac.caps.reading_pane);
    }

    #[test]
    fn counts_asked_for_are_drawn_when_they_come() {
        for platform in [Platform::Freedesktop, Platform::Apple] {
            let mut focus = FocusController::new(Policy::for_platform(platform));
            let Effect::Ask(ticket, Request::FocusCounts) = focus.refresh_counts() else {
                panic!("refreshing asks for the counts");
            };
            assert_eq!(
                focus.handle(Input::Reply(ticket, Reply::FocusCounts(Ok(counts(7))))),
                vec![Effect::Show(Intent::Counts(counts(7)))],
            );
        }
    }

    #[test]
    fn a_reply_asked_before_an_invalidation_changes_nothing() {
        let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
        let Effect::Ask(stale, _) = focus.refresh_counts() else {
            panic!("refreshing asks");
        };
        focus.invalidate();
        assert!(
            focus
                .handle(Input::Reply(stale, Reply::FocusCounts(Ok(counts(3)))))
                .is_empty()
        );
    }

    #[test]
    fn tickets_are_never_reused() {
        let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
        let Effect::Ask(first, _) = focus.refresh_counts() else {
            panic!()
        };
        let Effect::Ask(second, _) = focus.refresh_counts() else {
            panic!()
        };
        assert_ne!(first.id, second.id);
    }

    #[test]
    fn quit_is_drawn_as_quit() {
        let mut focus = FocusController::new(Policy::for_platform(Platform::Freedesktop));
        assert_eq!(
            focus.handle(Input::Command(CommandId::Quit)),
            vec![Effect::Show(Intent::Quit)],
        );
    }
}
