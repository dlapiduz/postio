//! Whether new mail is worth interrupting for and what the notification may say, decided once for
//! every frontend; delivery stays with the toolkit.
//! Wording is per platform: `Counts` never carries message content (a lock screen reads it out),
//! `Newest` names the newest arrival.
//! Every notification for a mailbox reuses one `identifier`, so successive wake-ups replace rather
//! than stack.

use postio_config::SyncConfig;
use postio_model::{MailboxId, MailboxRole, MessageId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrival {
    pub mailbox: MailboxId,
    pub messages: Vec<MessageId>,
}

/// Both halves are needed to suppress: a folder left open behind another application is not one the
/// user is watching.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attention {
    pub showing: Option<MailboxId>,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wording<'a> {
    Counts {
        mailbox_name: Option<&'a str>,
    },
    Newest {
        message: MessageId,
        from: Option<&'a str>,
        subject: Option<&'a str>,
        /// Named only when more than one account is enabled (#189); naming the only account is
        /// noise.
        account: Option<&'a str>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suppressed {
    /// The engine can emit a `NewMail` whose messages were all already known.
    NothingArrived,
    AlreadyOnScreen,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Suppress(Suppressed),
    Deliver(Notification),
}

/// Serialisable because the store's owner decides it and the frontend that delivers it is another
/// process (ADR 0041).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Notification {
    pub identifier: String,
    pub title: String,
    pub body: String,
    pub mailbox: MailboxId,
    pub message: Option<MessageId>,
}

pub fn watched(config: &SyncConfig, role: MailboxRole) -> bool {
    config.notify && role_watched(&config.notify_roles, role)
}

pub fn role_watched(notify_roles: &[String], role: MailboxRole) -> bool {
    notify_roles.iter().any(|name| name == role.as_str())
}

pub fn identifier(mailbox: MailboxId) -> String {
    format!("new-mail-{}", mailbox.get())
}

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
            // A single arrival names its message so the click lands on that row; a burst opens the
            // folder.
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
        let notification = delivered(decide(&arrival(1, &[42]), elsewhere(), COUNTS));
        let drawn = format!("{} {}", notification.title, notification.body);
        assert!(!drawn.contains("42"), "{drawn:?}");
    }

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
