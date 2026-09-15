//! Whether new mail is worth interrupting somebody for, and what the
//! notification may say — decided once, for every frontend.
//!
//! `postio-app` and the macOS app each had a copy of this decision, and
//! they had already drifted: the macOS copy suppressed mail landing in the
//! folder on screen and the GTK copy did not; one pointed a burst's click
//! at the folder and the other at the newest message. This module is the
//! one rule both call. The delivery — `gio::Notification` on one side,
//! `UNUserNotificationCenter` on the other — stays with the toolkit, thin.
//!
//! # What is shared and what is deliberately not
//!
//! The suppression, the identifier, and where a click lands are one rule.
//! **The wording is a per-platform choice**, carried in by [`Wording`]:
//!
//! - [`Wording::Counts`] says counts and a folder name, never message
//!   content. It is what the macOS app draws, because a notification there
//!   is a log the lock screen reads out, and `PRODUCT.md`'s rule that logs
//!   carry ids, counts and outcomes only applies to it more than anywhere.
//! - [`Wording::Newest`] names the newest arrival's sender and subject, plus
//!   how many more came with it. It is what the GTK app draws, where the
//!   desktop shell keeps banners off the lock screen and a popup that says
//!   nothing about the mail is not worth the interruption (#745).
//!
//! The click lands on the message the notification named, and on the
//! folder when it named none: under `Newest` that is always the newest
//! arrival, under `Counts` only a single arrival names one.
//!
//! # Coalescing
//!
//! Every notification for one mailbox reuses the same [`identifier`], which
//! both `gio::Application::send_notification` and `UNUserNotificationCenter`
//! treat as "replace the one already showing" rather than "queue another
//! beside it" — so several `IDLE` wake-ups in a row settle into one popup
//! saying the current state.

use postio_config::SyncConfig;
use postio_model::{MailboxId, MailboxRole, MessageId};

/// New mail as `Event::NewMail` reports it: ids and nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrival {
    /// The folder it landed in.
    pub mailbox: MailboxId,
    /// The newly delivered messages.
    pub messages: Vec<MessageId>,
}

/// What the person is looking at at the moment the mail lands.
///
/// **Both** halves are needed to suppress: a folder left open behind
/// another application is not one the user is watching, and treating "this
/// is the open mailbox" as sufficient is the version of this check that
/// silently swallows the notification somebody actually needed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attention {
    /// The mailbox the list has open, if any.
    pub showing: Option<MailboxId>,
    /// Whether Postio is the frontmost application.
    pub active: bool,
}

/// How much a notification may say. See the module docs for why this is
/// the frontend's choice rather than this module's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wording<'a> {
    /// Counts and the folder's name, never content.
    Counts {
        /// The folder's display name, when the frontend knows it.
        mailbox_name: Option<&'a str>,
    },
    /// The newest arrival's sender and subject.
    Newest {
        /// The arrival to name — whichever `received_at` is latest, which
        /// the caller decides because it is the one holding the rows.
        message: MessageId,
        /// Its sender, as the frontend displays addresses.
        from: Option<&'a str>,
        /// Its subject line.
        subject: Option<&'a str>,
        /// The account's name, when more than one is enabled (#189);
        /// naming the only account there is would be noise (ADR 0005 Q13).
        account: Option<&'a str>,
    },
}

/// Why an arrival did not become a notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suppressed {
    /// The event carried no new messages — the engine can emit a `NewMail`
    /// whose messages were all already known, and interrupting somebody to
    /// say nothing happened is the worst available outcome.
    NothingArrived,
    /// It landed in the folder the user is looking at, right now.
    AlreadyOnScreen,
}

/// What to do about an arrival.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Nothing, and why.
    Suppress(Suppressed),
    /// Post this.
    Deliver(Notification),
}

/// One notification, ready for the toolkit to post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    /// Stable per folder — see the module docs on coalescing.
    pub identifier: String,
    /// What it says.
    pub title: String,
    /// The rest of what it says.
    pub body: String,
    /// The folder a click opens.
    pub mailbox: MailboxId,
    /// The message a click lands on, when the notification named one.
    pub message: Option<MessageId>,
}

/// Whether `[sync]` says `role`'s arrivals are worth a notification.
///
/// Asked before [`decide`], by whoever holds the config: the role gate is
/// the one input that costs a store read to answer, and a frontend that
/// fails it has no reason to go on and read the messages.
pub fn watched(config: &SyncConfig, role: MailboxRole) -> bool {
    config.notify && config.notify_roles.iter().any(|name| name == role.as_str())
}

/// A notification id scoped to one mailbox.
pub fn identifier(mailbox: MailboxId) -> String {
    format!("new-mail-{}", mailbox.get())
}

/// Whether `arrival` becomes a notification, and what it says.
pub fn decide(arrival: &Arrival, attention: Attention, wording: Wording<'_>) -> Decision {
    let count = arrival.messages.len();
    if count == 0 {
        return Decision::Suppress(Suppressed::NothingArrived);
    }
    if attention.active && attention.showing == Some(arrival.mailbox) {
        return Decision::Suppress(Suppressed::AlreadyOnScreen);
    }
    let (title, body, message) = match wording {
        Wording::Counts { mailbox_name } => {
            let title = match mailbox_name {
                Some(name) => format!("New mail in {name}"),
                None => "New mail".to_owned(),
            };
            let body = if count == 1 {
                "1 new message".to_owned()
            } else {
                format!("{count} new messages")
            };
            // A single arrival names its message so the click lands on that
            // row; a burst's count picks no message, so the click opens the
            // folder, exactly as choosing it in the sidebar would.
            let message = (count == 1).then(|| arrival.messages[0]);
            (title, body, message)
        }
        Wording::Newest {
            message,
            from,
            subject,
            account,
        } => {
            let from = from.unwrap_or("Someone");
            let title = match account {
                Some(name) => format!("{from} — {name}"),
                None => from.to_owned(),
            };
            let subject = subject.unwrap_or("(no subject)");
            let body = if count == 1 {
                subject.to_owned()
            } else {
                format!("\"{subject}\" and {} more", count - 1)
            };
            (title, body, Some(message))
        }
    };
    Decision::Deliver(Notification {
        identifier: identifier(arrival.mailbox),
        title,
        body,
        mailbox: arrival.mailbox,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrival(mailbox: i64, messages: &[i64]) -> Arrival {
        Arrival {
            mailbox: MailboxId::new(mailbox),
            messages: messages.iter().map(|id| MessageId::new(*id)).collect(),
        }
    }

    fn elsewhere() -> Attention {
        Attention {
            showing: Some(MailboxId::new(99)),
            active: false,
        }
    }

    const COUNTS: Wording<'static> = Wording::Counts {
        mailbox_name: Some("Inbox"),
    };

    fn newest(message: i64, from: &'static str, subject: &'static str) -> Wording<'static> {
        Wording::Newest {
            message: MessageId::new(message),
            from: Some(from),
            subject: Some(subject),
            account: None,
        }
    }

    fn delivered(decision: Decision) -> Notification {
        match decision {
            Decision::Deliver(notification) => notification,
            Decision::Suppress(reason) => panic!("expected a notification, got {reason:?}"),
        }
    }

    // The suppression rule, the same on every platform.

    #[test]
    fn an_empty_arrival_is_not_a_notification() {
        assert_eq!(
            decide(&arrival(1, &[]), elsewhere(), COUNTS),
            Decision::Suppress(Suppressed::NothingArrived)
        );
    }

    #[test]
    fn mail_landing_in_the_folder_on_screen_in_front_of_you_is_not_news() {
        let attention = Attention {
            showing: Some(MailboxId::new(5)),
            active: true,
        };
        assert_eq!(
            decide(&arrival(5, &[10]), attention, COUNTS),
            Decision::Suppress(Suppressed::AlreadyOnScreen)
        );
        assert_eq!(
            decide(&arrival(5, &[10]), attention, newest(10, "Ada", "One")),
            Decision::Suppress(Suppressed::AlreadyOnScreen),
            "the wording does not change whether it is news"
        );
    }

    #[test]
    fn the_same_folder_in_a_window_you_are_not_looking_at_still_notifies() {
        // "On screen" means both halves. A folder left open behind another
        // application is not something the user is watching, and this is
        // the case a naive "is this the open mailbox" check gets wrong.
        let attention = Attention {
            showing: Some(MailboxId::new(5)),
            active: false,
        };
        assert!(matches!(
            decide(&arrival(5, &[10]), attention, COUNTS),
            Decision::Deliver(_)
        ));
    }

    #[test]
    fn mail_landing_elsewhere_notifies_even_while_the_app_is_in_front() {
        let attention = Attention {
            showing: Some(MailboxId::new(99)),
            active: true,
        };
        assert!(matches!(
            decide(&arrival(5, &[10]), attention, COUNTS),
            Decision::Deliver(_)
        ));
    }

    // The identifier, the same on every platform.

    #[test]
    fn a_second_batch_for_one_folder_replaces_the_first_rather_than_stacking() {
        let first = delivered(decide(&arrival(7, &[1]), elsewhere(), COUNTS));
        let second = delivered(decide(&arrival(7, &[2, 3]), elsewhere(), COUNTS));
        assert_eq!(first.identifier, second.identifier);
        assert_eq!(first.identifier, identifier(MailboxId::new(7)));
        let other = delivered(decide(&arrival(8, &[4]), elsewhere(), COUNTS));
        assert_ne!(
            other.identifier, first.identifier,
            "a different folder is a different one"
        );
    }

    #[test]
    fn each_mailbox_gets_one_stable_notification_id() {
        assert_eq!(identifier(MailboxId::new(7)), "new-mail-7");
    }

    // Counts wording: what a lock screen may read out.

    #[test]
    fn a_burst_under_counts_is_one_notification_carrying_a_count() {
        let notification = delivered(decide(&arrival(1, &[1, 2, 3]), elsewhere(), COUNTS));
        assert_eq!(notification.body, "3 new messages");
        assert_eq!(
            notification.message, None,
            "a burst has no one message to point at, so the click opens the folder"
        );
        assert_eq!(notification.mailbox, MailboxId::new(1));
    }

    #[test]
    fn a_single_arrival_under_counts_names_the_message_it_is_about() {
        let notification = delivered(decide(&arrival(1, &[42]), elsewhere(), COUNTS));
        assert_eq!(notification.body, "1 new message");
        assert_eq!(
            notification.message,
            Some(MessageId::new(42)),
            "clicking it has somewhere to land"
        );
    }

    #[test]
    fn counts_name_the_folder_when_known_and_do_not_invent_one() {
        let named = delivered(decide(
            &arrival(1, &[10]),
            elsewhere(),
            Wording::Counts {
                mailbox_name: Some("Archive"),
            },
        ));
        assert_eq!(named.title, "New mail in Archive");
        let unnamed = delivered(decide(
            &arrival(1, &[10]),
            elsewhere(),
            Wording::Counts { mailbox_name: None },
        ));
        assert_eq!(unnamed.title, "New mail");
    }

    #[test]
    fn no_message_content_reaches_a_counts_notification() {
        // Not even the id is drawn at somebody, and this asserts nobody later
        // adds a lookup to "improve" it.
        let notification = delivered(decide(&arrival(1, &[42]), elsewhere(), COUNTS));
        let drawn = format!("{} {}", notification.title, notification.body);
        assert!(!drawn.contains("42"), "{drawn:?}");
    }

    // Newest wording: the sender and subject of the newest arrival.

    #[test]
    fn a_single_arrival_names_the_sender_and_the_subject() {
        let notification = delivered(decide(
            &arrival(1, &[42]),
            elsewhere(),
            newest(42, "Ada Lovelace", "Quarterly report"),
        ));
        assert_eq!(notification.title, "Ada Lovelace");
        assert_eq!(notification.body, "Quarterly report");
        assert_eq!(notification.message, Some(MessageId::new(42)));
    }

    #[test]
    fn a_burst_is_a_count_rather_than_one_popup_per_message() {
        let notification = delivered(decide(
            &arrival(1, &[1, 99, 2]),
            elsewhere(),
            newest(99, "Carol", "Three"),
        ));
        assert_eq!(notification.title, "Carol");
        assert_eq!(notification.body, "\"Three\" and 2 more");
        assert_eq!(
            notification.message,
            Some(MessageId::new(99)),
            "the click lands on the message the notification actually named"
        );
    }

    #[test]
    fn a_two_message_burst_says_one_more() {
        let notification = delivered(decide(
            &arrival(1, &[1, 2]),
            elsewhere(),
            newest(2, "Bob", "Two"),
        ));
        assert_eq!(notification.body, "\"Two\" and 1 more");
    }

    #[test]
    fn a_ten_message_burst_counts_the_rest() {
        let ids: Vec<i64> = (0..10).collect();
        let notification = delivered(decide(
            &arrival(1, &ids),
            elsewhere(),
            newest(9, "Sender 9", "Subject 9"),
        ));
        assert_eq!(notification.body, "\"Subject 9\" and 9 more");
    }

    #[test]
    fn a_missing_sender_or_subject_still_reads_as_a_sentence() {
        let only = delivered(decide(
            &arrival(1, &[42]),
            elsewhere(),
            Wording::Newest {
                message: MessageId::new(42),
                from: None,
                subject: None,
                account: None,
            },
        ));
        assert_eq!(only.title, "Someone");
        assert_eq!(only.body, "(no subject)");
        let burst = delivered(decide(
            &arrival(1, &[41, 42]),
            elsewhere(),
            Wording::Newest {
                message: MessageId::new(42),
                from: Some("Ada Lovelace"),
                subject: None,
                account: None,
            },
        ));
        assert_eq!(burst.body, "\"(no subject)\" and 1 more");
    }

    // #189: notifications name the account when more than one is configured.

    #[test]
    fn newest_names_the_account_when_one_is_given() {
        let single = delivered(decide(
            &arrival(1, &[42]),
            elsewhere(),
            Wording::Newest {
                message: MessageId::new(42),
                from: Some("Ada Lovelace"),
                subject: Some("Quarterly report"),
                account: Some("Work"),
            },
        ));
        assert_eq!(single.title, "Ada Lovelace — Work");
        let burst = delivered(decide(
            &arrival(1, &[41, 42]),
            elsewhere(),
            Wording::Newest {
                message: MessageId::new(42),
                from: Some("Bob"),
                subject: Some("Two"),
                account: Some("Work"),
            },
        ));
        assert_eq!(burst.title, "Bob — Work");
    }

    // The role gate.

    #[test]
    fn notify_settings_gate_on_both_the_switch_and_the_role() {
        let mut config = SyncConfig {
            notify: true,
            notify_roles: vec!["inbox".to_owned()],
            ..SyncConfig::default()
        };
        assert!(watched(&config, MailboxRole::Inbox));
        assert!(
            !watched(&config, MailboxRole::Archive),
            "archive was never asked for"
        );
        config.notify = false;
        assert!(
            !watched(&config, MailboxRole::Inbox),
            "the master switch must override an explicitly listed role"
        );
    }

    #[test]
    fn a_role_this_build_does_not_recognise_is_just_never_matched() {
        let config = SyncConfig {
            notify: true,
            notify_roles: vec!["not-a-real-role".to_owned()],
            ..SyncConfig::default()
        };
        assert!(!watched(&config, MailboxRole::Inbox));
    }
}
