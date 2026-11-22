//! What a windowed list asks for and does when the mail moves, with no toolkit or store: the one
//! policy for every frontend's list. A list reacts only to events that can change its membership or
//! order; everything else reloads. `Flagged`/`Snoozed` reload on `MessagesChanged` (the flag is the
//! membership predicate) and gate on account, since they span folders.

use std::collections::HashMap;

use postio_core::Event;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::{Arrival, ListScope, Reaction};

use crate::list::PAGE_SIZE;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<T> {
    /// Carried with every page: the count and rows must come from one read, or the list can be told
    /// of a total no page will fill.
    pub total: u32,
    pub rows: Vec<T>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRequest {
    pub scope: ListScope,
    pub page: u32,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetch {
    Scope(PageRequest),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan<'a> {
    Ignore,
    InsertAtTop(u32),
    Refetch(&'a [MessageId]),
    Reload,
    Repaint,
}

/// Bounded so the repair is not a spin; three covers a read that collided with a write.
const FAILED_PAGE_ATTEMPTS: u8 = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Paging {
    scope: Option<ListScope>,
    /// Cleared whenever the scope changes: page 2 of the folder just left is not page 2 of this
    /// one.
    failed: HashMap<u32, u8>,
    /// What `ListScope::Unified` needs to answer an arrival: it is the inboxes (#1692). A folder
    /// missing here is treated as if it might be an inbox.
    inboxes: HashMap<MailboxId, bool>,
}

impl Paging {
    pub fn set_folders(&mut self, folders: impl IntoIterator<Item = (MailboxId, bool)>) {
        self.inboxes = folders.into_iter().collect();
    }

    pub fn is_inbox(&self, mailbox: MailboxId) -> bool {
        self.inboxes.get(&mailbox).copied().unwrap_or(false)
    }

    pub fn open(&mut self, scope: ListScope) {
        self.scope = Some(scope);
        self.failed.clear();
    }

    pub fn scope(&self) -> Option<ListScope> {
        self.scope
    }

    /// `None` in a smart folder too: a role-scoped query is not a mailbox an action can be aimed
    /// at.
    pub fn mailbox(&self) -> Option<MailboxId> {
        self.scope.and_then(ListScope::mailbox)
    }

    pub fn fetch_for(&self, page: u32) -> Option<Fetch> {
        let scope = self.scope?;
        Some(Fetch::Scope(PageRequest {
            scope,
            page,
            offset: page * PAGE_SIZE,
            limit: PAGE_SIZE,
        }))
    }

    /// Everything unrecognised is `Plan::Ignore`: the event stream carries the whole application.
    pub fn plan<'a>(&self, event: &'a Event) -> Plan<'a> {
        let (reaction, messages): (Reaction, &[MessageId]) = match event {
            Event::ContactsChanged { names_changed } => {
                let showing = self.scope.is_some();
                return if *names_changed && showing {
                    Plan::Repaint
                } else {
                    Plan::Ignore
                };
            }
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

    /// `true` while within `FAILED_PAGE_ATTEMPTS`: the caller abandons the page and asks once more,
    /// quietly.
    /// `false` when out of attempts, which is when the failure is said, once.
    pub fn retry(&mut self, page: u32) -> bool {
        let attempts = self.failed.entry(page).or_insert(0);
        *attempts += 1;
        *attempts <= FAILED_PAGE_ATTEMPTS
    }

    fn reaction(
        &self,
        arrival: Arrival,
        account: AccountId,
        mailbox: Option<MailboxId>,
    ) -> Reaction {
        let inbox = mailbox.and_then(|mailbox| self.inboxes.get(&mailbox).copied());
        self.scope
            .map(|scope| scope.reaction(arrival, account, mailbox, inbox))
            .unwrap_or(Reaction::Ignore)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_folder_is_an_inbox_only_when_it_was_named_one() {
        let mut paging = Paging::default();
        let (inbox, travel, unknown) = (MailboxId::new(1), MailboxId::new(2), MailboxId::new(3));
        paging.set_folders([(inbox, true), (travel, false)]);
        assert!(paging.is_inbox(inbox));
        assert!(!paging.is_inbox(travel));
        assert!(
            !paging.is_inbox(unknown),
            "an unplaced folder is not assumed an inbox"
        );
    }

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

    #[test]
    fn a_name_change_repaints_what_is_on_screen_and_nothing_else() {
        let names = Event::ContactsChanged {
            names_changed: true,
        };
        let quiet = Event::ContactsChanged {
            names_changed: false,
        };
        assert_eq!(inbox().plan(&names), Plan::Repaint);
        assert_eq!(inbox().plan(&quiet), Plan::Ignore);
        assert_eq!(
            Paging::default().plan(&names),
            Plan::Ignore,
            "a list showing nothing has nothing to repaint"
        );
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
    fn a_smart_folder_is_not_a_mailbox() {
        let mut paging = Paging::default();
        paging.open(ListScope::Flagged(ACCOUNT));
        assert_eq!(paging.scope(), Some(ListScope::Flagged(ACCOUNT)));
        assert_eq!(paging.mailbox(), None);
    }

    #[test]
    fn unified_ignores_a_folder_it_has_been_told_is_not_an_inbox() {
        // #1692: Unified is the inboxes. A folder the list was never told about still reloads: a
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
                message: "nothing to do with the list".to_owned(),
                account: None,
            }),
            Plan::Ignore
        );
    }

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
