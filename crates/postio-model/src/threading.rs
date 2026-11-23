//! JWZ linkage, one message at a time: messages arrive forever, so rethreading a mailbox per
//! arrival would stall the inbox.
//! A thread claims every id it mentions, present or not, so out-of-order arrivals join the waiting
//! thread and a message naming ids of two threads is a `Merge`.
//! Subject grouping is a narrow fallback: no reference link and a reply-decorated subject, the one
//! newest thread within `SUBJECT_WINDOW`, never merging, skipping list and bulk mail.

use chrono::{DateTime, TimeDelta, Utc};

use crate::ids::{MessageId, RfcMessageId, ThreadId};
use crate::message::Message;
use crate::promoted::PRECEDENCE;
use crate::subject::{is_reply, normalize_subject};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThreadCue {
    pub message_id: Option<RfcMessageId>,
    pub references: Vec<RfcMessageId>,
    pub subject: String,
    pub is_reply: bool,
    pub date: DateTime<Utc>,
    /// List mail is grouped by references only; the subject fallback skips it.
    pub is_list: bool,
}

/// A thread joins by subject only when its latest message is within this of the new message's date.
pub const SUBJECT_WINDOW: TimeDelta = TimeDelta::days(7);

impl ThreadCue {
    pub fn of(message: &Message) -> Self {
        let subject = message.subject.as_deref().unwrap_or_default();
        Self {
            message_id: message.rfc_message_id.clone(),
            references: message.reference_chain().cloned().collect(),
            subject: normalize_subject(subject),
            is_reply: is_reply(subject),
            // Received times on both sides: a thread's `first_at` and `last_at`
            // are received times, so the window compares like with like.
            date: message.received_at,
            is_list: message.list_id.is_some()
                || message
                    .promoted
                    .is_some_and(|p| p.unsubscribe_offered || p.automation & PRECEDENCE != 0),
        }
    }

    /// The message's own id leads: a thread may already hold replies that name it. Then the
    /// references in reverse, because the immediate parent beats a distant ancestor.
    pub fn links(&self) -> impl Iterator<Item = &RfcMessageId> {
        self.message_id.iter().chain(self.references.iter().rev())
    }

    /// Public because an index that loads answers before `assign` runs must know which questions
    /// will be asked.
    pub fn subject_is_usable(&self) -> bool {
        self.is_reply && !self.subject.is_empty() && !self.is_list
    }

    /// Both ends, because an initial sync files newest first: the stored thread may begin after
    /// this message, and a month-later conversation is not this one.
    pub fn subject_window(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        (self.date - SUBJECT_WINDOW, self.date + SUBJECT_WINDOW)
    }
}

/// Both answers must be cheap: `assign` calls the first once per reference and the second at most
/// once.
pub trait ThreadIndex {
    fn thread_of(&self, id: &RfcMessageId) -> Option<ThreadId>;

    /// Only consulted for the fallback, so an implementation that cannot answer cheaply may return
    /// nothing and lose only the repair of a broken `References` chain.
    fn threads_with_subject(
        &self,
        subject: &str,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Option<ThreadId>;

    /// Whether `thread` was made by splitting a message off another: it claims the id of a message
    /// the person split.
    fn is_split(&self, thread: ThreadId) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadNode {
    pub id: MessageId,
    pub message_id: Option<RfcMessageId>,
    pub references: Vec<RfcMessageId>,
}

pub fn descendants(nodes: &[ThreadNode], root: MessageId) -> Vec<MessageId> {
    use std::collections::HashSet;

    // The ids of everything in the family so far, folded. Walking `nodes` in
    // order is enough when a reply follows what it answers; a reply that
    // sorts earlier than its parent (a clock that disagrees) is caught by
    // going round again until nothing more joins.
    let Some(start) = nodes.iter().find(|node| node.id == root) else {
        return Vec::new();
    };
    let mut family: HashSet<MessageId> = HashSet::from([root]);
    let mut ids: HashSet<String> = start.message_id.iter().map(RfcMessageId::folded).collect();
    loop {
        let before = family.len();
        for node in nodes {
            if family.contains(&node.id) {
                continue;
            }
            if node
                .references
                .iter()
                .any(|named| ids.contains(&named.folded()))
            {
                family.insert(node.id);
                ids.extend(node.message_id.iter().map(RfcMessageId::folded));
            }
        }
        if family.len() == before {
            break;
        }
    }
    nodes
        .iter()
        .map(|node| node.id)
        .filter(|id| family.contains(id))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assignment {
    New,
    Join(ThreadId),
    Merge {
        into: ThreadId,
        /// The threads to fold into it. Never empty, never contains `into`.
        absorb: Vec<ThreadId>,
    },
}

impl Assignment {
    pub fn thread(&self) -> Option<ThreadId> {
        match self {
            Self::New => None,
            Self::Join(id) => Some(*id),
            Self::Merge { into, .. } => Some(*into),
        }
    }
}

pub fn assign(cue: &ThreadCue, index: &impl ThreadIndex) -> Assignment {
    let mut found: Vec<ThreadId> = Vec::new();
    for link in cue.links() {
        if let Some(thread) = index.thread_of(link)
            && !found.contains(&thread)
        {
            found.push(thread);
        }
    }

    // The subject path only ever joins one thread: two conversations that
    // happen to share a subject are not one conversation, so it never merges.
    if found.is_empty()
        && cue.subject_is_usable()
        && let Some(thread) = {
            let (from, until) = cue.subject_window();
            index.threads_with_subject(&cue.subject, from, until)
        }
    {
        return Assignment::Join(thread);
    }

    match found.len() {
        0 => Assignment::New,
        1 => Assignment::Join(found[0]),
        // A split thread is the person's own doing: it is not folded back
        // into the thread it came out of by a message that names both. The
        // message joins the thread of its nearest link (`found` is in link
        // order), and nothing merges (spec 013 FR-007).
        _ if found.iter().any(|thread| index.is_split(*thread)) => Assignment::Join(found[0]),
        _ => {
            // The oldest thread survives (lowest id): the user has looked at it longest, and by age
            // the outcome is the same however the references were written.
            let into = *found.iter().min().expect("checked non-empty");
            let mut absorb: Vec<ThreadId> = found.into_iter().filter(|id| *id != into).collect();
            // Ascending, so the caller folding them one at a time gets a stable order.
            absorb.sort_unstable();
            Assignment::Merge { into, absorb }
        }
    }
}

/// Claiming the references lets a parent arriving later find the thread its children made.
pub fn claimed_ids(cue: &ThreadCue) -> impl Iterator<Item = &RfcMessageId> {
    cue.message_id.iter().chain(cue.references.iter())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Debug, Default)]
    struct Index {
        by_id: BTreeMap<String, ThreadId>,
        by_subject: BTreeMap<String, Vec<ThreadId>>,
        last_at: BTreeMap<ThreadId, DateTime<Utc>>,
        first_at: BTreeMap<ThreadId, DateTime<Utc>>,
        splits: Vec<ThreadId>,
    }

    impl Index {
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
            let last = self.last_at.entry(thread).or_insert(cue.date);
            *last = (*last).max(cue.date);
            let first = self.first_at.entry(thread).or_insert(cue.date);
            *first = (*first).min(cue.date);
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

        fn threads_with_subject(
            &self,
            subject: &str,
            from: DateTime<Utc>,
            until: DateTime<Utc>,
        ) -> Option<ThreadId> {
            self.by_subject
                .get(subject)?
                .iter()
                .copied()
                .filter(|thread| self.last_at[thread] >= from && self.first_at[thread] <= until)
                .max_by_key(|thread| (self.last_at[thread], *thread))
        }

        fn is_split(&self, thread: ThreadId) -> bool {
            self.splits.contains(&thread)
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
            date: DateTime::<Utc>::default(),
            is_list: false,
        }
    }

    fn on_day(day: i64, mut cue: ThreadCue) -> ThreadCue {
        cue.date = DateTime::<Utc>::default() + TimeDelta::days(day);
        cue
    }

    fn listed(mut cue: ThreadCue) -> ThreadCue {
        cue.is_list = true;
        cue
    }

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

    #[test]
    fn a_reply_that_arrives_first_still_gathers_its_parent() {
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
        let mut index = Index::default();
        let first = cue("<b@example.com>", &["<a@example.com>"], "Re: Contract");
        let second = cue("<c@example.com>", &["<x@example.com>"], "Re: Notes");
        index.file(&first, ThreadId::new(1));
        index.file(&second, ThreadId::new(2));

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

    #[test]
    fn a_reply_with_no_references_falls_back_to_its_subject() {
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
            ..ThreadCue::default()
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

    #[test]
    fn the_subject_fallback_joins_the_newest_thread_in_the_window_and_never_merges() {
        let mut index = Index::default();
        index.file(
            &on_day(0, cue("<a@example.com>", &[], "Hello")),
            ThreadId::new(1),
        );
        index.file(
            &on_day(2, cue("<b@example.com>", &[], "Hello")),
            ThreadId::new(2),
        );

        let reply = on_day(3, cue("<c@example.com>", &[], "Re: Hello"));

        assert_eq!(assign(&reply, &index), Assignment::Join(ThreadId::new(2)));
    }

    #[test]
    fn the_newest_thread_wins_whatever_its_id() {
        let mut index = Index::default();
        index.file(
            &on_day(2, cue("<a@example.com>", &[], "Hello")),
            ThreadId::new(1),
        );
        index.file(
            &on_day(0, cue("<b@example.com>", &[], "Hello")),
            ThreadId::new(2),
        );

        let reply = on_day(3, cue("<c@example.com>", &[], "Re: Hello"));

        assert_eq!(assign(&reply, &index), Assignment::Join(ThreadId::new(1)));
    }

    #[test]
    fn an_eight_day_gap_starts_a_new_thread() {
        let (_, threads) = thread_them(&[
            on_day(0, cue("<a@example.com>", &[], "Re: Hello")),
            on_day(8, cue("<b@example.com>", &[], "Re: Hello")),
        ]);

        assert_eq!(threads, vec![ThreadId::new(1), ThreadId::new(2)]);
    }

    #[test]
    fn a_seven_day_gap_still_joins() {
        let (_, threads) = thread_them(&[
            on_day(0, cue("<a@example.com>", &[], "Re: Hello")),
            on_day(7, cue("<b@example.com>", &[], "Re: Hello")),
        ]);

        assert_eq!(threads, vec![ThreadId::new(1); 2]);
    }

    #[test]
    fn the_window_runs_from_the_threads_latest_message() {
        let (_, threads) = thread_them(&[
            on_day(0, cue("<a@example.com>", &[], "Re: Hello")),
            on_day(6, cue("<b@example.com>", &[], "Re: Hello")),
            on_day(12, cue("<c@example.com>", &[], "Re: Hello")),
        ]);

        assert_eq!(threads, vec![ThreadId::new(1); 3]);
    }

    #[test]
    fn the_window_applies_in_both_directions() {
        let mut index = Index::default();
        index.file(
            &on_day(31, cue("<a@example.com>", &[], "Hello")),
            ThreadId::new(1),
        );
        index.file(
            &on_day(35, cue("<b@example.com>", &[], "Re: Hello")),
            ThreadId::new(1),
        );

        let earlier = on_day(5, cue("<c@example.com>", &[], "Re: Hello"));
        assert_eq!(assign(&earlier, &index), Assignment::New);

        let later = on_day(40, cue("<d@example.com>", &[], "Re: Hello"));
        assert_eq!(assign(&later, &index), Assignment::Join(ThreadId::new(1)));

        let just_before = on_day(25, cue("<e@example.com>", &[], "Re: Hello"));
        assert_eq!(
            assign(&just_before, &index),
            Assignment::Join(ThreadId::new(1)),
            "six days before the thread began is still inside the window"
        );
    }

    #[test]
    fn list_mail_never_groups_by_subject() {
        let (_, threads) = thread_them(&[
            on_day(0, listed(cue("<a@example.com>", &[], "Re: Weekly"))),
            on_day(1, listed(cue("<b@example.com>", &[], "Re: Weekly"))),
        ]);

        assert_eq!(threads, vec![ThreadId::new(1), ThreadId::new(2)]);
    }

    #[test]
    fn list_mail_still_threads_by_references() {
        let (_, threads) = thread_them(&[
            on_day(0, listed(cue("<a@example.com>", &[], "Topic"))),
            on_day(
                1,
                listed(cue("<b@example.com>", &["<a@example.com>"], "Re: Topic")),
            ),
        ]);

        assert_eq!(threads, vec![ThreadId::new(1); 2]);
    }

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
        assert_eq!(cue.date, message.received_at);
        assert!(!cue.is_list);

        message.list_id = Some("dev.example.com".to_owned());
        assert!(ThreadCue::of(&message).is_list, "a List-Id is list mail");

        message.list_id = None;
        message.promoted = Some(crate::promoted::PromotedHeaders {
            unsubscribe_offered: false,
            automation: crate::promoted::PRECEDENCE_BULK,
        });
        assert!(ThreadCue::of(&message).is_list, "bulk is what is:bulk says");
        assert!(cue.is_reply);
    }

    fn split_at_four() -> Index {
        let mut index = Index::default();
        let (one, two, three) = (
            cue("<m1@example.com>", &[], "Plan"),
            cue("<m2@example.com>", &["<m1@example.com>"], "Re: Plan"),
            cue("<m3@example.com>", &["<m2@example.com>"], "Re: Plan"),
        );
        for cue in [&one, &two, &three] {
            index.file(cue, ThreadId::new(1));
        }
        index
            .by_id
            .insert("<m4@example.com>".to_owned(), ThreadId::new(2));
        index
            .by_id
            .insert("<m5@example.com>".to_owned(), ThreadId::new(2));
        index.splits.push(ThreadId::new(2));
        index
    }

    #[test]
    fn a_reply_to_a_split_message_joins_the_split_thread_and_merges_nothing() {
        let index = split_at_four();
        let reply = cue(
            "<m6@example.com>",
            &[
                "<m1@example.com>",
                "<m2@example.com>",
                "<m3@example.com>",
                "<m4@example.com>",
            ],
            "Re: Plan",
        );

        assert_eq!(assign(&reply, &index), Assignment::Join(ThreadId::new(2)));
    }

    #[test]
    fn a_reply_to_the_original_after_a_split_stays_in_the_original() {
        let index = split_at_four();
        let reply = cue(
            "<m7@example.com>",
            &["<m1@example.com>", "<m2@example.com>", "<m3@example.com>"],
            "Re: Plan",
        );

        assert_eq!(assign(&reply, &index), Assignment::Join(ThreadId::new(1)));
    }

    #[test]
    fn the_nearest_link_decides_when_a_split_thread_is_among_several() {
        let index = split_at_four();
        let reply = cue(
            "<m8@example.com>",
            &["<m4@example.com>", "<m3@example.com>"],
            "Re: Plan",
        );

        assert_eq!(assign(&reply, &index), Assignment::Join(ThreadId::new(1)));
    }

    #[test]
    fn threads_that_are_not_split_still_merge() {
        let mut index = split_at_four();
        index.splits.clear();
        let reply = cue(
            "<m6@example.com>",
            &["<m3@example.com>", "<m4@example.com>"],
            "Re: Plan",
        );

        assert!(matches!(
            assign(&reply, &index),
            Assignment::Merge { into, .. } if into == ThreadId::new(1)
        ));
    }

    fn node(id: i64, own: &str, answers: &[&str]) -> ThreadNode {
        ThreadNode {
            id: MessageId::new(id),
            message_id: Some(id_of(own)),
            references: answers.iter().map(|a| id_of(a)).collect(),
        }
    }

    fn id_of(raw: &str) -> RfcMessageId {
        RfcMessageId::new(raw)
    }

    #[test]
    fn descendants_cover_transitive_replies_and_leave_siblings() {
        let nodes = [
            node(1, "<m1@example.com>", &[]),
            node(2, "<m2@example.com>", &["<m1@example.com>"]),
            node(
                3,
                "<m3@example.com>",
                &["<m1@example.com>", "<m2@example.com>"],
            ),
            node(
                4,
                "<m4@example.com>",
                &["<m1@example.com>", "<m2@example.com>", "<m3@example.com>"],
            ),
            node(5, "<m5@example.com>", &["<M4@example.com>"]),
            node(6, "<m6@example.com>", &["<m5@example.com>"]),
            node(7, "<m7@example.com>", &["<m3@example.com>"]),
        ];

        assert_eq!(
            descendants(&nodes, MessageId::new(4)),
            vec![MessageId::new(4), MessageId::new(5), MessageId::new(6)],
            "the root, its reply, and the reply to that, case folded"
        );
        assert_eq!(
            descendants(&nodes, MessageId::new(7)),
            vec![MessageId::new(7)],
            "a leaf is only itself"
        );
    }
}
