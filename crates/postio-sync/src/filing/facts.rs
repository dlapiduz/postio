//! The classifier's guard facts for one arrival, read lazily: `Known` holds what has been read, and
//! an unread answer is "yes" (mail goes to the inbox, FR-112).
//! The pass first asks with every unread fact "no", so mail no rule would touch costs no read.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::Arc;

use postio_classify::Facts;
use postio_model::{EmailAddress, ThreadId};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Wanted {
    pub(crate) own_domain: bool,
    pub(crate) wrote_to: bool,
    pub(crate) took_part: bool,
}

impl Wanted {
    pub(crate) fn is_empty(self) -> bool {
        self == Wanted::default()
    }
}

/// Questions about the arrival itself can be read and are noted when asked unread; questions about
/// anything else are answered as unread, un-noted.
pub(crate) struct Known<'a> {
    senders: &'a [EmailAddress],
    thread: Option<ThreadId>,
    never: &'a (dyn Fn(&EmailAddress) -> bool + Sync),
    pub(crate) own_domains: Option<Arc<BTreeSet<String>>>,
    pub(crate) written: Option<BTreeSet<String>>,
    pub(crate) took_part: Option<bool>,
    unknown: bool,
    wanted: Cell<Wanted>,
}

impl<'a> Known<'a> {
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

    pub(crate) fn assuming(&mut self, answer: bool) -> &Self {
        self.unknown = answer;
        self.wanted.set(Wanted::default());
        self
    }

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
