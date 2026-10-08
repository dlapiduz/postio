//! What a frontend says while the store is opening, and when a search
//! matched nothing.
//!
//! Toolkit-free, so Focus, the terminal and macOS make the same claims: the
//! store's four waits are named as themselves ([`describe_wait`]), nothing is
//! said before [`OPENING_THRESHOLD`], and an empty result set blames the
//! query rather than the mailbox ([`no_matches_detail`]).

/// What a start that has not finished is actually waiting on.
///
/// Four waits, named separately because "Updating your mailbox's storage" is
/// a different promise from "Opening your mailbox" — and because the two that
/// can legitimately take tens of seconds are the two a person most needs told
/// about. Measured on the live install: a schema migration held a launch for
/// 12.6 s with nothing on screen, and a keyring prompt held another for 28 s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// The store key is being read out of the OS keyring.
    ///
    /// A D-Bus round trip against a service that may be showing a passphrase
    /// prompt of its own, which is why this can be the longest of the four
    /// and the one least under Postio's control.
    Keyring,
    /// The encrypted database is being opened.
    Store,
    /// Schema migrations are being applied.
    Migrating,
    /// The local search index is being built or rebuilt.
    Indexing,
}

impl Waiting {
    /// Every wait, in the order a start meets them.
    pub const ALL: [Waiting; 4] = [
        Waiting::Keyring,
        Waiting::Store,
        Waiting::Migrating,
        Waiting::Indexing,
    ];
}

/// How long a start may take before it is worth saying anything.
///
/// Twice `docs/PRODUCT.md` §18's 500 ms budget: by here the start has already
/// failed its own budget, so there is no risk of speaking over an ordinary
/// one. And far enough past the measured ~150 ms store phase that it cannot
/// fire on a healthy launch at all — which matters more than the exact
/// number, because a plate that appears and is gone inside 100 ms is the
/// flicker §18 forbids rather than the reassurance it was meant to be.
pub const OPENING_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(1);

/// The heading and the line under it for one wait.
///
/// A pure function, so the copy can be asserted on without a display.
pub fn describe_wait(waiting: Waiting) -> (&'static str, &'static str) {
    match waiting {
        // Named as the keyring rather than as Postio, because what to *do*
        // about it is somewhere else entirely: an unlock prompt that is
        // behind another window, or a keyring that is not running.
        Waiting::Keyring => (
            "Opening your mailbox",
            "Waiting for the keyring to unlock the local store.",
        ),
        Waiting::Store => ("Opening your mailbox", "Reading the local store from disk."),
        // A different promise, and deliberately so: this one changes the
        // store rather than reading it, it is once per upgrade, and it is
        // the wait that has actually taken tens of seconds on a real
        // mailbox.
        Waiting::Migrating => (
            "Updating your mailbox\u{2019}s storage",
            "This happens once after an update, and the mail is not touched.",
        ),
        Waiting::Indexing => (
            "Rebuilding the search index",
            "Your mail is all here; searching it will be ready in a moment.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_wait_is_named_as_itself() {
        // #1114: "one line of specific copy naming what is being waited on
        // — the keyring read and the database open are different waits and
        // the line should say which." So it is the *line* that has to be
        // unique. The heading is deliberately shared by the two ordinary
        // waits, because both are the same promise to the reader: your
        // mailbox is opening.
        let details: Vec<&str> = Waiting::ALL.iter().map(|w| describe_wait(*w).1).collect();
        let mut unique = details.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            details.len(),
            "two waits say the same thing, so the line names a wait it is \
             not: {details:?}"
        );

        assert_eq!(describe_wait(Waiting::Keyring).0, "Opening your mailbox");
        assert_eq!(describe_wait(Waiting::Store).0, "Opening your mailbox");
        // And the two that change the store rather than reading it promise
        // something else, because they are something else: they are once per
        // upgrade and they are the waits that have actually taken tens of
        // seconds on a real mailbox.
        for different in [Waiting::Migrating, Waiting::Indexing] {
            assert_ne!(
                describe_wait(different).0,
                "Opening your mailbox",
                "{different:?} is not the same promise as opening a mailbox"
            );
        }
    }
}
