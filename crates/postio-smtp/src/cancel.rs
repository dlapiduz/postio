//! A minimal cancellation token with `CancellationToken`'s semantics, duplicated from
//! `postio-account::cancel`: the two crates are independent siblings and duplication costs less
//! than a cross-protocol dependency.

use std::future;
use std::sync::Arc;

use tokio::sync::watch;

#[derive(Clone, Debug)]
pub struct CancelToken {
    tx: Arc<watch::Sender<bool>>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self {
            tx: Arc::new(watch::channel(false).0),
        }
    }

    /// `send_replace` rather than `send`: `send` discards the value when no receiver is subscribed,
    /// as for a token cancelled before anything awaits it.
    pub fn cancel(&self) {
        self.tx.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.tx.borrow()
    }

    pub async fn cancelled(&self) {
        let mut rx = self.tx.subscribe();
        loop {
            if *rx.borrow_and_update() {
                return;
            }
            if rx.changed().await.is_err() {
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
