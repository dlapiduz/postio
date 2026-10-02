//! The two account settings that are a switch rather than a form: whether an
//! account is synced at all, and which one new mail comes from (#1575).
//!
//! Both existed one layer down —
//! [`AccountRepository::set_enabled`](postio_storage::repository::AccountRepository::set_enabled)
//! and `set_default`, which GTK's settings panel has called since #464 and
//! #960 — and neither had a way across the boundary, so on macOS
//! `ToggleAccountEnabled` and `SetDefaultAccount` were commands with nowhere
//! to go. A frontend cannot reach a repository; this is the whole of what it
//! needs.
//!
//! The assertions read the *row back*, not the return value. A call that
//! reported success while the pane went on drawing the old state is the
//! failure worth catching, and `AccountFfi` is what the pane draws.

use std::sync::Arc;

use postio_account::secret::MemorySecretStore;
use postio_ffi::{Session, SessionOptions};
use postio_model::EmailAddress;
use postio_model::account::{Account, Backend};
use postio_model::ids::AccountId;
use postio_storage::Store;
use postio_storage::repository::AccountRepository;

/// An account row as the store holds one.
fn account(name: &str, address: &str) -> Account {
    let mut account = Account::new(name, EmailAddress::new(Some(name), address));
    account.backend = Backend::Imap;
    account.incoming.host = "imap.example.com".to_owned();
    account.incoming.port = 993;
    account.outgoing.host = "smtp.example.com".to_owned();
    account.outgoing.port = 465;
    account
}

/// A store holding both accounts, and the ids they were given.
async fn store_with_two() -> (Store, AccountId, AccountId) {
    let database = postio_storage::test_support::memory().await;
    let connection = database.connect().await.expect("a checkout");
    let repository = AccountRepository::new(&connection);
    let mut ada = account("Ada Lovelace", "ada@example.com");
    let mut grace = account("Grace Hopper", "grace@example.test");
    let first = repository.create(&mut ada).await.expect("ada is written");
    let second = repository
        .create(&mut grace)
        .await
        .expect("grace is written");
    drop(connection);
    (database, first, second)
}

fn session(database: Store) -> Arc<Session> {
    Session::open(
        SessionOptions::in_memory_with(database).with_secrets(Arc::new(MemorySecretStore::new())),
    )
    .expect("a session over the seeded store")
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_can_be_switched_off_and_the_row_says_so() {
    // A disabled account still has a row — it is configured, it is simply
    // not being synced — so the only way to see the switch took is to read
    // the row's own flag back.
    let (database, ada, _) = store_with_two().await;
    let session = session(database);

    assert!(
        session.accounts().await.iter().all(|row| row.enabled),
        "an account is synced when it is added; nothing has asked otherwise"
    );

    assert_eq!(
        session.set_account_enabled(ada.get(), false).await,
        None,
        "switching an account off is a single-column write and cannot fail here"
    );

    let rows = session.accounts().await;
    let row = rows
        .iter()
        .find(|row| row.id == ada.get())
        .expect("the pane still lists a disabled account");
    assert!(
        !row.enabled,
        "the switch was flipped and the row did not move"
    );
    assert!(
        row.facts.iter().any(|fact| fact == "disabled"),
        "the fact line does not say the account is switched off: {:?}",
        row.facts
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn making_one_account_the_default_takes_it_off_the_other() {
    // The invariant the repository's transaction exists for: "clear every
    // marker" landing without "set this one" would leave the store with no
    // default at all. Asserted from the rows, because two defaults and no
    // default look identical from the return value.
    let (database, ada, grace) = store_with_two().await;
    let session = session(database);

    assert_eq!(session.set_default_account(ada.get()).await, None);
    assert_eq!(session.set_default_account(grace.get()).await, None);

    let rows = session.accounts().await;
    let defaults: Vec<i64> = rows
        .iter()
        .filter(|row| row.is_default)
        .map(|row| row.id)
        .collect();
    assert_eq!(
        defaults,
        vec![grace.get()],
        "exactly one account is the default, and it is the one last asked for"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn naming_an_account_that_is_gone_is_a_sentence_rather_than_a_panic() {
    // The keyboard path can aim at nothing: a row is removed in one window
    // while the command is pressed in another. It has to come back as
    // something to show somebody.
    let (database, _, _) = store_with_two().await;
    let session = session(database);

    assert!(
        session.set_default_account(9_999).await.is_some(),
        "naming a row that does not exist reported success"
    );
    assert!(
        session.set_account_enabled(9_999, false).await.is_some(),
        "naming a row that does not exist reported success"
    );
    session.shutdown();
}
