//! A session over one of Focus's demo stores (specs/009-focus-macos T004).
//!
//! The Mac app is photographed and compared with its design over the same
//! seeded stores the GTK app is (`postio-demo`): today's inbox in the
//! references' shape, thirty conversations, a long newsletter. They live in
//! memory, so opening one reads no Keychain and touches nobody's mail.
//!
//! Only a build with the `demo` feature can open one; any other build says
//! so, which is what lets the Swift side call this unconditionally.

use std::sync::Arc;

use crate::session::{Session, SessionError};

#[uniffi::export]
impl Session {
    /// A session over the demo store `seed` names ("small",
    /// "thirty-threads", ...), in memory.
    #[uniffi::constructor]
    pub fn open_demo(seed: String) -> Result<Arc<Session>, SessionError> {
        open(&seed)
    }
}

/// `seed`, or `seed:screen` -- a store with the row screen 04 opens
/// refiled as one of the handoff's HTML bodies (`postio_demo::treatment_demo`:
/// `27` the newsletter on paper, `28` the work mail in app colours), which
/// the message window's screens are photographed over.
#[cfg(feature = "demo")]
fn open(name: &str) -> Result<Arc<Session>, SessionError> {
    let (seed, screen) = match name.split_once(':') {
        Some((seed, screen)) => (seed, Some(screen)),
        None => (name, None),
    };
    let seed = postio_demo::Seed::from_id(seed).ok_or_else(|| SessionError::StoreUnavailable {
        message: format!("There is no demo store called \u{201c}{name}\u{201d}."),
    })?;
    let (database, account) = crate::session::blocking(postio_demo::seeded(seed));
    if let Some(screen) = screen {
        crate::session::blocking(postio_demo::treatment_demo(&database, account, screen));
    }
    Session::open(
        crate::SessionOptions::in_memory_with(database)
            .with_config_for_test(&postio_demo::config()),
    )
}

#[cfg(not(feature = "demo"))]
fn open(_seed: &str) -> Result<Arc<Session>, SessionError> {
    Err(SessionError::StoreUnavailable {
        message: "This build of Postio has no demo stores.".to_owned(),
    })
}

#[cfg(all(test, feature = "demo"))]
mod tests {
    use super::*;

    #[test]
    fn the_small_demo_opens_on_focus_s_inbox() {
        let session = Session::open_demo("small".to_owned()).expect("the demo opens");
        let counts = session.focus_counts().expect("its counts");
        assert!(counts.conversations > 5, "{counts:?}");
        session.shutdown();
    }

    /// The row the inbox's Harbor draft stands in, refiled as one of the
    /// handoff's HTML bodies, as GTK's screens 27-31 have it: `small:27` is
    /// the newsletter that paints its own page, and the message window's
    /// paper screens are photographed over it.
    #[test]
    fn a_screen_after_the_seed_refiles_the_opened_row() {
        let session = Session::open_demo("small:27".to_owned()).expect("the demo opens");
        session.open_focus(crate::FocusScopeFfi::Inbox);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let found = loop {
            let found = (0..12).any(|row| {
                session
                    .focus_row_at(row)
                    .is_some_and(|row| row.subject == "Issue 48: The quiet season")
            });
            if found || std::time::Instant::now() > deadline {
                break found;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert!(found, "the newsletter is in the inbox");
        session.shutdown();
    }

    #[test]
    fn an_unknown_demo_says_so() {
        assert!(Session::open_demo("no-such-seed".to_owned()).is_err());
    }
}

#[cfg(all(test, not(feature = "demo")))]
mod without {
    use super::*;

    #[test]
    fn a_build_without_demos_refuses_one() {
        assert!(Session::open_demo("small".to_owned()).is_err());
    }
}
