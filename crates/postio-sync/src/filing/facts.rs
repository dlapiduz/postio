//! The guards' facts for one arrival, as the store answers them (spec 007
//! FR-111, `contracts/engine.md`, "The classifier").
//!
//! [`postio_classify::Facts`] asks and expects an answer on the spot, and
//! the store answers asynchronously, inside the transaction that filed the
//! mail. So the classifier is never handed the store. It is handed
//! [`Known`] -- what has been read so far -- and asked again once the
//! question it could not answer has been read. The pass pays for exactly
//! the questions the classifier asks, in the order it asks them, and no
//! question is read twice.
//!
//! Two things make that cheap:
//!
//! - **Most mail asks nothing.** Every question is a reason *not* to act
//!   (`Facts`' own contract), so if a classifier acts on nothing while every
//!   unread answer is "no", it acts on nothing whatever they say. The pass
//!   asks that first, and a message no rule would touch costs no read.
//! - **An unread answer is "yes" once it matters.** Asked again, a
//!   classifier gets `true` for what is not read yet, which is what `Facts`
//!   says a store that cannot answer must say: when in doubt, mail goes to
//!   the inbox (FR-112). The question it asked is noted, read, and the
//!   classifier asked once more.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::Arc;

use postio_classify::Facts;
use postio_model::{EmailAddress, ThreadId};

/// Which unread questions a classifier asked, in one pass over it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Wanted {
    /// Whether a sender is at one of the person's own domains.
    pub(crate) own_domain: bool,
    /// Whether the person wrote to a sender.
    pub(crate) wrote_to: bool,
    /// Whether the person took part in the conversation.
    pub(crate) took_part: bool,
}

impl Wanted {
    /// Nothing is left to read.
    pub(crate) fn is_empty(self) -> bool {
        self == Wanted::default()
    }
}

/// What is known of one arrival's guards so far.
///
/// Questions about the arrival itself -- its senders, its conversation --
/// can be read, and are noted when asked unread. Questions about anything
/// else cannot be, and are answered as an unread one is, without being
/// noted: nothing here could read them.
pub(crate) struct Known<'a> {
    senders: &'a [EmailAddress],
    thread: Option<ThreadId>,
    /// `[focus.filter] never`, which is config and always known.
    never: &'a (dyn Fn(&EmailAddress) -> bool + Sync),
    /// The domains the person sends from, lowercased, once read: once per
    /// call, and shared by every arrival in it.
    pub(crate) own_domains: Option<Arc<BTreeSet<String>>>,
    /// Which senders the person wrote to, normalised, once read.
    pub(crate) written: Option<BTreeSet<String>>,
    /// Whether the person took part in the conversation, once read.
    pub(crate) took_part: Option<bool>,
    /// What an unread question is answered.
    unknown: bool,
    wanted: Cell<Wanted>,
}

impl<'a> Known<'a> {
    /// Nothing read yet about an arrival from `senders`, in `thread`.
    pub(crate) fn new(
        senders: &'a [EmailAddress],
        thread: Option<ThreadId>,
        never: &'a (dyn Fn(&EmailAddress) -> bool + Sync),
    ) -> Self {
        Known {
            senders,
            thread,
            never,
            own_domains: None,
            written: None,
            took_part: None,
            unknown: false,
            wanted: Cell::default(),
        }
    }

    /// Answer every unread question `answer` from now on, and forget which
    /// ones were asked.
    pub(crate) fn assuming(&mut self, answer: bool) -> &Self {
        self.unknown = answer;
        self.wanted.set(Wanted::default());
        self
    }

    /// The unread questions asked since [`Self::assuming`].
    pub(crate) fn wanted(&self) -> Wanted {
        self.wanted.get()
    }

    fn want(&self, note: impl FnOnce(&mut Wanted)) -> bool {
        let mut wanted = self.wanted.get();
        note(&mut wanted);
        self.wanted.set(wanted);
        self.unknown
    }

    fn is_sender(&self, address: &EmailAddress) -> bool {
        self.senders
            .iter()
            .any(|sender| sender.same_address(address))
    }
}

impl Facts for Known<'_> {
    fn wrote_to(&self, address: &EmailAddress) -> bool {
        if !self.is_sender(address) {
            return self.unknown;
        }
        match &self.written {
            Some(written) => written.contains(&address.normalized()),
            None => self.want(|wanted| wanted.wrote_to = true),
        }
    }

    fn took_part(&self, thread: ThreadId) -> bool {
        if self.thread != Some(thread) {
            return self.unknown;
        }
        match self.took_part {
            Some(took_part) => took_part,
            None => self.want(|wanted| wanted.took_part = true),
        }
    }

    fn own_domain(&self, address: &EmailAddress) -> bool {
        match &self.own_domains {
            Some(domains) => address
                .domain()
                .is_some_and(|domain| domains.contains(&domain.to_ascii_lowercase())),
            None => self.want(|wanted| wanted.own_domain = true),
        }
    }

    fn never_filter(&self, address: &EmailAddress) -> bool {
        (self.never)(address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(address: &str) -> EmailAddress {
        EmailAddress::new(None::<&str>, address)
    }

    const THREAD: ThreadId = ThreadId::new(4);

    #[test]
    fn an_unread_question_is_answered_as_assumed_and_noted() {
        let senders = [address("deals@shop.example")];
        let never = |_: &EmailAddress| false;
        let mut known = Known::new(&senders, Some(THREAD), &never);

        let facts = known.assuming(false);
        assert!(!facts.wrote_to(&senders[0]));
        assert!(!facts.took_part(THREAD));
        assert!(!facts.own_domain(&senders[0]));
        assert_eq!(
            facts.wanted(),
            Wanted {
                own_domain: true,
                wrote_to: true,
                took_part: true
            }
        );

        let facts = known.assuming(true);
        assert!(facts.wanted().is_empty(), "each pass notes its own asks");
        assert!(facts.wrote_to(&senders[0]), "when in doubt, keep the mail");
    }

    #[test]
    fn a_read_answer_is_the_answer_and_is_not_asked_for_again() {
        let senders = [address("Deals@Shop.example")];
        let never = |_: &EmailAddress| false;
        let mut known = Known::new(&senders, Some(THREAD), &never);
        known.own_domains = Some(Arc::new(BTreeSet::from(["firm.example".to_owned()])));
        known.written = Some(BTreeSet::from(["deals@shop.example".to_owned()]));
        known.took_part = Some(false);

        let facts = known.assuming(true);
        assert!(facts.wrote_to(&senders[0]), "matched as addresses are");
        assert!(!facts.took_part(THREAD));
        assert!(!facts.own_domain(&senders[0]));
        assert!(facts.own_domain(&address("tove@FIRM.example")));
        assert!(facts.wanted().is_empty());
    }

    #[test]
    fn what_is_not_about_the_arrival_is_never_read_and_never_filed_on() {
        // A classifier asking about somebody else, or another conversation,
        // is asking what this pass cannot read: it gets the doubtful answer,
        // and nothing is noted to read.
        let senders = [address("deals@shop.example")];
        let never = |_: &EmailAddress| false;
        let mut known = Known::new(&senders, Some(THREAD), &never);

        let facts = known.assuming(true);
        assert!(facts.wrote_to(&address("somebody@else.example")));
        assert!(facts.took_part(ThreadId::new(99)));
        assert!(facts.wanted().is_empty());
    }

    #[test]
    fn never_is_config_and_always_known() {
        let senders = [address("pinned@example.org")];
        let never = |address: &EmailAddress| address.address == "pinned@example.org";
        let mut known = Known::new(&senders, Some(THREAD), &never);

        let facts = known.assuming(false);
        assert!(facts.never_filter(&senders[0]));
        assert!(!facts.never_filter(&address("other@example.org")));
        assert!(facts.wanted().is_empty());
    }
}
