//! `Refresh` (`F5`, `R`): bring the folder on screen in line with the server. Unlike the
//! local-first verbs in `actions.rs` it is a network pass that is not undoable, so the handler
//! starts it and returns rather than holding the bus.
//! The engine arrives after the bus (it starts when the window is fed, and may not at all);
//! `EngineSlot` is that gap, read at invocation time.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use postio_core::bridge::EventSink;
use postio_core::dispatch::{CommandError, DispatcherBuilder, Invocation};
use postio_core::state::{RefreshTarget, SharedState};
use postio_core::{CommandId, Event};
use postio_model::{AccountId, MailboxId, MailboxRole};
use postio_runtime::Engine;
use postio_storage::Store;
use postio_storage::repository::MailboxRepository;

/// The engines, once there are any, keyed by account since an engine syncs only its own account's
/// mailboxes.
#[derive(Debug, Clone, Default)]
pub struct EngineSlot(Arc<Mutex<BTreeMap<AccountId, Engine>>>);

impl EngineSlot {
    pub fn fill(&self, account: AccountId, engine: Engine) {
        self.0
            .lock()
            .expect("never poisoned")
            .insert(account, engine);
    }

    pub fn get(&self, account: AccountId) -> Option<Engine> {
        self.0
            .lock()
            .expect("never poisoned")
            .get(&account)
            .cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().expect("never poisoned").is_empty()
    }
}

pub fn wire(
    builder: DispatcherBuilder,
    engine: EngineSlot,
    database: Store,
    state: SharedState,
) -> DispatcherBuilder {
    builder.on(CommandId::Refresh, move |invocation: Invocation| {
        let engine = engine.clone();
        let database = database.clone();
        let state = state.clone();
        async move { refresh(&engine, &database, &state, &invocation.events()).await }
    })
}

/// Start a sync pass over every mailbox the view draws from (the folder, or each account's inbox
/// for Focus, unified and Flagged). Returns once the engines have been asked.
async fn refresh(
    engines: &EngineSlot,
    database: &Store,
    state: &SharedState,
    events: &EventSink,
) -> Result<(), CommandError> {
    let Some(view) = state.read(|state| state.viewing().cloned()) else {
        return Err(CommandError::rejected("Nothing is open to refresh"));
    };
    if engines.is_empty() {
        // No account or no transport: a key that silently did nothing would be worse than a
        // sentence.
        return Err(CommandError::rejected("This account is not syncing"));
    }

    let passes = passes_for(database, view.refresh_target())
        .await
        .map_err(|_| CommandError::rejected("Postio could not read its folders"))?;
    let mut started = 0usize;
    for (account, mailbox) in passes {
        let Some(engine) = engines.get(account) else {
            continue;
        };
        started += 1;
        let events = events.clone();
        tokio::spawn(async move {
            // The pass reports itself as it goes; this only says when it could not run at all.
            if let Err(error) = engine.sync(mailbox).await {
                events.emit(Event::Error {
                    message: error.message().to_string(),
                    account: Some(account),
                });
            }
        });
    }
    if started == 0 {
        return Err(CommandError::rejected("This account is not syncing"));
    }
    Ok(())
}

async fn passes_for(
    database: &Store,
    target: RefreshTarget,
) -> Result<Vec<(AccountId, MailboxId)>, postio_storage::Error> {
    let connection = database.read().await?;
    let mailboxes = MailboxRepository::new(&connection);
    match target {
        RefreshTarget::Folder(id) => Ok(mailboxes
            .get(id)
            .await?
            .map(|mailbox| (mailbox.account_id, id))
            .into_iter()
            .collect()),
        RefreshTarget::Inboxes(accounts) => {
            let mut passes = Vec::with_capacity(accounts.len());
            for account in accounts {
                if let Some(inbox) = mailboxes.by_role(account, MailboxRole::Inbox).await? {
                    passes.push((account, inbox.id));
                }
            }
            Ok(passes)
        }
    }
}

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use postio_account::backend::{MockBackend, MockMailbox};
    use postio_core::bridge::{EventStream, event_channel};
    use postio_core::state::AppState;
    use postio_model::MailboxRole;
    use postio_runtime::engine::{EngineParts, NetworkSource, SystemClock};
    use postio_storage::{BlobStore, Store, test_support};

    use super::*;

    fn looking_at(mailbox: postio_model::MailboxId) -> SharedState {
        let mut state = AppState::new();
        state.open_mailbox(mailbox);
        SharedState::new(state)
    }

    fn a_sink() -> (EventSink, EventStream) {
        event_channel()
    }

    async fn engine_in(
        slot: &EngineSlot,
        database: &Store,
        account: AccountId,
        backend: Arc<MockBackend>,
    ) {
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(directory.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");
        std::mem::forget(directory);
        let (sink, _engine_events) = event_channel();
        slot.fill(
            account,
            postio_runtime::Engine::spawn(EngineParts {
                account,
                database: database.clone(),
                blobs,
                backend,
                smtp: Arc::new(
                    postio_smtp::transport::RustlsConnector::new().expect("a connector"),
                ),
                tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
                    postio_account::secret::MemorySecretStore::default(),
                ))),
                events: sink,
                retry: Default::default(),
                backfill: Default::default(),
                reconnect: Default::default(),
                watch: Default::default(),
                network: NetworkSource::Ignored,
                mailbox_roles: Default::default(),
                clock: Arc::new(SystemClock),
            })
            .expect("the engine starts"),
        );
    }

    fn mock_with(path: &str) -> Arc<MockBackend> {
        Arc::new(
            MockBackend::builder()
                .mailbox(MockMailbox::new(path))
                .build(),
        )
    }

    async fn is_asked(backend: &MockBackend, before: u64) -> bool {
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                if backend.calls() > before {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .is_ok()
    }

    #[tokio::test]
    async fn refreshing_with_nothing_open_says_so() {
        let (sink, _events) = a_sink();
        let database = test_support::memory().await;

        let refused = refresh(
            &EngineSlot::default(),
            &database,
            &SharedState::default(),
            &sink,
        )
        .await
        .expect_err("there is nothing to refresh");

        assert!(
            refused.to_string().contains("Nothing is open"),
            "the sentence has to name what is missing: {refused}"
        );
    }

    #[tokio::test]
    async fn refreshing_without_an_engine_says_so_rather_than_doing_nothing() {
        let (sink, _events) = a_sink();
        let database = test_support::memory().await;

        let refused = refresh(
            &EngineSlot::default(),
            &database,
            &looking_at(postio_model::MailboxId::new(1)),
            &sink,
        )
        .await
        .expect_err("there is no engine to ask");

        assert!(
            refused.to_string().contains("not syncing"),
            "the sentence has to say why: {refused}"
        );
    }

    #[tokio::test]
    async fn refreshing_runs_a_sync_pass_over_the_folder_in_view() {
        let database = test_support::memory().await;
        let report = postio_storage::seed::seed_small(&database, 3).await;
        let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
        let backend = mock_with(&inbox.path);
        let engine = EngineSlot::default();
        engine_in(&engine, &database, report.account.id, backend.clone()).await;

        let before = backend.calls();
        let (sink, _events) = a_sink();
        refresh(&engine, &database, &looking_at(inbox.id), &sink)
            .await
            .expect("the pass starts");

        assert!(
            is_asked(&backend, before).await,
            "F5 never reached the server: the pass was started but nothing was asked"
        );
    }

    #[tokio::test]
    async fn refreshing_the_focus_inbox_syncs_every_accounts_inbox() {
        let database = test_support::memory().await;
        let first = postio_storage::seed::seed_small(&database, 3).await;
        let second =
            postio_storage::seed::seed_extra_account(&database, "Grace", "grace@example.com", 4)
                .await;
        let first_inbox = first.mailbox(MailboxRole::Inbox).expect("an inbox");
        let second_inbox = second.mailbox(MailboxRole::Inbox).expect("an inbox");
        let first_backend = mock_with(&first_inbox.path);
        let second_backend = mock_with(&second_inbox.path);
        let engines = EngineSlot::default();
        engine_in(&engines, &database, first.account.id, first_backend.clone()).await;
        engine_in(
            &engines,
            &database,
            second.account.id,
            second_backend.clone(),
        )
        .await;

        let mut state = AppState::new();
        state.open_view(postio_core::state::ViewScope::Focus {
            accounts: vec![first.account.id, second.account.id],
        });
        let state = SharedState::new(state);

        let (before_first, before_second) = (first_backend.calls(), second_backend.calls());
        let (sink, _events) = a_sink();
        refresh(&engines, &database, &state, &sink)
            .await
            .expect("the Focus inbox is a view of mail");

        assert!(
            is_asked(&first_backend, before_first).await,
            "the first account's inbox was not refreshed"
        );
        assert!(
            is_asked(&second_backend, before_second).await,
            "the second account's inbox was not refreshed"
        );
    }

    #[tokio::test]
    async fn refreshing_a_folder_asks_the_engine_of_its_account() {
        let database = test_support::memory().await;
        let first = postio_storage::seed::seed_small(&database, 3).await;
        let second =
            postio_storage::seed::seed_extra_account(&database, "Grace", "grace@example.com", 4)
                .await;
        let first_inbox = first.mailbox(MailboxRole::Inbox).expect("an inbox");
        let second_inbox = second.mailbox(MailboxRole::Inbox).expect("an inbox");
        let first_backend = mock_with(&first_inbox.path);
        let second_backend = mock_with(&second_inbox.path);
        let engines = EngineSlot::default();
        engine_in(&engines, &database, first.account.id, first_backend.clone()).await;
        engine_in(
            &engines,
            &database,
            second.account.id,
            second_backend.clone(),
        )
        .await;

        let before = second_backend.calls();
        let (sink, _events) = a_sink();
        refresh(&engines, &database, &looking_at(second_inbox.id), &sink)
            .await
            .expect("the pass starts");

        assert!(
            is_asked(&second_backend, before).await,
            "the folder's own account was not asked"
        );
    }
}
