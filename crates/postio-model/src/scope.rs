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
    /// Every folder in an account: that account's whole mail.
    Account(AccountId),
    /// Every folder in every enabled account at once (ADR 0005 Q4).
    ///
    /// A view, never a destination: mail cannot be moved *into* it, and the
    /// commands that need somewhere to put a message are unavailable here.
    /// Rows are conversations grouped across accounts, so one row can stand
    /// for two threads the user received at two addresses.
    Unified,
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
    /// Whether the list is showing something out of `mailboxes`.
    ///
    /// The question a frontend asks when a folder tree arrives and it has to
    /// decide whether to open a default folder: is there already something
    /// on screen that belongs to *this* tree?
    ///
    /// "Something is open" on its own cannot answer it, and #813 is both
    /// halves of that. A smart folder is open and names no `MailboxId`, so a
    /// frontend testing for one concluded nothing was showing and opened the
    /// inbox over the top on every reload. A folder left behind by the
    /// *previous* account names one perfectly well, so the same test
    /// concluded something was showing and never opened the new account's
    /// inbox. Asking which tree the scope comes from separates them.
    ///
    /// [`Unified`](Self::Unified) and [`Thread`](Self::Thread) always answer
    /// yes: neither is a folder in any one tree, and both are somewhere a
    /// person went deliberately, so a reload has no business replacing them.
    pub fn is_drawn_from(&self, mailboxes: &[crate::mailbox::Mailbox]) -> bool {
        let account_present =
            |account: AccountId| mailboxes.iter().any(|folder| folder.account_id == account);
        match self {
            Self::Mailbox(id) => mailboxes.iter().any(|folder| folder.id == *id),
            Self::Account(account) | Self::Flagged(account) | Self::Snoozed(account) => {
                account_present(*account)
            }
            Self::Unified | Self::Thread(_) => !mailboxes.is_empty(),
        }
    }

    /// The folder this scope names, when it names one.
    ///
    /// `None` for a smart folder or a thread — load-bearing wherever the
    /// caller goes on to use the answer as somewhere a message could be put.
    pub fn mailbox(self) -> Option<MailboxId> {
        match self {
            ListScope::Mailbox(id) => Some(id),
            ListScope::Account(_)
            | ListScope::Unified
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
            // Every account, so no arrival is somebody else's -- but a
            // delivery still never inserts. A unified row is a conversation
            // grouped across accounts, and mail arriving at the second
            // address for a conversation already on screen *folds into that
            // row* rather than adding one. An insert cannot express that: it
            // would draw the same conversation twice, which is the one thing
            // the grouping exists to prevent. Reloading re-runs the walk,
            // which is the only thing that knows which it was.
            ListScope::Unified => match arrival {
                NewMail | MessagesRemoved | MessageListChanged => Reload,
                MessagesChanged => Refetch,
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
    fn unified_reacts_to_every_account_and_never_inserts_a_delivery() {
        let scope = ListScope::Unified;
        for account in [HOME, AWAY] {
            assert_eq!(
                scope.reaction(Arrival::NewMail, account, Some(INBOX)),
                Reaction::Reload,
                "a delivery can fold into a row already on screen, and an \
                 insert would draw that conversation a second time"
            );
            for arrival in [Arrival::MessagesRemoved, Arrival::MessageListChanged] {
                assert_eq!(
                    scope.reaction(arrival, account, Some(INBOX)),
                    Reaction::Reload,
                    "{arrival:?} in {account:?}: no account's mail is somebody \
                     else's here"
                );
            }
            assert_eq!(
                scope.reaction(Arrival::MessagesChanged, account, None),
                Reaction::Refetch,
                "a flag change moves neither the membership nor the grouping"
            );
        }
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

    /// #813's question: is the list already showing something out of the
    /// folder tree that just arrived?
    mod is_drawn_from {
        use super::*;
        use crate::mailbox::Mailbox;

        fn folder(account: AccountId, id: i64) -> Mailbox {
            let mut mailbox = Mailbox::new(account, "INBOX", Some('/'));
            mailbox.id = MailboxId::new(id);
            mailbox
        }

        fn home_tree() -> Vec<Mailbox> {
            vec![folder(HOME, 1), folder(HOME, 2)]
        }

        #[test]
        fn a_folder_in_the_tree_is_drawn_from_it() {
            assert!(ListScope::Mailbox(MailboxId::new(2)).is_drawn_from(&home_tree()));
        }

        #[test]
        fn a_folder_from_another_account_is_not() {
            // The account switch. The folder the previous account left on
            // screen is not part of the tree that just replaced it, and
            // treating it as "something is already open" is what stopped the
            // new account's inbox from ever opening.
            assert!(!ListScope::Mailbox(MailboxId::new(9)).is_drawn_from(&home_tree()));
        }

        #[test]
        fn a_smart_folder_belongs_to_its_account() {
            assert!(ListScope::Flagged(HOME).is_drawn_from(&home_tree()));
            assert!(ListScope::Snoozed(HOME).is_drawn_from(&home_tree()));
            assert!(ListScope::Account(HOME).is_drawn_from(&home_tree()));
        }

        #[test]
        fn a_smart_folder_of_an_absent_account_does_not() {
            let other = AccountId::new(HOME.get() + 1);
            assert!(!ListScope::Flagged(other).is_drawn_from(&home_tree()));
        }

        #[test]
        fn the_unified_view_and_a_drill_in_are_never_overridden() {
            // Neither is a folder in the tree, and both are somewhere the
            // user went deliberately. A reload that replaced them would be
            // the bug this predicate exists to prevent.
            assert!(ListScope::Unified.is_drawn_from(&home_tree()));
            assert!(ListScope::Thread(crate::ids::ThreadId::new(1)).is_drawn_from(&home_tree()));
        }

        #[test]
        fn nothing_is_drawn_from_an_empty_tree() {
            // An account whose first read came back empty has nothing open
            // and nothing to open, so the caller is still owed a pick.
            assert!(!ListScope::Mailbox(MailboxId::new(1)).is_drawn_from(&[]));
            assert!(!ListScope::Flagged(HOME).is_drawn_from(&[]));
        }
    }
}
