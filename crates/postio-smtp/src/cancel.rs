//! A minimal cancellation token.
//!
//! Deliberately small and dependency-free: pulling in `tokio-util` for one
//! type is not worth it. The semantics match `CancellationToken`: cloning
//! shares one cancelled flag, cancelling is idempotent, and awaiting an
//! already-cancelled token returns immediately.
//!
//! Duplicated from `postio-imap::cancel` rather than shared: the two crates
//! are deliberately independent siblings under `postio-sync` (see
//! CLAUDE.md's architecture diagram), and this type is small enough that
//! duplication costs less than a cross-protocol dependency would.

use std::future;
use std::sync::Arc;

use tokio::sync::watch;

/// A shared "stop what you are doing" flag.
#[derive(Clone, Debug)]
pub struct CancelToken {
    tx: Arc<watch::Sender<bool>>,
}

impl CancelToken {
    /// A fresh, uncancelled token.
    pub fn new() -> Self {
        Self {
            tx: Arc::new(watch::channel(false).0),
        }
    }

    /// Cancels every clone of this token. Idempotent.
    ///
    /// `send_replace` rather than `send`: `send` discards the value when no
    /// receiver is currently subscribed, which is exactly the case for a
    /// token cancelled before anything awaits it.
    pub fn cancel(&self) {
        self.tx.send_replace(true);
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        *self.tx.borrow()
    }

    /// Resolves as soon as cancellation is requested — immediately if it
    /// already has been, and never otherwise.
    pub async fn cancelled(&self) {
        let mut rx = self.tx.subscribe();
        loop {
            if *rx.borrow_and_update() {
                return;
            }
            if rx.changed().await.is_err() {
                // Every sender is gone, so cancellation can never arrive.
                future::pending::<()>().await;
            }
        }
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_fresh_token_is_not_cancelled() {
        assert!(!CancelToken::new().is_cancelled());
    }

    #[tokio::test]
    async fn cancelling_is_shared_by_clones_and_idempotent() {
        let token = CancelToken::new();
        let clone = token.clone();

        clone.cancel();
        clone.cancel();

        assert!(token.is_cancelled());
        token.cancelled().await;
    }
}
