//! What a windowed list asks for, and what it does when the mail moves —
//! the policy behind every message list, with no toolkit and no store in it.
//!
//! Two frontends page the same [`crate::list::ListWindow`], and each had
//! its own copy of the policy around it. `postio-gtk`'s feed knew how to
//! turn a page number into a store request or a slice of search hits, what
//! each scope does with a runtime event (the table below) and how often a
//! failed page may be asked for again; the macOS boundary re-derived the
//! first, flattened the second into "count again and start over" on every
//! event, and had no answer to the third. [`Paging`] is the one copy. What
//! stays with the frontend is the *crossing* — how a read reaches the store
//! and comes back on the right thread — and the widget.
//!
//! # Driving the list from events
//!
//! [`ListScope::reaction`] answers this, per scope, and both frontends agree
//! on one rule: a list reacts to an event only when the event can change its
//! own membership or order, and it inserts at the top only when its own
//! order guarantees the new rows belong there. Everything else reloads —
//! dropping everything cached and asking again. #773 is the investigation
//! this table closes.
//!
//! | Scope | `NewMail` | `MessagesRemoved` | `MessageListChanged` | `MessagesChanged` |
//! |---|---|---|---|---|
//! | `Mailbox` | insert at top when the mailbox matches | reload when the mailbox matches | reload when the mailbox matches | refetch resident pages holding them |
//! | `Account` | insert at top when the account matches | reload when the account matches | reload when the account matches | refetch resident pages holding them |
//! | `Flagged` / `Snoozed` | ignore — a delivery is neither flagged nor snoozed | reload when the account matches | reload when the account matches | **reload** when the account matches |
//! | a result set | ignore | ignore | ignore | refetch resident pages holding them |
//!
//! `Flagged`/`Snoozed` reloading on `MessagesChanged` rather than refetching
//! is the one cell that looks like the others and is not: for them the flag
//! or the snooze *is* the membership predicate, so a change can remove a row
//! the same event would repaint for a mailbox. A page refetch cannot express
//! a row leaving; only a reload can, because the total moved. They are
//! gated on the *account*, not the mailbox, because they span every folder
//! in one — the mailbox an event names carries no information for them.
//!
//! A result set — however much the frontend remembers which folder to go
//! back to — refetches on `MessagesChanged` (the same rows in the same
//! order) and ignores the other three: its membership is decided by the
//! query, not by delivery order or a resync. Nothing opened yet ignores
//! everything too.
//!
//! `ListScope::Thread` never reaches a `Paging` at all: a drill-in issues one
//! direct fetch instead, so it has no event routing to get wrong.

use std::collections::HashMap;

use postio_core::Event;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::{Arrival, ListScope, Reaction};

use crate::list::PAGE_SIZE;

/// One page of a scope, as the store answered it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<T> {
    /// How many rows the scope has in total, as of this read.
    ///
    /// Carried with every page rather than asked for separately: the count
    /// and the rows have to come from one read of the database, or the list
    /// can be told about a total that no page will ever fill.
    pub total: u32,
    /// The rows themselves, in list order.
    pub rows: Vec<T>,
}

/// Which rows of a scope are wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRequest {
    /// The messages being listed.
    pub scope: ListScope,
    /// The page index, for the reply to be matched against.
    pub page: u32,
    /// The first row wanted, counted from the newest.
    pub offset: u32,
    /// How many rows to read.
    pub limit: u32,
}

/// What to read for one page of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetch {
    /// A page of the scope in view, by offset.
    Scope(PageRequest),
    /// A page of the result set: these ids, in this order.
    ///
    /// Search hits are ranked, not sorted, so no scope describes them and
    /// the store cannot page them by offset; each page names the ids it
    /// wants, and the order is the ranking — a store asked for a set of ids
    /// answers in whatever order its index found them, and a page that
    /// re-sorted the ranking would put the best match wherever its date
    /// happened to fall.
    Hits {
        /// The page index, for the reply to be matched against.
        page: u32,
        /// The ids on it, most relevant first.
        ids: Vec<MessageId>,
    },
}

/// What the list does about one runtime event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan<'a> {
    /// Nothing about this list changes.
    Ignore,
    /// This many new rows belong at the top, in the arrival's own order.
    InsertAtTop(u32),
    /// The same rows in the same order; re-read the resident pages holding
    /// these, so the row objects are updated in place.
    Refetch(&'a [MessageId]),
    /// The membership or the order moved: drop everything cached and ask
    /// again, keeping the scroll position.
    Reload,
}

/// How many times one page may be re-asked for after a failed read.
///
/// Bounded because the repair has to be a repair and not a spin: a store that
/// is failing every read would otherwise be asked again by every repaint, for
/// ever. Three is enough for the case this exists for — a write the page read
/// collided with — and stops well short of a loop.
const FAILED_PAGE_ATTEMPTS: u8 = 3;

/// The policy around a windowed list: what is in view, what a page of it
/// means, and what an event does to it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Paging {
    /// What the list is showing: one folder, or a role-scoped query.
    scope: Option<ListScope>,
    /// The hits in view, ranked, or `None` when the scope is in view.
    ///
    /// Only the ids: the rows are read a page at a time like any other, so
    /// a query matching forty thousand messages costs forty thousand ids and
    /// one page of mail.
    results: Option<Vec<MessageId>>,
    /// Pages whose read failed, and how often they have been re-asked.
    ///
    /// Cleared whenever the scope changes: page 2 of the folder just left is
    /// not page 2 of this one.
    failed: HashMap<u32, u8>,
    /// Which folders the frontend has placed, and whether each is an inbox.
    ///
    /// What [`ListScope::Unified`] needs to answer an arrival: it is the
    /// inboxes (#1692), so mail moving in any other folder cannot move a
    /// row. Kept across scope changes -- it describes the folders, not the
    /// view -- and a folder missing from it is answered as if it might be
    /// an inbox.
    inboxes: HashMap<MailboxId, bool>,
}

impl Paging {
    /// Say which folders are inboxes: `(folder, is_inbox)` for every folder
    /// the frontend knows, replacing what it said before.
    pub fn set_folders(&mut self, folders: impl IntoIterator<Item = (MailboxId, bool)>) {
        self.inboxes = folders.into_iter().collect();
    }

    /// Show `scope`, leaving any result set: the sidebar is a way out of a
    /// search as much as `Esc` is.
    pub fn open(&mut self, scope: ListScope) {
        self.scope = Some(scope);
        self.results = None;
        self.failed.clear();
    }

    /// What the list is showing, folder or query — `None` before anything
    /// is open. Set aside, not forgotten, while a result set is in view.
    pub fn scope(&self) -> Option<ListScope> {
        self.scope
    }

    /// The mailbox in view, if the scope is one.
    ///
    /// `None` in a smart folder as well as before anything is open. That is
    /// deliberate: a role-scoped query is not a mailbox an action can be
    /// aimed at. See [`ListScope::mailbox`].
    pub fn mailbox(&self) -> Option<MailboxId> {
        self.scope.and_then(ListScope::mailbox)
    }

    /// Show `ids` — the hits, most relevant first — instead of the scope,
    /// and answer how many there are. The scope is remembered, not left.
    pub fn show_results(&mut self, ids: Vec<MessageId>) -> u32 {
        let total = ids.len() as u32;
        self.results = Some(ids);
        self.failed.clear();
        total
    }

    /// Put the scope back. Returns whether there were results to leave.
    pub fn close_results(&mut self) -> bool {
        self.failed.clear();
        self.results.take().is_some()
    }

    /// Whether the list is showing search hits rather than the scope.
    pub fn showing_results(&self) -> bool {
        self.results.is_some()
    }

    /// What to read for `page`, or `None` when there is nothing to read:
    /// nothing is open, or the page is past the end of the result set —
    /// asking for ids a short last page does not have would make the store
    /// answer for messages nobody matched.
    pub fn fetch_for(&self, page: u32) -> Option<Fetch> {
        if let Some(results) = &self.results {
            let start = (page * PAGE_SIZE) as usize;
            if start >= results.len() {
                return None;
            }
            let end = results.len().min(start + PAGE_SIZE as usize);
            return Some(Fetch::Hits {
                page,
                ids: results[start..end].to_vec(),
            });
        }
        let scope = self.scope?;
        Some(Fetch::Scope(PageRequest {
            scope,
            page,
            offset: page * PAGE_SIZE,
            limit: PAGE_SIZE,
        }))
    }

    /// What the list does about `event`, by the table in the module docs.
    ///
    /// Everything it does not recognise is [`Plan::Ignore`], deliberately:
    /// the event stream carries the whole application, and a list that
    /// reacted to all of it would repaint on every keystroke in the composer.
    pub fn plan<'a>(&self, event: &'a Event) -> Plan<'a> {
        let (reaction, messages): (Reaction, &[MessageId]) = match event {
            Event::NewMail {
                account,
                mailbox,
                messages,
                ..
            } => (
                self.reaction(Arrival::NewMail, *account, Some(*mailbox)),
                messages,
            ),
            Event::MessagesChanged { account, messages } => (
                self.reaction(Arrival::MessagesChanged, *account, None),
                messages,
            ),
            Event::MessagesRemoved {
                account, mailbox, ..
            } => (
                self.reaction(Arrival::MessagesRemoved, *account, Some(*mailbox)),
                &[],
            ),
            Event::MessageListChanged { account, mailbox } => (
                self.reaction(Arrival::MessageListChanged, *account, Some(*mailbox)),
                &[],
            ),
            _ => return Plan::Ignore,
        };
        match reaction {
            Reaction::Ignore => Plan::Ignore,
            Reaction::InsertAtTop => Plan::InsertAtTop(messages.len() as u32),
            Reaction::Refetch => Plan::Refetch(messages),
            Reaction::Reload => Plan::Reload,
        }
    }

    /// A read of `page` failed. Whether to ask for it again.
    ///
    /// `true` while the page is within [`FAILED_PAGE_ATTEMPTS`]: the caller
    /// abandons the page and asks once more, quietly — a read that collided
    /// with a write and succeeds on the next ask is not something to tell
    /// somebody about. `false` once it is out of attempts, which is when the
    /// failure is worth saying, and it is said once.
    pub fn retry(&mut self, page: u32) -> bool {
        let attempts = self.failed.entry(page).or_insert(0);
        *attempts += 1;
        *attempts <= FAILED_PAGE_ATTEMPTS
    }

    /// What the list does with one mailbox-shaped `arrival`, result set or
    /// scope alike.
    fn reaction(
        &self,
        arrival: Arrival,
        account: AccountId,
        mailbox: Option<MailboxId>,
    ) -> Reaction {
        if self.results.is_some() {
            return match arrival {
                Arrival::MessagesChanged => Reaction::Refetch,
                Arrival::NewMail | Arrival::MessagesRemoved | Arrival::MessageListChanged => {
                    Reaction::Ignore
                }
            };
        }
        let inbox = mailbox.and_then(|mailbox| self.inboxes.get(&mailbox).copied());
        self.scope
            .map(|scope| scope.reaction(arrival, account, mailbox, inbox))
            .unwrap_or(Reaction::Ignore)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(range: std::ops::Range<i64>) -> Vec<MessageId> {
        range.map(MessageId::new).collect()
    }

    const ACCOUNT: AccountId = AccountId::new(1);
    const INBOX: MailboxId = MailboxId::new(10);
    const ARCHIVE: MailboxId = MailboxId::new(11);

    fn inbox() -> Paging {
        let mut paging = Paging::default();
        paging.open(ListScope::Mailbox(INBOX));
        paging
    }

    fn new_mail(mailbox: MailboxId, count: i64) -> Event {
        Event::NewMail {
            account: ACCOUNT,
            mailbox,
            messages: ids(100..100 + count),
        }
    }

    fn changed(messages: Vec<MessageId>) -> Event {
        Event::MessagesChanged {
            account: ACCOUNT,
            messages,
        }
    }

    fn removed(mailbox: MailboxId) -> Event {
        Event::MessagesRemoved {
            account: ACCOUNT,
            mailbox,
            messages: ids(1..3),
        }
    }

    fn list_changed(mailbox: MailboxId) -> Event {
        Event::MessageListChanged {
            account: ACCOUNT,
            mailbox,
        }
    }

    // What a page means.

    #[test]
    fn a_page_of_a_scope_is_an_offset_read_of_it() {
        assert_eq!(
            inbox().fetch_for(2),
            Some(Fetch::Scope(PageRequest {
                scope: ListScope::Mailbox(INBOX),
                page: 2,
                offset: 2 * PAGE_SIZE,
                limit: PAGE_SIZE,
            }))
        );
    }

    #[test]
    fn nothing_open_has_nothing_to_read() {
        assert_eq!(Paging::default().fetch_for(0), None);
    }

    #[test]
    fn a_page_of_a_result_set_names_its_ids_in_ranking_order() {
        let mut paging = inbox();
        let total = paging.show_results(ids(1..(PAGE_SIZE as i64 + 4)));
        assert_eq!(total, PAGE_SIZE + 3);
        assert_eq!(
            paging.fetch_for(0),
            Some(Fetch::Hits {
                page: 0,
                ids: ids(1..(PAGE_SIZE as i64 + 1)),
            })
        );
        assert_eq!(
            paging.fetch_for(1),
            Some(Fetch::Hits {
                page: 1,
                ids: ids((PAGE_SIZE as i64 + 1)..(PAGE_SIZE as i64 + 4)),
            }),
            "the last page is short, and asks only for the ids it has"
        );
        assert_eq!(
            paging.fetch_for(2),
            None,
            "past the end of the ranking there is nothing to ask for"
        );
    }

    #[test]
    fn leaving_a_result_set_pages_the_scope_again() {
        let mut paging = inbox();
        paging.show_results(ids(1..4));
        assert!(paging.showing_results());
        assert!(paging.close_results());
        assert!(!paging.showing_results());
        assert!(matches!(paging.fetch_for(0), Some(Fetch::Scope(_))));
        assert!(
            !paging.close_results(),
            "there were no results left to leave"
        );
    }

    #[test]
    fn opening_a_scope_leaves_the_result_set() {
        let mut paging = inbox();
        paging.show_results(ids(1..4));
        paging.open(ListScope::Mailbox(ARCHIVE));
        assert!(!paging.showing_results());
        assert_eq!(paging.mailbox(), Some(ARCHIVE));
    }

    #[test]
    fn a_smart_folder_is_not_a_mailbox() {
        let mut paging = Paging::default();
        paging.open(ListScope::Flagged(ACCOUNT));
        assert_eq!(paging.scope(), Some(ListScope::Flagged(ACCOUNT)));
        assert_eq!(paging.mailbox(), None);
    }

    // The table: what each scope does with each event.

    #[test]
    fn unified_ignores_a_folder_it_has_been_told_is_not_an_inbox() {
        // #1692: Unified is the inboxes. Mail landing in the Archive -- a
        // sync of it, an archive landing there -- cannot move a row, and a
        // folder the list was never told about still reloads, because a
        // reload cannot leave a row behind and an ignore can.
        let mut paging = Paging::default();
        paging.open(ListScope::Unified);
        assert_eq!(
            paging.plan(&new_mail(ARCHIVE, 2)),
            Plan::Reload,
            "not told yet"
        );

        paging.set_folders([(INBOX, true), (ARCHIVE, false)]);
        assert_eq!(paging.plan(&new_mail(ARCHIVE, 2)), Plan::Ignore);
        assert_eq!(paging.plan(&list_changed(ARCHIVE)), Plan::Ignore);
        assert_eq!(paging.plan(&removed(ARCHIVE)), Plan::Ignore);
        assert_eq!(paging.plan(&new_mail(INBOX, 2)), Plan::Reload);
        assert_eq!(paging.plan(&removed(INBOX)), Plan::Reload);
        assert_eq!(
            paging.plan(&new_mail(MailboxId::new(12), 1)),
            Plan::Reload,
            "a folder it cannot place"
        );
    }

    #[test]
    fn mail_landing_in_the_open_mailbox_is_inserted_at_the_top() {
        assert_eq!(inbox().plan(&new_mail(INBOX, 3)), Plan::InsertAtTop(3));
    }

    #[test]
    fn mail_landing_elsewhere_is_ignored() {
        assert_eq!(inbox().plan(&new_mail(ARCHIVE, 3)), Plan::Ignore);
    }

    #[test]
    fn changed_messages_refetch_the_pages_holding_them() {
        let event = changed(ids(5..8));
        assert_eq!(inbox().plan(&event), Plan::Refetch(&ids(5..8)));
    }

    #[test]
    fn rows_leaving_the_open_mailbox_reload_it() {
        assert_eq!(inbox().plan(&removed(INBOX)), Plan::Reload);
        assert_eq!(inbox().plan(&removed(ARCHIVE)), Plan::Ignore);
    }

    #[test]
    fn a_resync_of_the_open_mailbox_reloads_it() {
        assert_eq!(inbox().plan(&list_changed(INBOX)), Plan::Reload);
        assert_eq!(inbox().plan(&list_changed(ARCHIVE)), Plan::Ignore);
    }

    #[test]
    fn a_flagged_view_reloads_on_a_flag_change_because_the_flag_is_its_membership() {
        let mut paging = Paging::default();
        paging.open(ListScope::Flagged(ACCOUNT));
        assert_eq!(paging.plan(&changed(ids(5..8))), Plan::Reload);
        assert_eq!(
            paging.plan(&new_mail(INBOX, 3)),
            Plan::Ignore,
            "a delivery is neither flagged nor snoozed"
        );
        assert_eq!(paging.plan(&removed(INBOX)), Plan::Reload);
    }

    #[test]
    fn a_result_set_only_refetches() {
        let mut paging = inbox();
        paging.show_results(ids(1..4));
        assert_eq!(
            paging.plan(&new_mail(INBOX, 3)),
            Plan::Ignore,
            "membership is the query's, not delivery order's"
        );
        assert_eq!(paging.plan(&removed(INBOX)), Plan::Ignore);
        assert_eq!(paging.plan(&list_changed(INBOX)), Plan::Ignore);
        let event = changed(ids(2..3));
        assert_eq!(paging.plan(&event), Plan::Refetch(&ids(2..3)));
    }

    #[test]
    fn nothing_open_ignores_everything() {
        let paging = Paging::default();
        assert_eq!(paging.plan(&new_mail(INBOX, 3)), Plan::Ignore);
        assert_eq!(paging.plan(&changed(ids(1..2))), Plan::Ignore);
        assert_eq!(paging.plan(&list_changed(INBOX)), Plan::Ignore);
    }

    #[test]
    fn events_the_list_is_not_about_are_ignored() {
        assert_eq!(
            inbox().plan(&Event::Error {
                message: "nothing to do with the list".to_owned()
            }),
            Plan::Ignore
        );
    }

    // Failed pages.

    #[test]
    fn a_failed_page_is_asked_for_again_a_bounded_number_of_times() {
        let mut paging = inbox();
        assert!(paging.retry(2));
        assert!(paging.retry(2));
        assert!(paging.retry(2));
        assert!(!paging.retry(2), "out of attempts: now it is worth saying");
        assert!(paging.retry(3), "another page's attempts are its own");
    }

    #[test]
    fn opening_another_scope_forgets_the_failed_pages() {
        let mut paging = inbox();
        for _ in 0..3 {
            paging.retry(2);
        }
        paging.open(ListScope::Mailbox(ARCHIVE));
        assert!(
            paging.retry(2),
            "page 2 of the folder just left is not page 2 of this one"
        );
    }
}
