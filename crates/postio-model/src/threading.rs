//! JWZ threading, one message at a time.
//!
//! # Why not the batch algorithm
//!
//! Jamie Zawinski's algorithm is written as a batch pass: build a table of
//! containers keyed by `Message-ID`, link each message to its parent, prune
//! the empty containers, then group what is left by subject. That is the right
//! shape for threading a mailbox you have just been handed, and the wrong shape
//! for Postio, where messages arrive one at a time forever. Rethreading a
//! mailbox because one message came in is the difference between an inbox that
//! opens instantly and one that stalls (`CLAUDE.md`: never load a whole mailbox
//! into memory).
//!
//! So the *linkage rule* is kept and the *pass* is discarded. [`assign`] answers
//! one question — which thread does this message belong to — by asking a
//! [`ThreadIndex`] about the ids the message names. That is a lookup per
//! reference, and a message's reference chain is the length of its conversation,
//! not the length of its mailbox.
//!
//! # Out-of-order arrival, and merging
//!
//! Nothing guarantees a parent arrives before its children: a message can be
//! moved into a mailbox long after the replies to it, and an initial sync walks
//! newest-first on purpose. So a thread claims *every id it mentions*, present
//! or not, and a message that turns up later matching one of those ids joins the
//! thread that was waiting for it.
//!
//! That is also what makes merging fall out for free. When a message names ids
//! held by two different threads — the late-arriving parent that finally links
//! two halves of a conversation — [`assign`] returns [`Assignment::Merge`] and
//! the caller folds one into the other. No separate detection pass, and no way
//! for the two to disagree.
//!
//! # Subject grouping is a fallback, and a narrow one
//!
//! A mailing list that rewrites headers, or a client that replies without
//! `In-Reply-To`, leaves a message with no usable link. JWZ's answer is to fall
//! back to the subject — but only where one side looks like a reply, or every
//! message anyone titled "Hello" ends up in one conversation. [`assign`] applies
//! the fallback only when there is no reference link at all *and* the subject
//! carries reply decoration ([`is_reply`](crate::subject::is_reply)).

use crate::ids::{RfcMessageId, ThreadId};
use crate::message::Message;
use crate::subject::{is_reply, normalize_subject};

/// Everything about a message that bears on which thread it belongs to.
///
/// Built from a [`Message`] by [`ThreadCue::of`], but constructible on its own
/// so the rule can be tested without inventing a whole message.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThreadCue {
    /// The message's own `Message-ID`, when it has one.
    pub message_id: Option<RfcMessageId>,
    /// Its ancestors, oldest first — `References` with `In-Reply-To` appended
    /// when that is not already the last entry. See
    /// [`Message::reference_chain`].
    pub references: Vec<RfcMessageId>,
    /// The normalized subject, for the fallback.
    pub subject: String,
    /// Whether the subject carried reply or forward decoration.
    pub is_reply: bool,
}

impl ThreadCue {
    /// Reads the cue out of a message.
    pub fn of(message: &Message) -> Self {
        let subject = message.subject.as_deref().unwrap_or_default();
        Self {
            message_id: message.rfc_message_id.clone(),
            references: message.reference_chain().cloned().collect(),
            subject: normalize_subject(subject),
            is_reply: is_reply(subject),
        }
    }

    /// Every id that could tie this message to an existing thread, nearest
    /// first.
    ///
    /// The message's own id leads: a thread may already be holding replies that
    /// name it, and the point of looking is to find them. Then the references
    /// in reverse, because the immediate parent is a better answer than a
    /// distant ancestor when the two somehow disagree.
    pub fn links(&self) -> impl Iterator<Item = &RfcMessageId> {
        self.message_id.iter().chain(self.references.iter().rev())
    }

    /// Whether the subject may be used to place this message.
    fn subject_is_usable(&self) -> bool {
        self.is_reply && !self.subject.is_empty()
    }
}

/// What the caller knows about the threads it has already stored.
///
/// Implemented over SQLite by `postio-storage`, and over a map in this module's
/// tests. Both answers must be cheap: [`assign`] calls the first once per
/// reference and the second at most once.
pub trait ThreadIndex {
    /// The thread that holds — or is waiting for — a message with this id.
    fn thread_of(&self, id: &RfcMessageId) -> Option<ThreadId>;

    /// Threads whose root subject normalizes to this, oldest first.
    ///
    /// Only consulted for the fallback, so an implementation that cannot
    /// answer cheaply may return nothing and lose only the repair of a broken
    /// `References` chain.
    fn threads_with_subject(&self, subject: &str) -> Vec<ThreadId>;
}

/// Where a message goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assignment {
    /// Nothing links it. Start a thread.
    New,
    /// It belongs to one existing thread.
    Join(ThreadId),
    /// It links threads that were separate until now.
    Merge {
        /// The thread that survives, and that the message joins.
        into: ThreadId,
        /// The threads to fold into it. Never empty, never contains `into`.
        absorb: Vec<ThreadId>,
    },
}

impl Assignment {
    /// The thread the message ends up in, when one already exists.
    pub fn thread(&self) -> Option<ThreadId> {
        match self {
            Self::New => None,
            Self::Join(id) => Some(*id),
            Self::Merge { into, .. } => Some(*into),
        }
    }
}

/// Decides where `cue` belongs.
///
/// Pure: the only knowledge of the world comes through `index`, which is what
/// lets the rule be tested exhaustively and the storage layer be tested
/// separately for whether its lookups are cheap.
pub fn assign(cue: &ThreadCue, index: &impl ThreadIndex) -> Assignment {
    let mut found: Vec<ThreadId> = Vec::new();
    for link in cue.links() {
        if let Some(thread) = index.thread_of(link)
            && !found.contains(&thread)
        {
            found.push(thread);
        }
    }

    if found.is_empty() && cue.subject_is_usable() {
        for thread in index.threads_with_subject(&cue.subject) {
            if !found.contains(&thread) {
                found.push(thread);
            }
        }
    }

    match found.len() {
        0 => Assignment::New,
        1 => Assignment::Join(found[0]),
        _ => {
            // The oldest thread survives — lowest id is created first. The user
            // has been looking at it the longest, and picking by age rather
            // than by discovery order keeps the outcome the same however the
            // references happened to be written.
            let into = *found.iter().min().expect("checked non-empty");
            let mut absorb: Vec<ThreadId> = found.into_iter().filter(|id| *id != into).collect();
            // Ascending rather than in the order the references happened to
            // mention them: the caller folds these one at a time, and a stable
            // order is one less thing for it to be surprised by.
            absorb.sort_unstable();
            Assignment::Merge { into, absorb }
        }
    }
}

/// Every id a thread should claim once `cue` is filed into it.
///
/// Both the message's own id and everything it references: claiming the
/// references is what lets a parent arriving later find the thread its children
/// already made. Ids the caller has already recorded are harmless to record
/// again.
pub fn claimed_ids(cue: &ThreadCue) -> impl Iterator<Item = &RfcMessageId> {
    cue.message_id.iter().chain(cue.references.iter())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// A [`ThreadIndex`] over two maps, which is all the storage layer is.
    #[derive(Debug, Default)]
    struct Index {
        by_id: BTreeMap<String, ThreadId>,
        by_subject: BTreeMap<String, Vec<ThreadId>>,
    }

    impl Index {
        /// Files a cue into `thread`, claiming what it names — the same
        /// bookkeeping the storage layer does.
        fn file(&mut self, cue: &ThreadCue, thread: ThreadId) {
            for id in claimed_ids(cue) {
                self.by_id.insert(id.as_str().to_lowercase(), thread);
            }
            if !cue.subject.is_empty() {
                let threads = self.by_subject.entry(cue.subject.clone()).or_default();
                if !threads.contains(&thread) {
                    threads.push(thread);
                }
            }
        }

        fn merge(&mut self, into: ThreadId, absorb: &[ThreadId]) {
            for thread in self.by_id.values_mut() {
                if absorb.contains(thread) {
                    *thread = into;
                }
            }
            for threads in self.by_subject.values_mut() {
                for thread in threads.iter_mut() {
                    if absorb.contains(thread) {
                        *thread = into;
                    }
                }
                threads.dedup();
            }
        }
    }

    impl ThreadIndex for Index {
        fn thread_of(&self, id: &RfcMessageId) -> Option<ThreadId> {
            self.by_id.get(&id.as_str().to_lowercase()).copied()
        }

        fn threads_with_subject(&self, subject: &str) -> Vec<ThreadId> {
            self.by_subject.get(subject).cloned().unwrap_or_default()
        }
    }

    fn id(raw: &str) -> RfcMessageId {
        RfcMessageId::new(raw)
    }

    fn cue(message_id: &str, references: &[&str], subject: &str) -> ThreadCue {
        ThreadCue {
            message_id: Some(id(message_id)),
            references: references.iter().map(|r| id(r)).collect(),
            subject: normalize_subject(subject),
            is_reply: is_reply(subject),
        }
    }

    /// Files a run of cues in order, as they would arrive.
    fn thread_them(cues: &[ThreadCue]) -> (Index, Vec<ThreadId>) {
        let mut index = Index::default();
        let mut next = 1;
        let mut assigned = Vec::new();

        for cue in cues {
            let thread = match assign(cue, &index) {
                Assignment::New => {
                    let thread = ThreadId::new(next);
                    next += 1;
                    thread
                }
                Assignment::Join(thread) => thread,
                Assignment::Merge { into, absorb } => {
                    index.merge(into, &absorb);
                    for slot in assigned.iter_mut() {
                        if absorb.contains(slot) {
                            *slot = into;
                        }
                    }
                    into
                }
            };
            index.file(cue, thread);
            assigned.push(thread);
        }
        (index, assigned)
    }

    // -----------------------------------------------------------------------
    // The linkage rule
    // -----------------------------------------------------------------------

    #[test]
    fn a_message_with_nothing_to_link_to_starts_a_thread() {
        let (_, threads) = thread_them(&[cue("<a@example.com>", &[], "Contract")]);

        assert_eq!(threads, vec![ThreadId::new(1)]);
    }

    #[test]
    fn a_reply_joins_the_thread_of_what_it_answers() {
        let (_, threads) = thread_them(&[
            cue("<a@example.com>", &[], "Contract"),
            cue("<b@example.com>", &["<a@example.com>"], "Re: Contract"),
            cue(
                "<c@example.com>",
                &["<a@example.com>", "<b@example.com>"],
                "Re: Contract",
            ),
        ]);

        assert_eq!(threads, vec![ThreadId::new(1); 3]);
    }

    #[test]
    fn two_unrelated_messages_stay_apart() {
        let (_, threads) = thread_them(&[
            cue("<a@example.com>", &[], "Contract"),
            cue("<b@example.com>", &[], "Invoice"),
        ]);

        assert_eq!(threads, vec![ThreadId::new(1), ThreadId::new(2)]);
    }

    #[test]
    fn message_ids_match_without_regard_to_case() {
        let (_, threads) = thread_them(&[
            cue("<A@Example.com>", &[], "Contract"),
            cue("<b@example.com>", &["<a@example.COM>"], "Re: Contract"),
        ]);

        assert_eq!(threads, vec![ThreadId::new(1); 2]);
    }

    // -----------------------------------------------------------------------
    // Out of order, and merging
    // -----------------------------------------------------------------------

    #[test]
    fn a_reply_that_arrives_first_still_gathers_its_parent() {
        // An initial sync walks newest first, so this is the ordinary case and
        // not the exotic one.
        let (_, threads) = thread_them(&[
            cue("<b@example.com>", &["<a@example.com>"], "Re: Contract"),
            cue("<a@example.com>", &[], "Contract"),
        ]);

        assert_eq!(
            threads,
            vec![ThreadId::new(1); 2],
            "the thread was already claiming the id it was waiting for"
        );
    }

    #[test]
    fn a_late_parent_merges_the_two_halves_it_links() {
        // Two replies arrive with no common reference, so they look unrelated…
        let mut index = Index::default();
        let first = cue("<b@example.com>", &["<a@example.com>"], "Re: Contract");
        let second = cue("<c@example.com>", &["<x@example.com>"], "Re: Notes");
        index.file(&first, ThreadId::new(1));
        index.file(&second, ThreadId::new(2));

        // …until the message that references both turns up.
        let late = cue(
            "<a@example.com>",
            &["<x@example.com>"],
            "Contract and notes",
        );

        assert_eq!(
            assign(&late, &index),
            Assignment::Merge {
                into: ThreadId::new(1),
                absorb: vec![ThreadId::new(2)]
            }
        );
    }

    #[test]
    fn a_merge_survives_into_the_assignments_already_made() {
        let (_, threads) = thread_them(&[
            cue("<b@example.com>", &["<a@example.com>"], "Re: Contract"),
            cue("<c@example.com>", &["<x@example.com>"], "Re: Notes"),
            cue("<a@example.com>", &["<x@example.com>"], "Contract"),
        ]);

        assert_eq!(
            threads,
            vec![ThreadId::new(1); 3],
            "all three end up in the thread that survived"
        );
    }

    #[test]
    fn the_oldest_thread_survives_a_merge() {
        let mut index = Index::default();
        index.file(&cue("<b@example.com>", &[], "B"), ThreadId::new(7));
        index.file(&cue("<c@example.com>", &[], "C"), ThreadId::new(3));

        let joiner = cue(
            "<d@example.com>",
            &["<b@example.com>", "<c@example.com>"],
            "Re: B",
        );

        assert_eq!(
            assign(&joiner, &index),
            Assignment::Merge {
                into: ThreadId::new(3),
                absorb: vec![ThreadId::new(7)]
            },
            "the one the user has been looking at longest"
        );
    }

    #[test]
    fn three_threads_can_merge_at_once() {
        let mut index = Index::default();
        for (n, letter) in [(1, "a"), (2, "b"), (3, "c")] {
            index.file(
                &cue(&format!("<{letter}@example.com>"), &[], "X"),
                ThreadId::new(n),
            );
        }

        let joiner = cue(
            "<d@example.com>",
            &["<a@example.com>", "<b@example.com>", "<c@example.com>"],
            "Re: X",
        );

        assert_eq!(
            assign(&joiner, &index),
            Assignment::Merge {
                into: ThreadId::new(1),
                absorb: vec![ThreadId::new(2), ThreadId::new(3)]
            }
        );
    }

    // -----------------------------------------------------------------------
    // Broken chains
    // -----------------------------------------------------------------------

    #[test]
    fn a_reply_with_no_references_falls_back_to_its_subject() {
        // What a mailing list that rewrites headers leaves behind.
        let (_, threads) = thread_them(&[
            cue("<a@example.com>", &[], "Contract"),
            cue("<b@example.com>", &[], "Re: Contract"),
        ]);

        assert_eq!(threads, vec![ThreadId::new(1); 2]);
    }

    #[test]
    fn the_subject_fallback_needs_a_subject_that_looks_like_a_reply() {
        let (_, threads) = thread_them(&[
            cue("<a@example.com>", &[], "Hello"),
            cue("<b@example.com>", &[], "Hello"),
        ]);

        assert_eq!(
            threads,
            vec![ThreadId::new(1), ThreadId::new(2)],
            "two people saying Hello are not in a conversation"
        );
    }

    #[test]
    fn a_reference_beats_a_matching_subject() {
        let mut index = Index::default();
        index.file(&cue("<a@example.com>", &[], "Contract"), ThreadId::new(1));
        index.file(&cue("<x@example.com>", &[], "Contract"), ThreadId::new(2));

        let reply = cue("<b@example.com>", &["<x@example.com>"], "Re: Contract");

        assert_eq!(
            assign(&reply, &index),
            Assignment::Join(ThreadId::new(2)),
            "the chain says which conversation; the subject only guesses"
        );
    }

    #[test]
    fn a_message_with_no_id_at_all_can_still_be_placed_by_subject() {
        let mut index = Index::default();
        index.file(&cue("<a@example.com>", &[], "Contract"), ThreadId::new(1));

        let anonymous = ThreadCue {
            message_id: None,
            references: Vec::new(),
            subject: normalize_subject("Re: Contract"),
            is_reply: true,
        };

        assert_eq!(
            assign(&anonymous, &index),
            Assignment::Join(ThreadId::new(1))
        );
    }

    #[test]
    fn an_empty_subject_never_groups() {
        let mut index = Index::default();
        index.file(&cue("<a@example.com>", &[], "Re:"), ThreadId::new(1));

        let other = cue("<b@example.com>", &[], "Re:");

        assert_eq!(
            assign(&other, &index),
            Assignment::New,
            "`Re:` with nothing after it is not a topic"
        );
    }

    #[test]
    fn a_bracketed_list_tag_keeps_two_lists_apart() {
        let (_, threads) = thread_them(&[
            cue("<a@example.com>", &[], "[postio-dev] Contract"),
            cue("<b@example.com>", &[], "[other-list] Re: Contract"),
        ]);

        assert_eq!(threads, vec![ThreadId::new(1), ThreadId::new(2)]);
    }

    #[test]
    fn a_broken_middle_reference_still_reaches_the_root() {
        // The reply names an ancestor nobody ever delivered, plus the real root.
        let (_, threads) = thread_them(&[
            cue("<a@example.com>", &[], "Contract"),
            cue(
                "<c@example.com>",
                &["<a@example.com>", "<missing@example.com>"],
                "Re: Contract",
            ),
        ]);

        assert_eq!(threads, vec![ThreadId::new(1); 2]);
    }

    // -----------------------------------------------------------------------
    // Cues
    // -----------------------------------------------------------------------

    #[test]
    fn the_links_of_a_cue_run_nearest_first_behind_its_own_id() {
        let cue = cue(
            "<c@example.com>",
            &["<a@example.com>", "<b@example.com>"],
            "Re: X",
        );

        let links: Vec<&str> = cue.links().map(RfcMessageId::as_str).collect();
        assert_eq!(
            links,
            vec!["<c@example.com>", "<b@example.com>", "<a@example.com>"]
        );
    }

    #[test]
    fn a_cue_claims_its_own_id_and_everything_it_names() {
        let cue = cue("<c@example.com>", &["<a@example.com>"], "Re: X");

        let claimed: Vec<&str> = claimed_ids(&cue).map(RfcMessageId::as_str).collect();
        assert_eq!(claimed, vec!["<c@example.com>", "<a@example.com>"]);
    }

    #[test]
    fn a_cue_reads_the_reference_chain_a_message_already_computes() {
        use crate::ids::{AccountId, MailboxId};
        use chrono::Utc;

        let mut message = Message::new(AccountId::new(1), MailboxId::new(1), Utc::now());
        message.rfc_message_id = Some(id("<c@example.com>"));
        message.references = vec![id("<a@example.com>")];
        message.in_reply_to = Some(id("<b@example.com>"));
        message.subject = Some("Re: Contract".to_owned());

        let cue = ThreadCue::of(&message);

        assert_eq!(cue.message_id, Some(id("<c@example.com>")));
        assert_eq!(
            cue.references,
            vec![id("<a@example.com>"), id("<b@example.com>")],
            "In-Reply-To appended, as `reference_chain` gives it"
        );
        assert_eq!(cue.subject, "contract");
        assert!(cue.is_reply);
    }
}
