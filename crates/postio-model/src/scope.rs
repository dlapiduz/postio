//! Which messages a list is showing.
//!
//! Every reader of the message list — the store that pages through it, the
//! runtime that hands rows to a frontend, a frontend's own feed, and the FFI
//! boundary a second frontend crosses — answers the same question about the
//! same value, so it gets one type rather than a spelling per reader (#670).
//! `docs/archive/engineering-notes.md`'s "Six types are called *Scope*" entry has the
//! full map of what does and does not belong here — in particular
//! [`crate::AccountScope`] answers a different question ("which accounts?")
//! and `postio_core::state::ViewScope` is deliberately *not* this type: it is
//! the narrower result of a rule applied to one.

use crate::ids::{AccountId, MailboxId, ThreadId};

/// Which messages a list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ListScope {
    /// One folder, as the server has it.
    Mailbox(MailboxId),
    /// Every folder in an account: that account's whole mail.
    Account(AccountId),
    /// Every enabled account's inbox at once (ADR 0005 Q4, #1692).
    ///
    /// The inboxes, not all mail: a message archived, moved, snoozed or
    /// deleted from here leaves the view as it leaves a folder, which is what
    /// lets triage walk down it.
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
    /// This account's drafts whose send is under way — queued, or being sent.
    ///
    /// A view, never a destination, like [`Self::Unified`]: a message cannot
    /// be moved *into* the Outbox, because being there is a consequence of
    /// having been sent. [`Self::mailbox`] answers `None` for it, which is
    /// what makes that checkable rather than remembered.
    ///
    /// Per account, matching [`Self::Flagged`] and [`Self::Snoozed`]: the
    /// sidebar already knows how to place and count one row per account, and
    /// a unified Outbox would have to answer "which account is this sending
    /// from" for every row.
    Outbox(AccountId),
    /// One conversation, wherever its messages are filed.
    ///
    /// Not a narrowing of a mailbox: a thread routinely spans folders, and a
    /// drill-in that filtered the list's own resident rows used to show only
    /// the part of it that happened to be paged in.
    Thread(ThreadId),
    /// One of Postio Focus's own lists (spec 007).
    ///
    /// A view, never a destination, like [`Self::Unified`]. Only Focus reads
    /// these, and the classic app and the terminal never see one.
    Focus(FocusScope),
}

/// Which of Focus's lists a [`ListScope::Focus`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FocusScope {
    /// Focus's inbox: every enabled account's inbox, one row per
    /// conversation, newest first.
    ///
    /// Its membership is the unified inbox's. It is a scope of its own
    /// because it will not stay that: mail held for a digest and a
    /// conversation whose reminder has surfaced leave it, and every read of
    /// it asks the store's one Focus membership test so that its rows, its
    /// count and its seek marks agree about them.
    Inbox,
    /// The inbox's rows that draw a marker -- an invitation, a question, a
    /// to-do, a reminder -- and nothing else: what `!` narrows it to
    /// (spec 007 FR-017). Read from the markers rather than by walking the
    /// inbox, since they are few and it is not.
    HasAction,
    /// Everything snoozed, in every enabled account, wherever it is filed:
    /// `g z`. Messages, newest first, as the per-account
    /// [`ListScope::Snoozed`] lists them, over every account at once.
    Snoozed,
    /// Everything flagged, in every enabled account, wherever it is filed:
    /// `g *`. As [`FocusScope::Snoozed`] is to [`ListScope::Snoozed`].
    Flagged,
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
            | ListScope::Unified
            | ListScope::Flagged(_)
            | ListScope::Snoozed(_)
            // Never a destination: being in the Outbox is a consequence of
            // having been sent, not somewhere a message can be put.
            | ListScope::Outbox(_)
            | ListScope::Thread(_)
            | ListScope::Focus(_) => None,
        }
    }

    /// What a list showing this scope does when `account`'s `arrival`
    /// names `mailbox` — `None` for [`Arrival::MessagesChanged`], which is
    /// account-wide rather than about one mailbox.
    ///
    /// The rule, in one sentence (the classic app's feed module docs carried
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
    /// a feed at all, so every
    /// arrival is [`Reaction::Ignore`].
    ///
    /// `inbox` is whether `mailbox` is an inbox, when the caller knows --
    /// `None` when it cannot place the folder, which is always safe. Only
    /// [`ListScope::Unified`] asks: it is the inboxes (#1692), so an arrival
    /// in a folder known not to be one cannot move a row, and every other
    /// scope already names the folder or account it is gated on.
    pub fn reaction(
        self,
        arrival: Arrival,
        account: AccountId,
        mailbox: Option<MailboxId>,
        inbox: Option<bool>,
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
            // Every account's inbox (#1692), so no account's arrival is
            // somebody else's, and a folder known not to be an inbox holds
            // nothing drawn here -- but a delivery still never inserts. A unified row is a conversation
            // grouped across accounts, and mail arriving at the second
            // address for a conversation already on screen *folds into that
            // row* rather than adding one. An insert cannot express that: it
            // would draw the same conversation twice, which is the one thing
            // the grouping exists to prevent. Reloading re-runs the walk,
            // which is the only thing that knows which it was.
            //
            // Focus's inbox is the same inboxes, and it never inserts either:
            // Focus may hold an arrival for a digest or file it away before it
            // is ever a row, and only the store knows which.
            // Focus's two views over every account are membership questions
            // like the per-account ones, and so are never inserted into; no
            // account's change is somebody else's.
            ListScope::Focus(FocusScope::Snoozed | FocusScope::Flagged) => match arrival {
                MessagesRemoved | MessageListChanged | MessagesChanged => Reload,
                NewMail => Ignore,
            },
            ListScope::Unified | ListScope::Focus(_) => match arrival {
                NewMail | MessagesRemoved | MessageListChanged if inbox == Some(false) => Ignore,
                NewMail | MessagesRemoved | MessageListChanged => Reload,
                MessagesChanged => Refetch,
            },
            // The Outbox joins these two: all three are a question about one
            // account's mail wherever it is filed, so none of them can insert
            // at the top -- a row's membership depends on an answer that may
            // have changed, not on where the mail arrived.
            ListScope::Flagged(scoped) | ListScope::Snoozed(scoped) | ListScope::Outbox(scoped) => {
                match arrival {
                    MessagesRemoved | MessageListChanged | MessagesChanged if account == scoped => {
                        Reload
                    }
                    _ => Ignore,
                }
            }
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
    // ── A view is never a destination (spec 003 FR-010, ADR 0036) ────────

    #[test]
    fn only_a_mailbox_scope_names_somewhere_a_message_could_be_put() {
        // The rule ADR 0036 records, asserted rather than remembered. Every
        // caller that goes on to use this answer as a destination -- a move,
        // an append, a per-folder setting -- is relying on it, and the way it
        // fails is silent: `MessageSet::InMailbox` with a mailbox nobody has
        // matches no rows and reports success.
        //
        // Written out per variant rather than in a loop, so that adding a
        // scope is a compile error here instead of a case nobody classified.
        assert_eq!(
            ListScope::Mailbox(MailboxId::new(7)).mailbox(),
            Some(MailboxId::new(7))
        );
        assert_eq!(ListScope::Account(AccountId::new(1)).mailbox(), None);
        assert_eq!(ListScope::Unified.mailbox(), None);
        assert_eq!(ListScope::Flagged(AccountId::new(1)).mailbox(), None);
        assert_eq!(ListScope::Snoozed(AccountId::new(1)).mailbox(), None);
        assert_eq!(
            ListScope::Outbox(AccountId::new(1)).mailbox(),
            None,
            "being in the Outbox is a consequence of having been sent, not \
             somewhere a message can be put"
        );
        assert_eq!(ListScope::Thread(ThreadId::new(3)).mailbox(), None);
        assert_eq!(
            ListScope::Focus(FocusScope::Inbox).mailbox(),
            None,
            "Focus's inbox is a view over every inbox, not a folder"
        );
    }

    #[test]
    fn the_has_action_filter_reacts_as_focus_s_inbox_does() {
        // A marker arrives with a message, and a row leaves the filter when
        // its mail leaves the inbox: the same events, the same reloads.
        for arrival in [
            Arrival::NewMail,
            Arrival::MessagesRemoved,
            Arrival::MessageListChanged,
            Arrival::MessagesChanged,
        ] {
            for (mailbox, inbox) in [
                (Some(INBOX), Some(true)),
                (Some(ARCHIVE), Some(false)),
                (None, None),
            ] {
                assert_eq!(
                    ListScope::Focus(FocusScope::HasAction).reaction(arrival, HOME, mailbox, inbox),
                    ListScope::Focus(FocusScope::Inbox).reaction(arrival, HOME, mailbox, inbox),
                    "{arrival:?} in {mailbox:?}"
                );
            }
        }
    }

    #[test]
    fn focus_s_inbox_reacts_to_every_arrival_as_the_unified_inbox_does() {
        // The same inboxes, read as conversations, so the same events move
        // it -- and like Unified it never inserts a delivery at the top: Focus
        // may hold an arrival back or file it away before it is ever a row
        // (spec 007), and only a reload asks the store which it was.
        let focus = ListScope::Focus(FocusScope::Inbox);
        for arrival in [
            Arrival::NewMail,
            Arrival::MessagesRemoved,
            Arrival::MessageListChanged,
            Arrival::MessagesChanged,
        ] {
            for (mailbox, inbox) in [
                (Some(INBOX), Some(true)),
                (Some(ARCHIVE), Some(false)),
                (Some(ARCHIVE), None),
                (None, None),
            ] {
                for account in [HOME, AWAY] {
                    assert_eq!(
                        focus.reaction(arrival, account, mailbox, inbox),
                        ListScope::Unified.reaction(arrival, account, mailbox, inbox),
                        "{arrival:?} in {mailbox:?} (an inbox: {inbox:?}) of {account:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_outbox_reloads_for_its_own_account_and_ignores_another() {
        // It joins Flagged and Snoozed: a question about one account's mail
        // wherever it is filed, so no arrival can insert at the top -- a
        // row's membership depends on an answer that may have changed, not on
        // where the mail landed.
        let mine = AccountId::new(1);
        let theirs = AccountId::new(2);
        let outbox = ListScope::Outbox(mine);

        for arrival in [
            Arrival::MessagesRemoved,
            Arrival::MessageListChanged,
            Arrival::MessagesChanged,
        ] {
            assert_eq!(
                outbox.reaction(arrival, mine, None, None),
                Reaction::Reload,
                "{arrival:?} in this account has to re-ask what is on its way"
            );
            assert_eq!(
                outbox.reaction(arrival, theirs, None, None),
                Reaction::Ignore,
                "{arrival:?} in another account is not this Outbox's business"
            );
        }
        assert_eq!(
            outbox.reaction(Arrival::NewMail, mine, None, None),
            Reaction::Ignore,
            "mail arriving is not a message being sent"
        );
    }

    use super::*;

    const HOME: AccountId = AccountId::new(1);
    const AWAY: AccountId = AccountId::new(2);
    const INBOX: MailboxId = MailboxId::new(10);
    const ARCHIVE: MailboxId = MailboxId::new(11);

    #[test]
    fn a_mailbox_scope_inserts_new_mail_at_the_top_of_its_own_mailbox_only() {
        let scope = ListScope::Mailbox(INBOX);
        assert_eq!(
            scope.reaction(Arrival::NewMail, HOME, Some(INBOX), None),
            Reaction::InsertAtTop
        );
        assert_eq!(
            scope.reaction(Arrival::NewMail, HOME, Some(ARCHIVE), None),
            Reaction::Ignore,
            "mail landing in a different mailbox does not belong at this one's top"
        );
    }

    #[test]
    fn a_mailbox_scope_reloads_on_removal_and_reorder_of_its_own_mailbox_only() {
        let scope = ListScope::Mailbox(INBOX);
        for arrival in [Arrival::MessagesRemoved, Arrival::MessageListChanged] {
            assert_eq!(
                scope.reaction(arrival, HOME, Some(INBOX), None),
                Reaction::Reload
            );
            assert_eq!(
                scope.reaction(arrival, HOME, Some(ARCHIVE), None),
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
            scope.reaction(Arrival::MessagesChanged, HOME, None, None),
            Reaction::Refetch
        );
        assert_eq!(
            scope.reaction(Arrival::MessagesChanged, AWAY, None, None),
            Reaction::Refetch
        );
    }

    #[test]
    fn an_account_scope_behaves_like_mailbox_but_gated_on_the_account() {
        let scope = ListScope::Account(HOME);
        assert_eq!(
            scope.reaction(Arrival::NewMail, HOME, Some(INBOX), None),
            Reaction::InsertAtTop,
            "the unified view's own order puts a delivery at the top too"
        );
        assert_eq!(
            scope.reaction(Arrival::NewMail, AWAY, Some(INBOX), None),
            Reaction::Ignore
        );
        for arrival in [Arrival::MessagesRemoved, Arrival::MessageListChanged] {
            assert_eq!(
                scope.reaction(arrival, HOME, Some(INBOX), None),
                Reaction::Reload
            );
            assert_eq!(
                scope.reaction(arrival, AWAY, Some(INBOX), None),
                Reaction::Ignore
            );
        }
        assert_eq!(
            scope.reaction(Arrival::MessagesChanged, AWAY, None, None),
            Reaction::Refetch
        );
    }

    #[test]
    fn unified_reacts_to_every_account_and_never_inserts_a_delivery() {
        let scope = ListScope::Unified;
        for account in [HOME, AWAY] {
            assert_eq!(
                scope.reaction(Arrival::NewMail, account, Some(INBOX), None),
                Reaction::Reload,
                "a delivery can fold into a row already on screen, and an \
                 insert would draw that conversation a second time"
            );
            for arrival in [Arrival::MessagesRemoved, Arrival::MessageListChanged] {
                assert_eq!(
                    scope.reaction(arrival, account, Some(INBOX), None),
                    Reaction::Reload,
                    "{arrival:?} in {account:?}: no account's mail is somebody \
                     else's here"
                );
            }
            assert_eq!(
                scope.reaction(Arrival::MessagesChanged, account, None, None),
                Reaction::Refetch,
                "a flag change moves neither the membership nor the grouping"
            );
        }
    }

    #[test]
    fn unified_ignores_a_folder_it_knows_is_not_an_inbox() {
        // #1692: Unified is the inboxes, so mail landing in, leaving, or
        // changing wholesale in any other folder cannot move a row -- a sync
        // of the Archive used to re-read the whole view.
        let scope = ListScope::Unified;
        for arrival in [
            Arrival::NewMail,
            Arrival::MessagesRemoved,
            Arrival::MessageListChanged,
        ] {
            assert_eq!(
                scope.reaction(arrival, HOME, Some(ARCHIVE), Some(false)),
                Reaction::Ignore,
                "{arrival:?} in a folder that is not an inbox"
            );
            assert_eq!(
                scope.reaction(arrival, HOME, Some(INBOX), Some(true)),
                Reaction::Reload,
                "{arrival:?} in an inbox"
            );
            assert_eq!(
                scope.reaction(arrival, HOME, Some(ARCHIVE), None),
                Reaction::Reload,
                "{arrival:?} in a folder the caller cannot place: a reload is \
                 the answer that cannot leave a row behind"
            );
        }
        assert_eq!(
            scope.reaction(Arrival::MessagesChanged, HOME, None, None),
            Reaction::Refetch,
            "a flag change names messages, not a folder, and patches rows in place"
        );
    }

    #[test]
    fn flagged_never_inserts_new_mail_a_delivery_is_never_flagged_yet() {
        let scope = ListScope::Flagged(HOME);
        assert_eq!(
            scope.reaction(Arrival::NewMail, HOME, Some(INBOX), None),
            Reaction::Ignore,
            "a delivery does not carry \\Flagged; inserting it would put a \
             non-matching row above matching ones"
        );
    }

    #[test]
    fn snoozed_never_inserts_new_mail_either() {
        let scope = ListScope::Snoozed(HOME);
        assert_eq!(
            scope.reaction(Arrival::NewMail, HOME, Some(INBOX), None),
            Reaction::Ignore
        );
    }

    #[test]
    fn flagged_and_snoozed_reload_on_a_flag_change_because_membership_moved() {
        for scope in [ListScope::Flagged(HOME), ListScope::Snoozed(HOME)] {
            assert_eq!(
                scope.reaction(Arrival::MessagesChanged, HOME, None, None),
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
                    scope.reaction(arrival, HOME, Some(INBOX), None),
                    Reaction::Reload,
                    "{scope:?} / {arrival:?} in this scope's own account"
                );
                assert_eq!(
                    scope.reaction(arrival, AWAY, Some(INBOX), None),
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
            scope.reaction(Arrival::MessagesRemoved, HOME, Some(INBOX), None),
            scope.reaction(Arrival::MessagesRemoved, HOME, Some(ARCHIVE), None),
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
                scope.reaction(arrival, HOME, Some(INBOX), None),
                Reaction::Ignore,
                "{arrival:?}: a drill-in reads its own thread directly and \
                 never routes through here"
            );
        }
    }
}
