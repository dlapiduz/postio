//! Which messages a list is showing.
//!
//! Every reader of the message list — the store that pages through it, the
//! runtime that hands rows to a frontend, a frontend's own feed, and the FFI
//! boundary a second frontend crosses — answers the same question about the
//! same value, so it gets one type rather than a spelling per reader (#670).
//! `docs/engineering-notes.md`'s "Six types are called *Scope*" entry has the
//! full map of what does and does not belong here — in particular
//! [`crate::AccountScope`] answers a different question ("which accounts?")
//! and `postio_core::state::ViewScope` is deliberately *not* this type: it is
//! the narrower result of a rule applied to one.

use crate::ids::{AccountId, MailboxId, ThreadId};

/// Which messages a list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListScope {
    /// One folder, as the server has it.
    Mailbox(MailboxId),
    /// Every folder in an account: the unified view.
    Account(AccountId),
    /// Everything flagged in an account, wherever it is filed.
    Flagged(AccountId),
    /// Everything currently snoozed in an account, wherever it is filed.
    Snoozed(AccountId),
    /// One conversation, wherever its messages are filed.
    ///
    /// Not a narrowing of a mailbox: a thread routinely spans folders, and a
    /// drill-in that filtered the list's own resident rows used to show only
    /// the part of it that happened to be paged in.
    Thread(ThreadId),
}

impl ListScope {
    /// The folder this scope names, when it names one.
    ///
    /// `None` for a smart folder or a thread — load-bearing wherever the
    /// caller goes on to use the answer as somewhere a message could be put.
    pub fn mailbox(self) -> Option<MailboxId> {
        match self {
            ListScope::Mailbox(id) => Some(id),
            ListScope::Account(_)
            | ListScope::Flagged(_)
            | ListScope::Snoozed(_)
            | ListScope::Thread(_) => None,
        }
    }

    /// What a list showing this scope does when `account`'s `arrival`
    /// names `mailbox` — `None` for [`Arrival::MessagesChanged`], which is
    /// account-wide rather than about one mailbox.
    ///
    /// The rule, in one sentence (`postio_gtk::feed`'s module docs carry
    /// the full table this answers): a list reacts to an event only when
    /// the event can change its own membership or order, and it inserts at
    /// the top only when its own order guarantees the new rows belong
    /// there. Everything else reloads.
    ///
    /// [`ListScope::Mailbox`] and [`ListScope::Account`] are gated on the
    /// identity they name — a folder or every folder in an account — and
    /// insert new mail at the top, because their order already guarantees
    /// it belongs there. [`ListScope::Flagged`] and [`ListScope::Snoozed`]
    /// are gated on the account, because they span every folder in it, and
    /// they never insert: neither scope's membership is decided by
    /// arrival, only by the flag or the snooze, so [`Arrival::NewMail`] is
    /// always [`Reaction::Ignore`] and [`Arrival::MessagesChanged`] — a
    /// flag or a snooze changing — is [`Reaction::Reload`] rather than
    /// [`Reaction::Refetch`]: the membership moved, and a page refetch
    /// cannot express a row leaving. [`ListScope::Thread`] never reaches a
    /// [`Feed`](../../postio_gtk/feed/struct.Feed.html) at all, so every
    /// arrival is [`Reaction::Ignore`].
    pub fn reaction(
        self,
        arrival: Arrival,
        account: AccountId,
        mailbox: Option<MailboxId>,
    ) -> Reaction {
        use Arrival::{MessageListChanged, MessagesChanged, MessagesRemoved, NewMail};
        use Reaction::{Ignore, InsertAtTop, Refetch, Reload};

        match self {
            ListScope::Mailbox(scoped) => match arrival {
                NewMail if mailbox == Some(scoped) => InsertAtTop,
                MessagesRemoved | MessageListChanged if mailbox == Some(scoped) => Reload,
                MessagesChanged => Refetch,
                _ => Ignore,
            },
            ListScope::Account(scoped) => match arrival {
                NewMail if account == scoped => InsertAtTop,
                MessagesRemoved | MessageListChanged if account == scoped => Reload,
                MessagesChanged => Refetch,
                _ => Ignore,
            },
            ListScope::Flagged(scoped) | ListScope::Snoozed(scoped) => match arrival {
                MessagesRemoved | MessageListChanged | MessagesChanged if account == scoped => {
                    Reload
                }
                _ => Ignore,
            },
            ListScope::Thread(_) => Ignore,
        }
    }
}

/// One of the four events whose effect on a list depends on what it shows.
///
/// Named apart from `postio_core::Event`, which this crate may not depend
/// on — `postio-core` depends on `postio-model`, never the reverse — so a
/// frontend maps its own event to one of these before asking
/// [`ListScope::reaction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arrival {
    /// Mail delivered.
    NewMail,
    /// Messages left a mailbox: archived, deleted, moved away.
    MessagesRemoved,
    /// A mailbox's list changed enough that the window must reload: a
    /// resync, a re-sort, a filter change.
    MessageListChanged,
    /// Messages changed in place: flags, labels, read state.
    MessagesChanged,
}

/// What a scope does with one [`Arrival`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reaction {
    /// Nothing about this list changes.
    Ignore,
    /// New rows belong at the top, in the arrival's own order.
    InsertAtTop,
    /// The membership or the order moved; drop everything cached and
    /// re-ask.
    Reload,
    /// The rows are the same rows in the same order; refetch only the
    /// pages holding them.
    Refetch,
}

#[cfg(test)]
mod reaction_tests {
    use super::*;

    const HOME: AccountId = AccountId::new(1);
    const AWAY: AccountId = AccountId::new(2);
    const INBOX: MailboxId = MailboxId::new(10);
    const ARCHIVE: MailboxId = MailboxId::new(11);

    #[test]
    fn a_mailbox_scope_inserts_new_mail_at_the_top_of_its_own_mailbox_only() {
        let scope = ListScope::Mailbox(INBOX);
        assert_eq!(
            scope.reaction(Arrival::NewMail, HOME, Some(INBOX)),
            Reaction::InsertAtTop
        );
        assert_eq!(
            scope.reaction(Arrival::NewMail, HOME, Some(ARCHIVE)),
            Reaction::Ignore,
            "mail landing in a different mailbox does not belong at this one's top"
        );
    }

    #[test]
    fn a_mailbox_scope_reloads_on_removal_and_reorder_of_its_own_mailbox_only() {
        let scope = ListScope::Mailbox(INBOX);
        for arrival in [Arrival::MessagesRemoved, Arrival::MessageListChanged] {
            assert_eq!(scope.reaction(arrival, HOME, Some(INBOX)), Reaction::Reload);
            assert_eq!(
                scope.reaction(arrival, HOME, Some(ARCHIVE)),
                Reaction::Ignore,
                "{arrival:?} for a different mailbox must not reload this one"
            );
        }
    }

    #[test]
    fn a_mailbox_scope_refetches_on_messages_changed_regardless_of_account() {
        let scope = ListScope::Mailbox(INBOX);
        // No mailbox to compare against -- MessagesChanged is account-wide
        // by shape, and `pages_holding` is what actually filters it.
        assert_eq!(
            scope.reaction(Arrival::MessagesChanged, HOME, None),
            Reaction::Refetch
        );
        assert_eq!(
            scope.reaction(Arrival::MessagesChanged, AWAY, None),
            Reaction::Refetch
        );
    }

    #[test]
    fn an_account_scope_behaves_like_mailbox_but_gated_on_the_account() {
        let scope = ListScope::Account(HOME);
        assert_eq!(
            scope.reaction(Arrival::NewMail, HOME, Some(INBOX)),
            Reaction::InsertAtTop,
            "the unified view's own order puts a delivery at the top too"
        );
        assert_eq!(
            scope.reaction(Arrival::NewMail, AWAY, Some(INBOX)),
            Reaction::Ignore
        );
        for arrival in [Arrival::MessagesRemoved, Arrival::MessageListChanged] {
            assert_eq!(scope.reaction(arrival, HOME, Some(INBOX)), Reaction::Reload);
            assert_eq!(scope.reaction(arrival, AWAY, Some(INBOX)), Reaction::Ignore);
        }
        assert_eq!(
            scope.reaction(Arrival::MessagesChanged, AWAY, None),
            Reaction::Refetch
        );
    }

    #[test]
    fn flagged_never_inserts_new_mail_a_delivery_is_never_flagged_yet() {
        let scope = ListScope::Flagged(HOME);
        assert_eq!(
            scope.reaction(Arrival::NewMail, HOME, Some(INBOX)),
            Reaction::Ignore,
            "a delivery does not carry \\Flagged; inserting it would put a \
             non-matching row above matching ones"
        );
    }

    #[test]
    fn snoozed_never_inserts_new_mail_either() {
        let scope = ListScope::Snoozed(HOME);
        assert_eq!(
            scope.reaction(Arrival::NewMail, HOME, Some(INBOX)),
            Reaction::Ignore
        );
    }

    #[test]
    fn flagged_and_snoozed_reload_on_a_flag_change_because_membership_moved() {
        for scope in [ListScope::Flagged(HOME), ListScope::Snoozed(HOME)] {
            assert_eq!(
                scope.reaction(Arrival::MessagesChanged, HOME, None),
                Reaction::Reload,
                "{scope:?}: unflagging removes the row, which a page \
                 refetch cannot express -- only a reload moves the total"
            );
        }
    }

    #[test]
    fn flagged_and_snoozed_reload_on_removal_and_reorder_gated_on_the_account() {
        for scope in [ListScope::Flagged(HOME), ListScope::Snoozed(HOME)] {
            for arrival in [
                Arrival::MessagesRemoved,
                Arrival::MessageListChanged,
                Arrival::MessagesChanged,
            ] {
                assert_eq!(
                    scope.reaction(arrival, HOME, Some(INBOX)),
                    Reaction::Reload,
                    "{scope:?} / {arrival:?} in this scope's own account"
                );
                assert_eq!(
                    scope.reaction(arrival, AWAY, Some(INBOX)),
                    Reaction::Ignore,
                    "{scope:?} / {arrival:?}: a different account must not \
                     reload a list it cannot affect"
                );
            }
        }
    }

    #[test]
    fn flagged_and_snoozed_are_indifferent_to_which_mailbox_an_event_names() {
        // These scopes span every folder in the account, so the mailbox in
        // the event carries no information for them -- only the account
        // does.
        let scope = ListScope::Flagged(HOME);
        assert_eq!(
            scope.reaction(Arrival::MessagesRemoved, HOME, Some(INBOX)),
            scope.reaction(Arrival::MessagesRemoved, HOME, Some(ARCHIVE)),
        );
    }

    #[test]
    fn a_thread_scope_ignores_every_arrival() {
        let scope = ListScope::Thread(crate::ids::ThreadId::new(1));
        for arrival in [
            Arrival::NewMail,
            Arrival::MessagesRemoved,
            Arrival::MessageListChanged,
            Arrival::MessagesChanged,
        ] {
            assert_eq!(
                scope.reaction(arrival, HOME, Some(INBOX)),
                Reaction::Ignore,
                "{arrival:?}: a drill-in reads its own thread directly and \
                 never routes through here"
            );
        }
    }
}
