use crate::facts::Facts;
use crate::input::FiledMessage;

/// Any one guard is enough. A message with no sender or no conversation, or whose facts cannot be
/// read (`Facts` answers `true`), is guarded too: when in doubt mail goes to the inbox.
/// Cheapest question first, stopping at the first yes: the filing pass pays for each on every
/// message.
pub(crate) fn guarded(filed: &FiledMessage<'_>, facts: &dyn Facts) -> bool {
    let Some(thread) = filed.thread else {
        return true;
    };
    let senders = &filed.message.from;
    senders.is_empty()
        || senders.iter().any(|sender| facts.never_filter(sender))
        || senders.iter().any(|sender| facts.own_domain(sender))
        || senders.iter().any(|sender| facts.wrote_to(sender))
        || facts.took_part(thread)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use chrono::Utc;
    use postio_model::{AccountId, EmailAddress, MailboxId, MailboxRole, Message, ThreadId};

    use super::*;

    #[derive(Default)]
    struct Known {
        wrote_to: Vec<&'static str>,
        took_part: Vec<ThreadId>,
        own_domains: Vec<&'static str>,
        never: Vec<&'static str>,
        asked: RefCell<Vec<&'static str>>,
    }

    impl Facts for Known {
        fn wrote_to(&self, address: &EmailAddress) -> bool {
            self.asked.borrow_mut().push("wrote_to");
            self.wrote_to
                .iter()
                .any(|known| address.address.eq_ignore_ascii_case(known))
        }
        fn took_part(&self, thread: ThreadId) -> bool {
            self.asked.borrow_mut().push("took_part");
            self.took_part.contains(&thread)
        }
        fn own_domain(&self, address: &EmailAddress) -> bool {
            self.asked.borrow_mut().push("own_domain");
            address.domain().is_some_and(|domain| {
                self.own_domains
                    .iter()
                    .any(|own| domain.eq_ignore_ascii_case(own))
            })
        }
        fn never_filter(&self, address: &EmailAddress) -> bool {
            self.asked.borrow_mut().push("never_filter");
            self.never
                .iter()
                .any(|known| address.address.eq_ignore_ascii_case(known))
        }
    }

    struct Unreadable;

    impl Facts for Unreadable {
        fn wrote_to(&self, _: &EmailAddress) -> bool {
            true
        }
        fn took_part(&self, _: ThreadId) -> bool {
            true
        }
        fn own_domain(&self, _: &EmailAddress) -> bool {
            true
        }
        fn never_filter(&self, _: &EmailAddress) -> bool {
            true
        }
    }

    const THREAD: ThreadId = ThreadId::new(7);

    fn filed(message: &Message) -> FiledMessage<'_> {
        FiledMessage {
            message,
            thread: message.thread_id,
            role: MailboxRole::Inbox,
        }
    }

    fn from(addresses: &[&str]) -> Message {
        let mut message = Message::new(AccountId::new(1), MailboxId::new(1), Utc::now());
        message.from = addresses
            .iter()
            .map(|address| EmailAddress::new(None::<&str>, *address))
            .collect();
        message.to = vec![EmailAddress::new(None::<&str>, "ada.norwood@example.com")];
        message.thread_id = Some(THREAD);
        message
    }

    #[test]
    fn each_guard_alone_keeps_mail_in_the_inbox() {
        let rows: [(&str, Known, bool); 6] = [
            ("nothing is known of the sender", Known::default(), false),
            (
                "the user wrote to the sender",
                Known {
                    wrote_to: vec!["deals@shop.example"],
                    ..Known::default()
                },
                true,
            ),
            (
                "the user took part in the conversation",
                Known {
                    took_part: vec![THREAD],
                    ..Known::default()
                },
                true,
            ),
            (
                "the sender is at the user's own domain",
                Known {
                    own_domains: vec!["shop.example"],
                    ..Known::default()
                },
                true,
            ),
            (
                "the user pinned the sender",
                Known {
                    never: vec!["deals@shop.example"],
                    ..Known::default()
                },
                true,
            ),
            (
                "the user wrote to somebody else there",
                Known {
                    wrote_to: vec!["tove@shop.example"],
                    took_part: vec![ThreadId::new(8)],
                    ..Known::default()
                },
                false,
            ),
        ];

        for (case, known, expected) in rows {
            assert_eq!(
                guarded(&filed(&from(&["deals@shop.example"])), &known),
                expected,
                "{case}"
            );
        }
    }

    #[test]
    fn any_of_several_senders_is_enough() {
        let known = Known {
            wrote_to: vec!["tove@example.org"],
            ..Known::default()
        };

        assert!(guarded(
            &filed(&from(&["deals@shop.example", "tove@example.org"])),
            &known
        ));
    }

    #[test]
    fn a_store_that_cannot_answer_keeps_mail_in_the_inbox() {
        assert!(guarded(&filed(&from(&["deals@shop.example"])), &Unreadable));
    }

    #[test]
    fn a_message_the_guards_cannot_ask_about_is_guarded() {
        let anonymous = from(&[]);
        let unthreaded = from(&["deals@shop.example"]);

        assert!(guarded(&filed(&anonymous), &Known::default()));
        assert!(guarded(
            &FiledMessage {
                thread: None,
                ..filed(&unthreaded)
            },
            &Known::default()
        ));
    }

    #[test]
    fn the_conversation_is_the_one_filing_says_it_joined() {
        // Sync threads an arrival after it is stored, so the row has no `thread_id`; a guard that
        // read
        // the row would guard every arrival and nothing would be filtered.
        let mut arrival = from(&["deals@shop.example"]);
        arrival.thread_id = None;

        assert!(!guarded(
            &FiledMessage {
                thread: Some(THREAD),
                ..filed(&arrival)
            },
            &Known::default()
        ));
        assert!(guarded(
            &FiledMessage {
                thread: Some(THREAD),
                ..filed(&arrival)
            },
            &Known {
                took_part: vec![THREAD],
                ..Known::default()
            }
        ));
    }

    #[test]
    fn the_guards_ask_the_cheap_questions_first_and_stop_at_a_yes() {
        let pinned = Known {
            never: vec!["deals@shop.example"],
            ..Known::default()
        };
        guarded(&filed(&from(&["deals@shop.example"])), &pinned);
        assert_eq!(*pinned.asked.borrow(), ["never_filter"]);

        let stranger = Known::default();
        guarded(&filed(&from(&["deals@shop.example"])), &stranger);
        assert_eq!(
            *stranger.asked.borrow(),
            ["never_filter", "own_domain", "wrote_to", "took_part"]
        );
    }
}
