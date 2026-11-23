use std::sync::Arc;

use postio_account::discovery::{AccountSettings, Encryption, ServerSettings, SettingsSource};
use postio_account::secret::{MemorySecretStore, Password, SecretStore};
use postio_ffi::{Session, SessionOptions};
use postio_session::provision::{Provisioned, account_from, provision};

const ADDRESS: &str = "ada@example.com";

fn settings() -> AccountSettings {
    AccountSettings {
        email: ADDRESS.to_owned(),
        imap: ServerSettings::new("imap.example.com", 993, Encryption::Tls),
        smtp: ServerSettings::new("smtp.example.com", 465, Encryption::Tls),
        login: ADDRESS.to_owned(),
        source: SettingsSource::Builtin,
        requires_app_password: false,
        note: None,
        password_help_url: None,
        display_name: None,
        oauth: None,
        jmap: None,
        backends: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_finds_the_account_the_provisioning_helper_wrote() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio.db");
    let keyring = Arc::new(MemorySecretStore::new());
    let secrets: Arc<dyn SecretStore> = keyring.clone();

    let before = Session::open(SessionOptions::at(&path).with_secrets(secrets.clone()))
        .expect("a session over a fresh store");
    assert_eq!(
        before.configured_accounts().await,
        0,
        "a store nobody has provisioned has no accounts in it"
    );
    before.shutdown();
    drop(before);

    // `open_store_at` rather than `open_store`: a store per test, not per machine.
    let key = postio_session::store_key_blocking(secrets.as_ref())
        .expect("the key the first session minted");
    let (database, _blobs) = postio_session::open_store_at(&path, &key)
        .await
        .expect("the store the session just made");
    let outcome = provision(
        &database,
        secrets.as_ref(),
        account_from(&settings()),
        Password::new("an app-specific password"),
    )
    .await
    .expect("provisioning succeeds");
    assert!(matches!(outcome, Provisioned::Created(_)));
    drop(database);

    let after = Session::open(SessionOptions::at(&path).with_secrets(secrets))
        .expect("a session over the provisioned store");
    assert_eq!(
        after.configured_accounts().await,
        1,
        "the helper wrote an account the frontend cannot see, which is the \
         whole failure mode this test exists for"
    );

    let listed = after.accounts().await;
    assert_eq!(
        listed.len(),
        1,
        "the accounts pane would show an empty list"
    );
    let account = &listed[0];
    assert_eq!(account.address, ADDRESS);
    assert!(
        !account.initials.is_empty(),
        "the row's chip has no letters"
    );
    assert!(
        account.facts.iter().any(|fact| fact.contains("IMAP")),
        "the row says nothing about what kind of account this is: {:?}",
        account.facts
    );
    assert!(
        !account.facts.iter().any(|fact| fact == "disabled"),
        "a freshly provisioned account is not disabled: {:?}",
        account.facts
    );

    after.shutdown();
}
