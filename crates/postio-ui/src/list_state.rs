//! What a frontend says while the store is opening; nothing is said before `OPENING_THRESHOLD`.

/// Four waits, named separately: the two that can take tens of seconds (a migration, a keyring
/// prompt) are the ones a person most needs told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// A D-Bus round trip against a service that may show its own passphrase prompt: the longest
    /// wait and the least under Postio's control.
    Keyring,
    Store,
    Migrating,
    Indexing,
}

impl Waiting {
    pub const ALL: [Waiting; 4] = [
        Waiting::Keyring,
        Waiting::Store,
        Waiting::Migrating,
        Waiting::Indexing,
    ];
}

/// Twice the 500 ms start budget, so it cannot fire on a healthy launch: a plate that appears and
/// vanishes inside 100 ms is the flicker the budget forbids.
pub const OPENING_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(1);

pub fn describe_wait(waiting: Waiting) -> (&'static str, &'static str) {
    match waiting {
        // Named as the keyring because what to do is elsewhere: a prompt behind another window, or
        // a keyring not running.
        Waiting::Keyring => (
            "Opening your mailbox",
            "Waiting for the keyring to unlock the local store.",
        ),
        Waiting::Store => ("Opening your mailbox", "Reading the local store from disk."),
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
        for different in [Waiting::Migrating, Waiting::Indexing] {
            assert_ne!(
                describe_wait(different).0,
                "Opening your mailbox",
                "{different:?} is not the same promise as opening a mailbox"
            );
        }
    }
}
