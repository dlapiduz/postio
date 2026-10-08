//! Filling Focus's list, and keeping it in step (research R2, slice 2).
//!
//! Moved from `postio-gtk`'s `list/feed.rs` so the Mac fills its list by the
//! same rules. Opening a place counts it, reads which folders are inboxes and
//! what Focus surfaces among its conversations, then has the list change
//! over; a page asked for is read with its label pills and the surfaced rows
//! spliced in; an engine event re-reads what it moved, by `postio_ui::paging`'s
//! one policy. Focus's inbox never inserts an arrival at the top, because
//! Focus may hold it for a digest or file it away first, and only a re-read
//! of what is on screen can say which (`ListScope::reaction`).
//!
//! What the toolkit's list counts as stale is the toolkit's: each page asked
//! for carries the list's own stamp (GTK's model generation, the Mac's
//! window generation), and every answer for that page echoes it back for the
//! list to check. What the feed itself counts as stale is a place that is no
//! longer in view.

use postio_core::Event;
use postio_model::listing::MessageSummary;
use postio_model::listing::{PageRequest, Surfaced, ThreadSummary};
use postio_model::{FocusScope, Label, ListScope, MailboxId, ThreadId};
use postio_ui::focus_list::{self, FocusRow};
use postio_ui::paging::{Fetch, Paging, Plan};
use postio_ui::surfaced::Spliced;

use crate::{Intent, Request};

/// One thing the feed wants done: drawn, or asked of the engine. The
/// controller gives each ask its ticket.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Step {
    Show(Intent),
    Ask(Request),
    /// Open this place through the feed, as `FocusController::open` does.
    Open(ListScope),
}

/// A page's rows as the store answered them, before the feed shapes them.
#[derive(Debug, Clone, PartialEq)]
pub enum PageAnswer {
    /// A scope that lists conversations, with each row's label pills.
    Threads {
        /// How many conversations the scope holds, as of this read.
        total: u32,
        /// The conversations, most recently active first.
        rows: Vec<ThreadSummary>,
        /// The label pills of those conversations, read in one round trip.
        labels: Vec<(ThreadId, Label)>,
    },
    /// A scope that lists messages: Drafts.
    Messages {
        /// How many messages the scope holds, as of this read.
        total: u32,
        /// The messages, newest first.
        rows: Vec<MessageSummary>,
    },
}

/// What opening a place learned, in one round.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    /// The place that was opened.
    pub scope: ListScope,
    /// The enabled accounts: what Focus's inbox is made of, and what
    /// "select everything" reaches.
    pub accounts: Vec<postio_model::AccountId>,
    /// Every enabled account's folders, and whether each is an inbox.
    pub folders: Vec<(MailboxId, bool)>,
    /// How many conversations the place holds.
    pub total: Result<u32, String>,
    /// What Focus surfaces among them, for a place that splices; `None` for
    /// one that does not.
    pub surfaced: Option<Result<Vec<Surfaced>, String>>,
}

/// The list's state, as the feed keeps it.
#[derive(Debug, Default)]
pub(crate) struct Feed {
    paging: Paging,
    /// The row count the list draws: conversations plus surfaced rows.
    total: u32,
    /// How many conversations the store holds, without the surfaced rows.
    stored: u32,
    /// Every page asked of the store, in order: "only the visible window is
    /// read" (spec 007 US1 scenario 7) is a question about which pages.
    pages_asked: Vec<u32>,
    /// Whether the first page of the place in view has landed.
    landed: bool,
    /// Whether the place in view was opened and its rows have not yet been
    /// shown to a cursor (`take_opened`).
    opened: bool,
    /// The rows Focus's inbox surfaces among its conversations.
    surfaced: Vec<FocusRow>,
    /// Where they sit.
    spliced: Spliced,
    /// The newest stamp the list has asked a page under: only an answer for
    /// it may move the totals, as the list's own generation guarded them.
    newest_stamp: Option<u64>,
}

impl Feed {
    /// Show `scope`: count it and read what it splices, then change over.
    ///
    /// The rows already on screen stay until the new place's first page
    /// lands, so a change of place never flashes empty.
    pub(crate) fn open(&mut self, scope: ListScope) -> Request {
        self.paging.open(scope);
        self.landed = false;
        self.opened = true;
        Request::OpenScope {
            scope,
            splices: splices(scope),
        }
    }

    /// The answer to [`open`](Self::open): the list changes over, and its
    /// first page is asked for.
    pub(crate) fn opened(&mut self, opened: Opened) -> Vec<Step> {
        if self.paging.scope() != Some(opened.scope) {
            // Another place was opened while this one was being counted.
            return Vec::new();
        }
        self.paging.set_folders(opened.folders);
        let total = opened.total.unwrap_or_else(|error| {
            tracing::warn!(%error, "Focus could not count its list");
            0
        });
        self.place_surfaced(opened.surfaced);
        self.stored = total;
        self.total = self.spliced.total(total);
        // The first page is asked for by the list as it changes over: rows
        // already on screen stay until it lands, and nothing else would ask.
        vec![Step::Show(Intent::ReplaceSource { total: self.total })]
    }

    /// The list wants `page`, under its own `stamp`.
    pub(crate) fn wanted(&mut self, page: u32, stamp: u64) -> Vec<Step> {
        let Some(Fetch::Scope(request)) = self.paging.fetch_for(page) else {
            return vec![Step::Show(Intent::GiveUp { stamp, page })];
        };
        // The page's positions, in the store's terms. The store's total may
        // have moved since it was last read, so the run asked for assumes
        // conversations fill every position that is not a surfaced row, and
        // the answer's own total places them.
        let asked = self.spliced.page(request.offset, request.limit, u32::MAX);
        self.pages_asked.push(page);
        self.newest_stamp = Some(self.newest_stamp.map_or(stamp, |newest| newest.max(stamp)));
        vec![
            Step::Show(Intent::PagePending { stamp, page }),
            Step::Ask(Request::Page {
                page,
                stamp,
                start: request.offset,
                count: request.limit,
                wanted: PageRequest {
                    scope: request.scope,
                    offset: asked.offset,
                    limit: asked.limit.max(1),
                },
            }),
        ]
    }

    /// A page's answer: shaped, spliced and delivered, or asked again.
    pub(crate) fn page(
        &mut self,
        page: u32,
        stamp: u64,
        start: u32,
        count: u32,
        answer: Result<PageAnswer, String>,
    ) -> Vec<Step> {
        let current = self.newest_stamp == Some(stamp);
        match answer {
            Ok(PageAnswer::Threads {
                total: stored,
                rows,
                labels,
            }) => {
                let conversations = focus_list::conversations(rows, labels);
                let placed = self.spliced.page(start, count, stored);
                let rows = focus_list::place(&placed.slots, &self.surfaced, &conversations);
                let total = self.spliced.total(stored);
                if current {
                    self.stored = stored;
                    self.total = total;
                }
                self.landed = true;
                vec![
                    Step::Show(Intent::DeliverPage {
                        stamp,
                        page,
                        total,
                        rows,
                    }),
                    Step::Show(Intent::Filled),
                ]
            }
            Ok(PageAnswer::Messages { total, rows }) => {
                // Drafts lists messages, not conversations: each is a row of
                // its own, with no label pills and nothing spliced in.
                let rows = rows
                    .into_iter()
                    .map(|message| FocusRow::conversation(focus_list::lone(message)))
                    .collect();
                if current {
                    self.stored = total;
                    self.total = total;
                }
                self.landed = true;
                vec![
                    Step::Show(Intent::DeliverPage {
                        stamp,
                        page,
                        total,
                        rows,
                    }),
                    Step::Show(Intent::Filled),
                ]
            }
            Err(error) => {
                tracing::warn!(page, %error, "Focus could not read a page");
                if self.paging.retry(page) {
                    let mut effects = vec![Step::Show(Intent::AbandonPage { stamp, page })];
                    effects.extend(self.wanted(page, stamp));
                    effects
                } else {
                    vec![Step::Show(Intent::GiveUp { stamp, page })]
                }
            }
        }
    }

    /// Bring the list into step with `event`, by `postio_ui::paging`'s table.
    pub(crate) fn event(&mut self, event: &Event) -> Vec<Step> {
        let mut effects = Vec::new();
        if let Event::MessagesRemoved {
            mailbox, messages, ..
        } = event
        {
            // Said before the list re-reads, so the store's own caches have
            // already let go of what left.
            effects.push(Step::Ask(Request::NoteRemoved {
                mailbox: *mailbox,
                messages: messages.clone(),
            }));
        }
        let surfaced_moved = matches!(event, Event::SurfacedChanged);
        match self.paging.plan(event) {
            Plan::Ignore if !surfaced_moved => {}
            _ if self.splices_now() => {
                // What is surfaced may have moved with the mail -- an
                // archived reminder's row goes with its conversation -- so it
                // is read again before the pages are.
                effects.push(Step::Ask(Request::Surfaced));
            }
            Plan::Ignore => {}
            Plan::InsertAtTop(_) | Plan::Refetch(_) | Plan::Reload => {
                effects.push(Step::Show(Intent::RefreshList));
            }
        }
        effects
    }

    /// The surfaced rows, read again after an event: placed, then the pages
    /// re-read around them.
    pub(crate) fn resurfaced(&mut self, surfaced: Result<Vec<Surfaced>, String>) -> Vec<Step> {
        if !self.splices_now() {
            return Vec::new();
        }
        self.place_surfaced(Some(surfaced));
        vec![Step::Show(Intent::RefreshList)]
    }

    fn place_surfaced(&mut self, surfaced: Option<Result<Vec<Surfaced>, String>>) {
        let mut rows = Vec::new();
        let mut positions = Vec::new();
        match surfaced {
            Some(Ok(surfaced)) => {
                for row in &surfaced {
                    if let Some(focus_row) = FocusRow::surfaced(row) {
                        positions.push(row.position());
                        rows.push(focus_row);
                    }
                }
            }
            Some(Err(error)) => {
                tracing::warn!(%error, "Focus could not read its surfaced rows");
            }
            None => {}
        }
        self.spliced = Spliced::new(&positions);
        self.surfaced = rows;
        self.total = self.spliced.total(self.stored);
    }

    fn splices_now(&self) -> bool {
        self.paging.scope().is_some_and(splices)
    }

    /// The place in view, once one is open.
    /// The digest whose row is `row`, among what Focus surfaces.
    pub(crate) fn digest(&self, row: postio_model::MessageId) -> Option<focus_list::Digest> {
        self.surfaced.iter().find_map(|surfaced| match surfaced {
            FocusRow::Digest(digest) if surfaced.id() == row => Some(digest.clone()),
            _ => None,
        })
    }

    pub(crate) fn scope(&self) -> Option<ListScope> {
        self.paging.scope()
    }

    /// How many rows the place in view has, as the store last said.
    pub(crate) fn total(&self) -> u32 {
        self.total
    }

    /// Every page asked of the store, in order.
    pub(crate) fn pages_asked(&self) -> &[u32] {
        &self.pages_asked
    }

    /// Whether the first page of the place in view has landed: a list still
    /// waiting for it is loading, not empty.
    pub(crate) fn has_landed(&self) -> bool {
        self.landed
    }

    /// Whether a place was opened and its first page has landed since this
    /// was last asked: true once per opening, so the cursor goes to the new
    /// list's first row once and a later re-read leaves it where it is.
    pub(crate) fn take_opened(&mut self) -> bool {
        self.landed && std::mem::take(&mut self.opened)
    }

    /// Whether `mailbox` is one of the inboxes Focus's own inbox is made of.
    pub(crate) fn is_inbox(&self, mailbox: MailboxId) -> bool {
        self.paging.is_inbox(mailbox)
    }

    /// The cursor's `index` among the messages of a list of `total` rows,
    /// and how many messages it holds: digests are not counted, matching the
    /// strip, which counts what the store holds.
    pub(crate) fn message_place(&self, index: u32, total: u32) -> (u32, u32) {
        let digests: Vec<usize> = self
            .surfaced
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, FocusRow::Digest(_)))
            .map(|(which, _)| which)
            .collect();
        self.spliced.message_place(&digests, index, total)
    }
}

/// Whether `scope` has rows spliced among its conversations: Focus's own
/// inbox, and nothing else.
fn splices(scope: ListScope) -> bool {
    scope == ListScope::Focus(FocusScope::Inbox)
}
