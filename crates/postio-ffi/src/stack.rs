//! The engine runs on a stack this crate owns: Swift's cooperative-pool threads have 512 KiB, and
//! Turso's trigger/cascade compile needs 130-230 KiB or more.
//! When less than `RED_ZONE` remains, the call moves to a fresh `ENGINE_STACK`; the main thread
//! never does.

const ENGINE_STACK: usize = 8 * 1024 * 1024;

/// Less than this left and the call moves; several times the deepest compile measured.
const RED_ZONE: usize = 2 * 1024 * 1024;

pub(crate) fn on_engine_stack<T>(work: impl FnOnce() -> T) -> T {
    stacker::maybe_grow(RED_ZONE, ENGINE_STACK, work)
}

#[cfg(test)]
mod tests {
    use postio_model::DraftState;

    const CALLER_STACK: usize = 128 * 1024;

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
