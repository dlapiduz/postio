//! `Refresh` (`F5`, `R`): bring the folder on screen in line with the server.
//!
//! # Why this is not in `actions.rs`
//!
//! Every verb there is local-first — a handful of indexed writes and their
//! queue rows, done before the handler returns, so app state and the undo
//! stack see a total order. Refresh is the opposite kind of thing: it is a
//! *network* pass with no local write of its own, it can take as long as the
//! server takes, and it is not undoable. Awaiting it on the bus would hold
//! every other command behind a mailbox that is slow to answer.
//!
//! So the handler starts the pass and returns. The engine already reports what
//! it is doing — `SyncProgress` while it counts, `ConnectionChanged` when it
//! settles, `MessageListChanged` when the folder actually moved — which is the
//! same way the status line hears about a sync nobody asked for.
//!
//! # Why the engine arrives late
//!
//! The bus is built before the window exists, because the window's very first
//! gesture must reach a real handler. The engine is started when the window is
//! fed, which is later and may not happen at all — no account, or no usable
//! TLS. [`EngineSlot`] is that gap: the handler reads it at invocation time and
//! says so plainly when there is nothing behind it, rather than the key doing
//! nothing.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use postio_model::ids::AccountId;

use postio_core::bridge::EventSink;
use postio_core::dispatch::{CommandError, DispatcherBuilder, Invocation};
use postio_core::state::SharedState;
use postio_core::{CommandId, Event};
use postio_runtime::Engine;

/// The engines, one per account, as they are started.
///
/// #183 made this a table: the composition root starts one engine per
/// *enabled* account, and a handler that needs "the" engine has to say whose.
/// Written at startup by whoever starts them; read on every invocation. The
/// write lock is held for a `BTreeMap` insert and nothing else, so a reader
/// never waits on a writer that is opening a connection — that work happens
/// before `fill` is called, exactly as it did under the old `OnceLock`.
#[derive(Debug, Clone, Default)]
pub struct EngineSlot(Arc<RwLock<BTreeMap<AccountId, Engine>>>);

impl EngineSlot {
    /// Put an account's engine in. The first engine for an account keeps it:
    /// an engine is never replaced while the process runs.
    pub fn fill(&self, account: AccountId, engine: Engine) {
        self.0
            .write()
            .expect("no engine writer panics")
            .entry(account)
            .or_insert(engine);
    }

    /// The engine syncing `account`, if one was started.
    pub fn for_account(&self, account: AccountId) -> Option<Engine> {
        self.0
            .read()
            .expect("no engine writer panics")
            .get(&account)
            .cloned()
    }

    /// The engine, when exactly one account is syncing.
    ///
    /// The bridge for consumers that predate multi-account — the reader's
    /// body fetch, drag-out — which know a message but have not yet been
    /// taught to name its account. With several engines running this is
    /// `None`, deliberately: guessing an engine fetches one account's mail
    /// over another account's session, which is worse than declining.
    /// #184/#185 teach those call sites accounts, and then this goes.
    pub fn single(&self) -> Option<Engine> {
        let engines = self.0.read().expect("no engine writer panics");
        if engines.len() == 1 {
            engines.values().next().cloned()
        } else {
            None
        }
    }

    /// How many engines are running.
    pub fn count(&self) -> usize {
        self.0.read().expect("no engine writer panics").len()
    }
}

/// Register `Refresh` on `builder`.
///
/// Separate from `actions::dispatcher` so the two kinds of verb keep their own
/// reasoning: see the module docs.
pub fn wire(
    builder: DispatcherBuilder,
    engine: EngineSlot,
    state: SharedState,
) -> DispatcherBuilder {
    builder.on(CommandId::Refresh, move |invocation: Invocation| {
        let engine = engine.clone();
        let state = state.clone();
        async move { refresh(&engine, &state, &invocation.events()) }
    })
}

/// Start a sync pass over the mailbox in view.
///
/// Returns as soon as the engine has been asked, not when it has answered.
fn refresh(
    engine: &EngineSlot,
    state: &SharedState,
    events: &EventSink,
) -> Result<(), CommandError> {
    let (Some(mailbox), scope) = state.read(|state| (state.mailbox(), *state.scope())) else {
        return Err(CommandError::rejected("No folder is open to refresh"));
    };
    // A mailbox is only ever open within one account (#182), so the scope
    // names whose engine this pass belongs to. Unified has no mailbox open
    // and is caught by the rejection above.
    let Some(account) = scope.account() else {
        return Err(CommandError::rejected("No folder is open to refresh"));
    };
    let Some(engine) = engine.for_account(account) else {
        // No account, or no transport: the sidebar already says the account is
        // offline, and a key that silently did nothing would be worse than a
        // sentence saying why.
        return Err(CommandError::rejected("This account is not syncing"));
    };
    let events = events.clone();
    tokio::spawn(async move {
        // The pass reports itself as it goes — progress, connection state, and
        // the list changing — so all this has left to do is say when it could
        // not run at all. A failure the engine already announced through the
        // status line would otherwise be announced twice.
        if let Err(error) = engine.sync(mailbox).await {
            events.emit(Event::Error {
                message: error.message().to_string(),
            });
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    //! What `Refresh` does before it hands off, and that it hands off at all.
    //!
    //! Nothing here draws anything, so unlike `compose`'s test these need no
    //! display — they are ordinary functions over a real bus state and a mock
    //! server, which is the point of the handler being a plain function rather
    //! than a closure buried in the builder.

    use std::sync::Arc;

    use postio_core::bridge::{EventStream, event_channel};
    use postio_core::state::AppState;
    use postio_imap::backend::{MockBackend, MockMailbox};
    use postio_model::MailboxRole;
    use postio_runtime::engine::{EngineParts, NetworkSource};
    use postio_storage::{BlobStore, test_support};

    use super::*;

    /// A state with a folder open, so a refresh has something to refresh.
    /// The account every test engine here syncs.
    fn the_account() -> AccountId {
        AccountId::new(1)
    }

    fn looking_at(mailbox: postio_model::MailboxId) -> SharedState {
        let mut state = AppState::new();
        // The scope first, as the composition root sets it (#182): a mailbox
        // is only ever open within one account, and refresh resolves whose
        // engine to ask from exactly this.
        state.open_account(the_account());
        state.open_mailbox(mailbox);
        SharedState::new(state)
    }

    /// Somewhere for the handler to report, and the stream that would carry it
    /// to the window.
    fn a_sink() -> (EventSink, EventStream) {
        event_channel()
    }

    #[test]
    fn refreshing_with_no_folder_open_says_so() {
        let (sink, _events) = a_sink();

        let refused = refresh(&EngineSlot::default(), &SharedState::default(), &sink)
            .expect_err("there is nothing to refresh");

        assert!(
            refused.to_string().contains("No folder"),
            "the sentence has to name what is missing: {refused}"
        );
    }

    #[test]
    fn refreshing_without_an_engine_says_so_rather_than_doing_nothing() {
        // No account, or no usable transport. A key that silently did nothing
        // is the failure mode this whole branch exists to avoid.
        let (sink, _events) = a_sink();

        let refused = refresh(
            &EngineSlot::default(),
            &looking_at(postio_model::MailboxId::new(1)),
            &sink,
        )
        .expect_err("there is no engine to ask");

        assert!(
            refused.to_string().contains("not syncing"),
            "the sentence has to say why: {refused}"
        );
    }

    #[tokio::test]
    async fn refreshing_runs_a_sync_pass_over_the_folder_in_view() {
        let database = test_support::memory();
        let report = postio_storage::seed::seed_small(&database, 3);
        let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(directory.path()).expect("a blob store");
        let (sink, _engine_events) = event_channel();

        let backend = Arc::new(
            MockBackend::builder()
                .mailbox(MockMailbox::new(&inbox.path))
                .build(),
        );
        let engine = EngineSlot::default();
        engine.fill(
            report.account.id,
            postio_runtime::Engine::spawn(EngineParts {
                account: report.account.id,
                database: database.clone(),
                blobs,
                backend: backend.clone(),
                smtp: Arc::new(
                    postio_smtp::transport::RustlsConnector::new().expect("a connector"),
                ),
                secrets: Arc::new(postio_imap::secret::MemorySecretStore::default()),
                events: sink,
                retry: Default::default(),
                backfill: Default::default(),
                reconnect: Default::default(),
                watch: Default::default(),
                network: NetworkSource::Ignored,
                mailbox_roles: Default::default(),
            })
            .expect("the engine starts"),
        );

        let before = backend.calls();
        let (sink, _events) = a_sink();
        refresh(&engine, &looking_at(inbox.id), &sink).expect("the pass starts");

        // The handler returns before the pass finishes — that is the whole
        // point of it — so what is asserted is that the server was actually
        // asked, not what it said.
        let asked = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                if backend.calls() > before {
                    return true;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;

        assert!(
            asked.is_ok(),
            "F5 never reached the server: the pass was started but nothing was asked"
        );
    }
}
