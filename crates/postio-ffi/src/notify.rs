//! New-mail notifications at the boundary: the decision, not the delivery. `decide_notification` is
//! `postio_ui::notify`'s
//! rule with the macOS wording: ids and counts only, since a notification is a log the lock screen
//! reads out.
//! The role gate (`[sync] notify_roles`) is decided here; the master switch crosses as
//! `Session::notices`.

use postio_model::{MailboxId, MessageId};

use crate::MailboxRoleFfi;
use postio_ui::notify::{self, Attention, Decision, Suppressed, Wording};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MailArrivalFfi {
    pub mailbox: i64,
    pub messages: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SuppressedFfi {
    NothingArrived,
    AlreadyOnScreen,
    NotWatched,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MailNotificationFfi {
    /// Stable per folder, so a second batch replaces the one on screen.
    pub identifier: String,
    /// What it says: counts and a folder name, never message content.
    pub title: String,
    pub body: String,
    pub mailbox: i64,
    pub message: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum NotificationDecisionFfi {
    Suppress { reason: SuppressedFfi },
    Deliver { notification: MailNotificationFfi },
}

/// `showing` is the folder the list has open and `active` whether Postio is frontmost, both needed
/// to suppress. An
/// arrival in a folder whose `role` is not in `notify_roles` is `SuppressedFfi::NotWatched`.
#[uniffi::export]
pub fn decide_notification(
    arrival: MailArrivalFfi,
    showing: Option<i64>,
    active: bool,
    mailbox_name: Option<String>,
    role: MailboxRoleFfi,
    notify_roles: Vec<String>,
) -> NotificationDecisionFfi {
    if !notify::role_watched(&notify_roles, role.into()) {
        return NotificationDecisionFfi::Suppress {
            reason: SuppressedFfi::NotWatched,
        };
    }
    let arrival = notify::Arrival {
        mailbox: MailboxId::new(arrival.mailbox),
        messages: arrival.messages.into_iter().map(MessageId::new).collect(),
    };
    let attention = Attention {
        showing: showing.map(MailboxId::new),
        active,
    };
    let wording = Wording::Counts {
        mailbox_name: mailbox_name.as_deref(),
    };
    match notify::decide(&arrival, attention, wording) {
        Decision::Suppress(reason) => NotificationDecisionFfi::Suppress {
            reason: match reason {
                Suppressed::NothingArrived => SuppressedFfi::NothingArrived,
                Suppressed::AlreadyOnScreen => SuppressedFfi::AlreadyOnScreen,
            },
        },
        Decision::Deliver(notification) => NotificationDecisionFfi::Deliver {
            notification: MailNotificationFfi {
                identifier: notification.identifier,
                title: notification.title,
                body: notification.body,
                mailbox: notification.mailbox.get(),
                message: notification.message.map(|message| message.get()),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boundary_words_a_notification_as_counts_and_a_folder_name() {
        let decision = decide_notification(
            MailArrivalFfi {
                mailbox: 7,
                messages: vec![1, 2, 3],
            },
            Some(99),
            true,
            Some("Inbox".to_owned()),
            MailboxRoleFfi::Inbox,
            vec!["inbox".to_owned()],
        );
        assert_eq!(
            decision,
            NotificationDecisionFfi::Deliver {
                notification: MailNotificationFfi {
                    identifier: "new-mail-7".to_owned(),
                    title: "New mail in Inbox".to_owned(),
                    body: "3 new messages".to_owned(),
                    mailbox: 7,
                    message: None,
                }
            }
        );
    }

    fn inbox_only() -> Vec<String> {
        vec!["inbox".to_owned()]
    }

    #[test]
    fn an_arrival_in_a_folder_the_person_did_not_choose_is_not_told() {
        // `[sync] notify_roles` defaults to the inbox, so a message landing in Trash does not say
        // "New mail in Trash".
        let arrival = || MailArrivalFfi {
            mailbox: 9,
            messages: vec![1],
        };
        let decide = |role, roles: Vec<String>| {
            decide_notification(
                arrival(),
                None,
                false,
                Some("Trash".to_owned()),
                role,
                roles,
            )
        };
        assert_eq!(
            decide(MailboxRoleFfi::Trash, inbox_only()),
            NotificationDecisionFfi::Suppress {
                reason: SuppressedFfi::NotWatched
            }
        );
        assert!(matches!(
            decide(
                MailboxRoleFfi::Trash,
                vec!["inbox".to_owned(), "trash".to_owned()]
            ),
            NotificationDecisionFfi::Deliver { .. }
        ));
        assert!(matches!(
            decide(MailboxRoleFfi::Inbox, inbox_only()),
            NotificationDecisionFfi::Deliver { .. }
        ));
    }

    #[test]
    fn the_boundary_carries_the_reason_a_notification_was_suppressed() {
        let arrival = |messages: Vec<i64>| MailArrivalFfi {
            mailbox: 5,
            messages,
        };
        assert_eq!(
            decide_notification(
                arrival(vec![]),
                None,
                false,
                None,
                MailboxRoleFfi::Inbox,
                inbox_only()
            ),
            NotificationDecisionFfi::Suppress {
                reason: SuppressedFfi::NothingArrived
            }
        );
        assert_eq!(
            decide_notification(
                arrival(vec![10]),
                Some(5),
                true,
                None,
                MailboxRoleFfi::Inbox,
                inbox_only()
            ),
            NotificationDecisionFfi::Suppress {
                reason: SuppressedFfi::AlreadyOnScreen
            }
        );
    }
}
