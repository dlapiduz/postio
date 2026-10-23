//! New-mail notifications at the boundary: the decision, not the delivery.
//!
//! [`postio_ui::notify`] is the one rule for whether an arrival is worth
//! interrupting somebody for, which id coalesces it and where a click
//! lands; `decide_notification` is that rule with the macOS app's wording,
//! [`Wording::Counts`] — ids and counts only, because a notification there
//! is a log the lock screen reads out. The frontend's `MailNotifier` is a
//! shim over this, and `UNUserNotificationCenter` is the only thing it adds.
//!
//! The master switch (`[sync] notify`) crosses as `Session::notices` and
//! the frontend applies it. The role gate (`[sync] notify_roles`) is decided
//! here, by the same rule the host uses (`postio_ui::notify::role_watched`):
//! the frontend passes the folder's role and the roles `Session::notices`
//! reports, so the app notifies only for the folders the person chose.

use postio_model::{MailboxId, MessageId};

use crate::MailboxRoleFfi;
use postio_ui::notify::{self, Attention, Decision, Suppressed, Wording};

/// New mail that arrived, as `UiEvent::NewMail` reports it: ids only.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MailArrivalFfi {
    /// The folder it landed in.
    pub mailbox: i64,
    /// The newly delivered messages.
    pub messages: Vec<i64>,
}

/// Why an arrival did not become a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SuppressedFfi {
    /// The event carried no new messages.
    NothingArrived,
    /// It landed in the folder the user is looking at, right now.
    AlreadyOnScreen,
    /// `[sync] notify_roles` does not list the folder's role.
    NotWatched,
}

/// One notification, ready to hand the notification centre.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MailNotificationFfi {
    /// Stable per folder, so a second batch replaces the one on screen.
    pub identifier: String,
    /// What it says: counts and a folder name, never message content.
    pub title: String,
    /// The rest of what it says.
    pub body: String,
    /// Where a click should land.
    pub mailbox: i64,
    /// The one message a single arrival is about, if it is one.
    pub message: Option<i64>,
}

/// What to do about an arrival.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum NotificationDecisionFfi {
    /// Nothing, and why.
    Suppress {
        /// Why not.
        reason: SuppressedFfi,
    },
    /// Post this.
    Deliver {
        /// What to post.
        notification: MailNotificationFfi,
    },
}

/// Whether `arrival` becomes a notification, and what it says.
///
/// `showing` is the folder the list currently has open and `active` is
/// whether Postio is the frontmost application; both are needed to suppress
/// (see [`Attention`]). `mailbox_name` is the folder's display name when the
/// frontend knows it. `role` is the folder's role and `notify_roles` is
/// `[sync] notify_roles`: an arrival in a folder whose role is not listed is
/// [`SuppressedFfi::NotWatched`].
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
        // `[sync] notify_roles` defaults to the inbox. The macOS app used to
        // notify for every folder, so a message landing in Trash said
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
