use std::sync::Arc;

use postio_account::secret::MemorySecretStore;
use postio_ffi::{Session, SessionOptions};
use postio_model::EmailAddress;
use postio_model::account::{Account, Backend};
use postio_model::ids::AccountId;
use postio_storage::Store;
use postio_storage::repository::AccountRepository;

fn account(name: &str, address: &str) -> Account {
    let mut account = Account::new(name, EmailAddress::new(Some(name), address));
    account.backend = Backend::Imap;
    account.incoming.host = "imap.example.com".to_owned();
    account.incoming.port = 993;
    account.outgoing.host = "smtp.example.com".to_owned();
    account.outgoing.port = 465;
    account
}

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
    // Clearing every marker without setting one would leave no default; assert on the rows, as two
    // defaults and none look alike from the return value.
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
