//! ADR 0006 Q3: the consent screen opens in the user's own browser, never Postio's `WebView`: an
//! embedded browser can read the typed password, providers block embedded agents, and the reader's
//! `WebView` has JavaScript off.

use std::io;
use std::process::{Command, Stdio};

use url::Url;

/// A trait so tests never touch a real desktop.
pub trait BrowserOpener: Send + Sync {
    fn open(&self, url: &Url) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBrowserOpener;

impl BrowserOpener for SystemBrowserOpener {
    fn open(&self, url: &Url) -> io::Result<()> {
        // POSTIO-CONSENT: this process is spawned only from `oauth::authorize`,
        // itself reached only from the account's own "Sign in" click: one per attempt, never on
        // render, never retried (ADR 0006 Q3).
        // Output is discarded: it is chatter about the handler, and inheriting it would let a
        // handler write to Postio's terminal.
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
