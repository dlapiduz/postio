//! Layer 1, the guards (FR-111): mail from somebody the user deals with is
//! never filtered, whatever a rule or a classifier says of it.

use postio_model::Message;

use crate::facts::Facts;

/// Whether `message` is guarded against filtering (FR-111): the user
/// pinned a sender of it or restored mail from one (`[focus.filter]
/// never`), a sender is at one of the user's own domains, the user has
/// written to a sender, or the user took part in its conversation. Any one
/// is enough.
///
/// A message the guards cannot ask about, with no sender or no
/// conversation, is guarded too: nothing says it is safe to filter, and
/// when in doubt mail goes to the inbox (FR-112). So is one whose facts
/// cannot be read, since [`Facts`] answers `true` when it cannot answer.
///
/// The questions go cheapest first and stop at the first yes, because the
/// filing pass pays for each on every new message: the config and the
/// identities are in memory, `correspondents` is a seek, and the
/// conversation is an `EXISTS` (research R8).
pub(crate) fn guarded(message: &Message, facts: &dyn Facts) -> bool {
    let Some(thread) = message.thread_id else {
        return true;
    };
    let senders = &message.from;
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
    use postio_model::{AccountId, EmailAddress, MailboxId, ThreadId};

    use super::*;

    /// What the store and the config know, as lists; and which questions
    /// were asked, in order.
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

    /// A store that cannot answer: its reads fail, and the contract says a
    /// failed read answers `true` (`Facts`).
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

    /// Mail from `from`, in thread 7.
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
        // FR-111, and US9 scenarios 2 and 3: one guard is enough.
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
                guarded(&from(&["deals@shop.example"]), &known),
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
            &from(&["deals@shop.example", "tove@example.org"]),
            &known
        ));
    }

    #[test]
    fn a_store_that_cannot_answer_keeps_mail_in_the_inbox() {
        // Facts says a failed read answers true: every question is a reason
        // not to act, so when in doubt mail goes to the inbox (FR-112).
        assert!(guarded(&from(&["deals@shop.example"]), &Unreadable));
    }

    #[test]
    fn a_message_the_guards_cannot_ask_about_is_guarded() {
        // No sender to ask after, or no conversation yet: the guards cannot
        // say it is safe to filter, so it is not filtered.
        let anonymous = from(&[]);
        let mut unthreaded = from(&["deals@shop.example"]);
        unthreaded.thread_id = None;

        assert!(guarded(&anonymous, &Known::default()));
        assert!(guarded(&unthreaded, &Known::default()));
    }

    #[test]
    fn the_guards_ask_the_cheap_questions_first_and_stop_at_a_yes() {
        // The filing pass pays for every question on every new message: the
        // config and the identities are in memory, `correspondents` is a
        // seek, and the conversation is an EXISTS (research R8).
        let pinned = Known {
            never: vec!["deals@shop.example"],
            ..Known::default()
        };
        guarded(&from(&["deals@shop.example"]), &pinned);
        assert_eq!(*pinned.asked.borrow(), ["never_filter"]);

        let stranger = Known::default();
        guarded(&from(&["deals@shop.example"]), &stranger);
        assert_eq!(
            *stranger.asked.borrow(),
            ["never_filter", "own_domain", "wrote_to", "took_part"]
        );
    }
}
