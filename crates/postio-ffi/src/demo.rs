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

#[cfg(feature = "demo")]
fn open(seed: &str) -> Result<Arc<Session>, SessionError> {
    let seed = postio_demo::Seed::from_id(seed).ok_or_else(|| SessionError::StoreUnavailable {
        message: format!("There is no demo store called \u{201c}{seed}\u{201d}."),
    })?;
    let (database, _account) = crate::session::blocking(postio_demo::seeded(seed));
    Session::open(
        crate::SessionOptions::in_memory_with(database).with_config_for_test(&postio_demo::config()),
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
