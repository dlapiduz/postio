//! The engine runs on a stack this crate owns, never the caller's.
//!
//! Everything exported here is called from a thread Swift chose, and Swift
//! chooses badly for us: `Task.detached` runs on the cooperative pool,
//! whose threads have **512 KiB** of stack against the main thread's 8 MiB.
//! The engine's deepest work does not fit that with any margin. Turso
//! compiles every trigger and foreign-key cascade a write can fire into the
//! statement's program at prepare time, recursively (an `UPDATE messages`
//! reaches a `DELETE` on `message_contents`, whose cascade deletes
//! `messages`, whose triggers update `mailboxes`), and that alone is 130 to
//! 230 KiB of stack; a debug build's async frames sit under it. The demo's
//! `outbox` seed needed 502 KiB before Swift's own frames, and the Mac app
//! died on launch with SIGBUS in the guard page.
//!
//! The depth is the schema's, not one statement's, and how much of a
//! foreign thread is left when it calls in is not something this side can
//! know. So the boundary does not depend on it: when less than
//! [`RED_ZONE`] remains, the work moves to a fresh [`ENGINE_STACK`] for the
//! length of the call. On the main thread that never happens; on a
//! cooperative-pool thread it always does, for the price of mapping (lazily)
//! a stack per call. The thread is the same one, so the per-thread runtime
//! in `postio_core::blocking` is reused rather than built again.

/// What a call is guaranteed: as much as a main thread starts with.
const ENGINE_STACK: usize = 8 * 1024 * 1024;

/// Less than this left on the caller's stack, and the call moves. Several
/// times the deepest compile measured, so a call that stays is one that
/// fits.
const RED_ZONE: usize = 2 * 1024 * 1024;

/// Run `work` with at least [`RED_ZONE`] of stack beneath it.
pub(crate) fn on_engine_stack<T>(work: impl FnOnce() -> T) -> T {
    stacker::maybe_grow(RED_ZONE, ENGINE_STACK, work)
}

#[cfg(test)]
mod tests {
    use postio_model::DraftState;

    /// Less than the demo's `outbox` seed needs (about 500 KiB in a debug
    /// build) and a quarter of a Swift cooperative-pool thread's stack.
    const CALLER_STACK: usize = 128 * 1024;

    /// The Mac app's crash on launch: `Session.openDemo("outbox")` from
    /// `Task.detached`, which seeds the store through
    /// [`blocking`](crate::session::blocking) on a cooperative-pool thread.
    #[test]
    fn the_outbox_demo_seeds_from_a_small_foreign_stack() {
        let (database, account) = std::thread::Builder::new()
            .stack_size(CALLER_STACK)
            .spawn(|| crate::session::blocking(postio_demo::seeded(postio_demo::Seed::Outbox)))
            .expect("a thread")
            .join()
            .expect("the seed returns rather than overflowing");

        let drafts = crate::session::blocking(async {
            let connection = database.connect().await.expect("a connection");
            postio_storage::repository::DraftRepository::new(&connection)
                .list_for_account(account)
                .await
                .expect("the drafts")
        });
        assert!(
            drafts.iter().any(|draft| draft.state == DraftState::Queued),
            "the seed queued its send"
        );
    }
}
