//! Opening the consent screen in the user's own browser.
//!
//! ADR 0006 Q3: never Postio's hardened `WebView` — an embedded browser can
//! read the password typed into it, providers block embedded user agents
//! outright, and the reader's `WebView` has JavaScript off, which a consent
//! screen requires. [`SystemBrowserOpener`] shells out to `xdg-open`, the
//! freedesktop way to ask the desktop "open this URL with whatever the user
//! has chosen" — the same mechanism `docs/PRODUCT.md`'s v1 scope (Linux
//! only) already assumes elsewhere.

use std::io;
use std::process::{Command, Stdio};

use url::Url;

/// Opens a URL outside the application.
///
/// A trait so the flow in [`super::authorize`] never touches a real desktop
/// in a test: [`RecordingOpener`] stands in for it there.
pub trait BrowserOpener: Send + Sync {
    /// Opens `url` in the user's browser. Returns once the request to open
    /// it has been made — never waits for the browser, let alone the user.
    fn open(&self, url: &Url) -> io::Result<()>;
}

/// The real opener: `xdg-open <url>`, detached.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBrowserOpener;

impl BrowserOpener for SystemBrowserOpener {
    fn open(&self, url: &Url) -> io::Result<()> {
        // POSTIO-CONSENT: this process is spawned only from `oauth::authorize`,
        // itself reached only from the account's own "Sign in" action — one
        // explicit click per attempt, never on render and never retried
        // automatically. See ADR 0006 Q3 and CLAUDE.md, "Privacy is a
        // feature".
        //
        // Output is discarded rather than inherited: `xdg-open`'s stdout and
        // stderr are chatter about which handler it picked, not something a
        // mail client's own log should carry, and inheriting them would let
        // a misbehaving handler write to Postio's terminal.
        Command::new("xdg-open")
            .arg(url.as_str())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(())
    }
}

#[cfg(test)]
pub use test_support::RecordingOpener;

#[cfg(test)]
mod test_support {
    use std::sync::Mutex;

    use super::*;

    /// A [`BrowserOpener`] that records the URL instead of opening anything,
    /// for tests that play the browser's part themselves.
    #[derive(Debug, Default)]
    pub struct RecordingOpener {
        opened: Mutex<Vec<Url>>,
    }

    impl BrowserOpener for RecordingOpener {
        fn open(&self, url: &Url) -> io::Result<()> {
            self.opened
                .lock()
                .expect("recording opener mutex")
                .push(url.clone());
            Ok(())
        }
    }

    impl RecordingOpener {
        /// The most recently opened URL, if any.
        pub fn last(&self) -> Option<Url> {
            self.opened
                .lock()
                .expect("recording opener mutex")
                .last()
                .cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recording_opener_never_touches_the_desktop_and_remembers_the_url() {
        let opener = RecordingOpener::default();
        let url: Url = "https://example.com/authorize?state=abc".parse().unwrap();

        opener.open(&url).expect("recording never fails");

        assert_eq!(opener.last(), Some(url));
    }
}
