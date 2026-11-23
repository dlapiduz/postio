use std::sync::Arc;
use std::time::Duration;

use postio_account::backend::BackendError;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_session::checkup::{explain, remove_account};
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;

const ADDRESS: &str = "test@example.com";

async fn an_account() -> (
    postio_storage::test_support::TempStore,
    MemorySecretStore,
    postio_model::ids::AccountId,
) {
    let database = test_support::temp().await;
    let keyring = MemorySecretStore::new();
    for key in [
        AccountKey::new(ADDRESS.to_owned()),
        AccountKey::new(format!("{ADDRESS}#oauth-refresh")),
    ] {
        keyring
            .store(&key, &Password::new("a-secret"))
            .await
            .expect("the keyring takes it");
    }
    let id = {
        let connection = database.connect().await.expect("a connection");
        let (account, _) = test_support::account_with_inbox(&connection).await;
        account.id
    };
    (database, keyring, id)
}

#[tokio::test]
async fn removing_an_account_takes_its_credentials_with_it() {
    let (database, keyring, id) = an_account().await;

    remove_account(&database, Arc::new(keyring.reopen()), id)
        .await
        .expect("the account is removed");

    let connection = database.connect().await.expect("a connection");
    assert!(
        AccountRepository::new(&connection)
            .get(id)
            .await
            .expect("a read")
            .is_none(),
        "the row is gone"
    );
    assert!(
        keyring.is_empty(),
        "and so is everything the keyring held for it"
    );
}

#[tokio::test]
async fn removing_an_account_that_is_already_gone_is_not_an_error() {
    let (database, keyring, id) = an_account().await;
    let secrets = Arc::new(keyring.reopen());

    remove_account(&database, secrets.clone(), id)
        .await
        .expect("the first removal");
    remove_account(&database, secrets, id)
        .await
        .expect("the second says nothing broke");
}

#[test]
fn a_rejected_password_names_the_thing_that_is_actually_wrong() {
    let said = explain(&BackendError::Auth {
        account: ADDRESS.to_owned(),
        reason: "AUTHENTICATIONFAILED".to_owned(),
    });

    assert!(said.contains("app-specific password"), "{said}");
}

#[test]
fn a_refused_tls_connection_says_postio_will_not_fall_back() {
    let said = explain(&BackendError::Tls {
        host: "imap.example.com".to_owned(),
        reason: "certificate expired".to_owned(),
    });

    assert!(said.contains("will not fall back"), "{said}");
    assert!(said.contains("imap.example.com"));
}

#[test]
fn a_timeout_says_how_long_it_waited() {
    let said = explain(&BackendError::TimedOut {
        context: "connect".to_owned(),
        after: Duration::from_secs(30),
    });

    assert!(said.contains("30s"), "{said}");
}

#[test]
fn a_port_that_answers_but_is_not_imap_says_which_host_that_was() {
    let said = explain(&BackendError::EmptyCapabilities {
        host: "imap.example.com".to_owned(),
    });

    assert!(said.contains("not like an IMAP server"), "{said}");
    assert!(said.contains("imap.example.com"));
}

#[tokio::test]
async fn an_account_with_nothing_in_the_keyring_is_partial_rather_than_wrong() {
    let database = test_support::temp().await;
    let empty = MemorySecretStore::new();
    let account = {
        let connection = database.connect().await.expect("a connection");
        let (account, _) = test_support::account_with_inbox(&connection).await;
        AccountRepository::new(&connection)
            .get(account.id)
            .await
            .expect("a read")
            .expect("the account")
    };

    let report = postio_session::checkup::test_connection(&account, Arc::new(empty.reopen())).await;

    assert!(!report.reachable);
    assert!(
        report.missing_credential,
        "nothing was in the keyring: {}",
        report.message
    );
    assert!(
        !report.rejected_credential,
        "nothing reached a server to refuse it"
    );
}

#[tokio::test]
async fn renaming_an_account_changes_what_it_calls_itself_and_nothing_else() {
    let database = test_support::temp().await;
    let (id, address) = {
        let connection = database.connect().await.expect("a connection");
        let (account, _) = test_support::account_with_inbox(&connection).await;
        (account.id, account.address.address.clone())
    };

    postio_session::checkup::set_display_name(&database, id, "  Ada at work  ")
        .await
        .expect("the rename lands");

    let connection = database.connect().await.expect("a connection");
    let stored = AccountRepository::new(&connection)
        .get(id)
        .await
        .expect("a read")
        .expect("the account");
    assert_eq!(stored.display_name, "Ada at work", "trimmed");
    assert_eq!(
        stored.address.address, address,
        "and the address it signs in with is untouched"
    );
}

#[tokio::test]
async fn renaming_an_account_that_is_gone_says_so() {
    let database = test_support::temp().await;
    let error = postio_session::checkup::set_display_name(
        &database,
        postio_model::ids::AccountId::new(404),
        "Nobody",
    )
    .await
    .expect_err("there is no such account");
    assert!(error.contains("not in the store"), "{error}");
}

#[test]
fn a_refused_password_is_reported_as_one_the_pane_can_replace() {
    let report = postio_session::checkup::failure(&BackendError::Auth {
        account: ADDRESS.to_owned(),
        reason: "AUTHENTICATIONFAILED".to_owned(),
    });

    assert!(!report.reachable);
    assert!(report.rejected_credential, "{}", report.message);
    assert!(!report.missing_credential);
}

#[test]
fn a_server_that_did_not_answer_is_not_a_refused_password() {
    let report = postio_session::checkup::failure(&BackendError::TimedOut {
        context: "connect".to_owned(),
        after: Duration::from_secs(30),
    });

    assert!(!report.reachable);
    assert!(!report.rejected_credential, "{}", report.message);
}
