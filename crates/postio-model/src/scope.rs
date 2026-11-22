//! One type for every reader of the message list (#670). `AccountScope` answers a different
//! question (which accounts),
//! and `postio_core::state::ViewScope` is deliberately not this type.

use crate::ids::{AccountId, MailboxId, ThreadId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ListScope {
    Mailbox(MailboxId),
    Account(AccountId),
    /// The inboxes, not all mail: a message archived, moved, snoozed or deleted leaves the view as
    /// it leaves a folder, so triage can walk
    /// down it (ADR 0005 Q4, #1692). A view, never a destination. One row can stand for two threads
    /// received at two addresses.
    Unified,
    Flagged(AccountId),
    Snoozed(AccountId),
    /// A view, never a destination: being in the Outbox is a consequence of having been sent, and
    /// `mailbox()` answers `None` for it.
    /// Per account like `Flagged` and `Snoozed`, since a unified Outbox would have to say which
    /// account each row sends from.
    Outbox(AccountId),
    /// Not a narrowing of a mailbox: a thread spans folders, and filtering resident rows showed
    /// only the part paged in.
    Thread(ThreadId),
    /// A view, never a destination. Only Focus reads these.
    Focus(FocusScope),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FocusScope {
    /// Its membership is the unified inbox's, but it is its own scope because it will not stay
    /// that: mail held for a digest and
    /// conversations whose reminder has surfaced leave it. Every read asks the store's one Focus
    /// membership test so rows, count and seek marks agree.
    Inbox,
    /// Rows that draw a marker, which `!` narrows to; read from the markers, which are few, rather
    /// than by walking the inbox.
    HasAction,
    Snoozed,
    Flagged,
}

impl ListScope {
    /// `None` for a smart folder or a thread; load-bearing wherever the answer is used as somewhere
    /// a message could be put.
    pub fn mailbox(self) -> Option<MailboxId> {
        match self {
            ListScope::Mailbox(id) => Some(id),
            ListScope::Account(_)
            | ListScope::Unified
            | ListScope::Flagged(_)
            | ListScope::Snoozed(_)
            // Never a destination: being in the Outbox is a consequence of having been sent.
            | ListScope::Outbox(_)
            | ListScope::Thread(_)
            | ListScope::Focus(_) => None,
        }
    }

    /// A list reacts only when the event can change its own membership or order, and inserts at the
    /// top only when its own order guarantees the
    /// new rows belong there; everything else reloads. `Flagged`, `Snoozed` and `Outbox` span every
    /// folder, so they never insert, and
    /// `MessagesChanged` is `Reload` since a refetch cannot express a row leaving. `Thread` never
    /// reaches a feed. `inbox` is whether
    /// `mailbox` is an inbox, `None` when the caller cannot place it, which is always safe; only
    /// `Unified` asks (#1692).
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
            // Every account's inbox (#1692). A delivery never inserts: a unified row is a
            // conversation grouped across accounts, so mail
            // at a second address folds into a row on screen and an insert would draw it twice; a
            // reload re-runs the walk. Focus's inbox
            // never inserts either: Focus may hold an arrival for a digest or file it away before
            // it is a row.
            ListScope::Focus(FocusScope::Snoozed | FocusScope::Flagged) => match arrival {
                MessagesRemoved | MessageListChanged | MessagesChanged => Reload,
                NewMail => Ignore,
            },
            ListScope::Unified | ListScope::Focus(_) => match arrival {
                NewMail | MessagesRemoved | MessageListChanged if inbox == Some(false) => Ignore,
                NewMail | MessagesRemoved | MessageListChanged => Reload,
                MessagesChanged => Refetch,
            },
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

/// Named apart from `postio_core::Event`, which this crate cannot depend on; a frontend maps its
/// own event to one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arrival {
    NewMail,
    MessagesRemoved,
    MessageListChanged,
    MessagesChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reaction {
    Ignore,
    InsertAtTop,
    Reload,
    Refetch,
}

#[cfg(test)]
mod reaction_tests {

    #[test]
    fn only_a_mailbox_scope_names_somewhere_a_message_could_be_put() {
        // Written out per variant, not in a loop, so adding a scope is a compile error here. A
        // wrong destination fails silently:
        // `MessageSet::InMailbox` with a mailbox nobody has matches no rows and reports success
        // (ADR 0036).
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
        // #1692: Unified is the inboxes, so other folders cannot move a row.
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
