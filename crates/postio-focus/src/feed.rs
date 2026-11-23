//! Filling Focus's list and keeping it in step: opening a place, paging, and re-reading what an
//! engine event moved. Focus's inbox never inserts an arrival at the top, because Focus may hold
//! it for a digest or file it away first; only a re-read can say which (`ListScope::reaction`).

use postio_core::Event;
use postio_model::listing::MessageSummary;
use postio_model::listing::{PageRequest, Surfaced, ThreadSummary};
use postio_model::{FocusScope, Label, ListScope, MailboxId, ThreadId};
use postio_ui::focus_list::{self, FocusRow};
use postio_ui::paging::{Fetch, Paging, Plan};
use postio_ui::surfaced::Spliced;

use crate::{Intent, Request};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Step {
    Show(Intent),
    Ask(Request),
    Open(ListScope),
    Timer {
        token: u64,
        after: std::time::Duration,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PageAnswer {
    Threads {
        total: u32,
        rows: Vec<ThreadSummary>,
        labels: Vec<(ThreadId, Label)>,
    },
    Messages {
        total: u32,
        rows: Vec<MessageSummary>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    pub scope: ListScope,
    pub accounts: Vec<postio_model::AccountId>,
    pub folders: Vec<(MailboxId, bool)>,
    pub total: Result<u32, String>,
    /// `None` for a place that does not splice.
    pub surfaced: Option<Result<Vec<Surfaced>, String>>,
}

/// `Input::Timer` with this token repaints the list once. Apart from the composer's small counts
/// and the open message's high bit.
pub(crate) const THREAD_REFRESH_TIMER: u64 = 1 << 62;

/// So a sync pass that changes many threads (each emits one `ThreadChanged`) reads the pages once.
pub(crate) const THREAD_REFRESH_WAIT: std::time::Duration = std::time::Duration::from_millis(60);

#[derive(Debug, Default)]
pub(crate) struct Feed {
    paging: Paging,
    total: u32,
    stored: u32,
    pages_asked: Vec<u32>,
    landed: bool,
    opened: bool,
    surfaced: Vec<FocusRow>,
    spliced: Spliced,
    refresh_armed: bool,
    /// Only an answer for it may move the totals.
    newest_stamp: Option<u64>,
}

impl Feed {
    /// The rows already on screen stay until the new place's first page lands, so a change of place
    /// never flashes empty.
    pub(crate) fn open(&mut self, scope: ListScope) -> Request {
        self.paging.open(scope);
        self.landed = false;
        self.opened = true;
        Request::OpenScope {
            scope,
            splices: splices(scope),
        }
    }

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

    pub(crate) fn wanted(&mut self, page: u32, stamp: u64) -> Vec<Step> {
        let Some(Fetch::Scope(request)) = self.paging.fetch_for(page) else {
            return vec![Step::Show(Intent::GiveUp { stamp, page })];
        };
        // The store's total may have moved since it was last read, so the run asked for assumes
        // conversations fill every position that is not a surfaced row; the answer's total places
        // them.
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
                // Drafts lists messages: each is a row of its own, with no label pills.
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
        if matches!(event, Event::ThreadChanged { .. }) {
            // A pass emits one per thread, back to back: the first starts
            // the wait and the rest join it.
            if !self.refresh_armed {
                self.refresh_armed = true;
                effects.push(Step::Timer {
                    token: THREAD_REFRESH_TIMER,
                    after: THREAD_REFRESH_WAIT,
                });
            }
            return effects;
        }
        match self.paging.plan(event) {
            Plan::Ignore if !surfaced_moved => {}
            _ if self.splices_now() => {
                // What is surfaced may have moved with the mail (an archived reminder's row goes
                // with its conversation): read it before the pages.
                effects.push(Step::Ask(Request::Surfaced));
            }
            Plan::Ignore => {}
            Plan::InsertAtTop(_) | Plan::Refetch(_) | Plan::Reload | Plan::Repaint => {
                effects.push(Step::Show(Intent::RefreshList));
            }
        }
        effects
    }

    pub(crate) fn thread_refresh_due(&mut self) -> Vec<Step> {
        if !std::mem::take(&mut self.refresh_armed) {
            return Vec::new();
        }
        vec![Step::Show(Intent::RefreshList)]
    }

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

    pub(crate) fn digest(&self, row: postio_model::MessageId) -> Option<focus_list::Digest> {
        self.surfaced.iter().find_map(|surfaced| match surfaced {
            FocusRow::Digest(digest) if surfaced.id() == row => Some(digest.clone()),
            _ => None,
        })
    }

    pub(crate) fn scope(&self) -> Option<ListScope> {
        self.paging.scope()
    }

    pub(crate) fn total(&self) -> u32 {
        self.total
    }

    pub(crate) fn pages_asked(&self) -> &[u32] {
        &self.pages_asked
    }

    /// A list still waiting for it is loading, not empty.
    pub(crate) fn has_landed(&self) -> bool {
        self.landed
    }

    /// True once per opening, so the cursor goes to the new list's first row once and a later
    /// re-read leaves it.
    pub(crate) fn take_opened(&mut self) -> bool {
        self.landed && std::mem::take(&mut self.opened)
    }

    pub(crate) fn is_inbox(&self, mailbox: MailboxId) -> bool {
        self.paging.is_inbox(mailbox)
    }

    /// Digests are not counted, matching the strip, which counts what the store holds.
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

fn splices(scope: ListScope) -> bool {
    scope == ListScope::Focus(FocusScope::Inbox)
}
