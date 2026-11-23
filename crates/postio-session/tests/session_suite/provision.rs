use postio_account::discovery::{AccountSettings, Encryption, ServerSettings, SettingsSource};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::account::{AuthMethod, TransportSecurity};
use postio_session::provision::{ProvisionError, Provisioned, account_from, provision, repair};
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;

const ADDRESS: &str = "ada@example.com";

fn settings() -> AccountSettings {
    AccountSettings {
        email: ADDRESS.to_owned(),
        imap: ServerSettings::new("imap.example.com", 993, Encryption::Tls),
        smtp: ServerSettings::new("smtp.example.com", 465, Encryption::StartTls),
        login: ADDRESS.to_owned(),
        source: SettingsSource::Builtin,
        requires_app_password: false,
        note: None,
        password_help_url: None,
        display_name: Some("Example Mail".to_owned()),
        oauth: None,
        jmap: None,
        backends: Vec::new(),
    }
}

#[tokio::test]
async fn a_fresh_store_gains_an_account_and_the_password_goes_to_the_keyring() {
    let database = test_support::temp().await;
    let keyring = MemorySecretStore::new();

    let outcome = provision(
        &database,
        &keyring,
        account_from(&settings()),
        Password::new("hunter2"),
    )
    .await
    .expect("provisioning succeeds");

    let id = match outcome {
        Provisioned::Created(id) => id,
        other => panic!("a fresh store should have created one, got {other:?}"),
    };

    let connection = database.connect().await.expect("checkout");
    let accounts = AccountRepository::new(&connection)
        .list_enabled()
        .await
        .expect("read the accounts");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, id);
    assert_eq!(accounts[0].address.address, ADDRESS);

    let kept = keyring
        .retrieve(&AccountKey::new(ADDRESS))
        .await
        .expect("the credential is in the keyring");
    assert_eq!(kept.expose(), "hunter2");
    assert_eq!(keyring.len(), 1, "one entry, and nothing else stored");
}

#[tokio::test]
async fn a_keyring_that_refuses_leaves_no_account_behind() {
    let database = test_support::temp().await;
    let keyring = MemorySecretStore::locked();

    let error = provision(
        &database,
        &keyring,
        account_from(&settings()),
        Password::new("hunter2"),
    )
    .await
    .expect_err("a locked keyring cannot take the password");
    assert!(
        format!("{error}").to_lowercase().contains("keyring")
            || format!("{error}").to_lowercase().contains("unlock"),
        "the error has to say what to do about it, got: {error}"
    );

    let connection = database.connect().await.expect("checkout");
    let accounts = AccountRepository::new(&connection)
        .list_enabled()
        .await
        .expect("read the accounts");
    assert!(
        accounts.is_empty(),
        "an account whose password never landed is worse than no account: it \
         cannot sync and it stops onboarding ever running again"
    );
}

#[tokio::test]
async fn provisioning_an_address_that_is_already_there_leaves_it_alone() {
    let database = test_support::temp().await;
    let keyring = MemorySecretStore::new();

    let first = provision(
        &database,
        &keyring,
        account_from(&settings()),
        Password::new("the one that works"),
    )
    .await
    .expect("first run");
    let second = provision(
        &database,
        &keyring,
        account_from(&settings()),
        Password::new("a typo"),
    )
    .await
    .expect("second run");

    match (first, second) {
        (Provisioned::Created(a), Provisioned::AlreadyProvisioned(b)) => assert_eq!(a, b),
        other => panic!("the second run should have found the first, got {other:?}"),
    }

    let connection = database.connect().await.expect("checkout");
    assert_eq!(
        AccountRepository::new(&connection)
            .list_enabled()
            .await
            .expect("read the accounts")
            .len(),
        1,
        "one address, one row"
    );
    assert_eq!(
        keyring
            .retrieve(&AccountKey::new(ADDRESS))
            .await
            .expect("the credential")
            .expose(),
        "the one that works",
        "a re-run must not overwrite a credential that is working"
    );
}

#[test]
fn discovered_settings_become_a_row_the_engine_can_dial() {
    let account = account_from(&settings());

    assert_eq!(account.incoming.host, "imap.example.com");
    assert_eq!(account.incoming.port, 993);
    assert_eq!(account.incoming.security, TransportSecurity::Tls);
    assert_eq!(account.outgoing.host, "smtp.example.com");
    assert_eq!(account.outgoing.port, 465);
    assert_eq!(
        account.outgoing.security,
        TransportSecurity::StartTls,
        "carried across, not flattened to the incoming server's answer"
    );
    assert_eq!(account.incoming.username, ADDRESS);
    assert_eq!(account.outgoing.username, ADDRESS);
    assert_eq!(account.auth, AuthMethod::Password);
    assert!(account.enabled);

    let identity = account
        .identities
        .first()
        .expect("an account with no identity can receive mail and never answer it");
    assert!(identity.is_default);
    assert_eq!(identity.address.address, ADDRESS);
}

#[test]
fn a_login_that_differs_from_the_address_is_what_both_servers_are_told() {
    let account = account_from(&AccountSettings {
        login: "ada@icloud.example".to_owned(),
        ..settings()
    });

    assert_eq!(
        account.address.address, ADDRESS,
        "the identity is unchanged"
    );
    assert_eq!(account.incoming.username, "ada@icloud.example");
    assert_eq!(account.outgoing.username, "ada@icloud.example");
}

#[tokio::test]
async fn a_local_account_is_one_write_and_no_credential() {
    let database = test_support::memory().await;
    let keyring = MemorySecretStore::new();

    let provisioned = postio_session::provision::provision_local(&database, ADDRESS, "/srv/mail")
        .await
        .expect("a local account needs nothing but a directory");

    let Provisioned::Created(id) = provisioned else {
        panic!("the first account is created, not already there");
    };
    let connection = database.connect().await.expect("checkout");
    let account = AccountRepository::new(&connection)
        .get(id)
        .await
        .expect("read")
        .expect("the account");
    assert_eq!(
        account.backend,
        postio_model::account::Backend::Maildir {
            root: "/srv/mail".to_owned()
        }
    );
    assert!(
        keyring.is_empty(),
        "nothing was put in the keyring, because there is no credential"
    );
}

#[tokio::test]
async fn adding_the_same_local_store_twice_changes_nothing() {
    let database = test_support::memory().await;

    let first = postio_session::provision::provision_local(&database, ADDRESS, "/srv/mail")
        .await
        .expect("first");
    let again = postio_session::provision::provision_local(&database, ADDRESS, "/srv/other")
        .await
        .expect("second");

    let Provisioned::Created(id) = first else {
        panic!("created");
    };
    assert!(
        matches!(again, Provisioned::AlreadyProvisioned(found) if found == id),
        "a second run reports the account that is there rather than moving it"
    );
}

#[test]
fn a_home_relative_path_is_stored_as_the_directory_it_names() {
    let home = std::env::var("HOME").expect("a home directory");

    assert_eq!(
        postio_session::provision::absolute("~/mail"),
        std::path::PathBuf::from(&home).join("mail")
    );
    assert_eq!(
        postio_session::provision::absolute("/srv/mail"),
        std::path::PathBuf::from("/srv/mail")
    );
    assert_eq!(
        postio_session::provision::absolute("~ada/mail"),
        std::path::PathBuf::from("~ada/mail"),
        "another user's home is not Postio's to guess the layout of"
    );
}

#[tokio::test]
async fn the_helper_is_inert_where_the_repair_route_is_not() {
    let database = test_support::temp().await;
    let keyring = MemorySecretStore::new();

    provision(
        &database,
        &keyring,
        account_from(&settings()),
        Password::new("the one that works"),
    )
    .await
    .expect("first run");

    let again = provision(
        &database,
        &keyring,
        account_from(&settings()),
        Password::new("a drifted environment"),
    )
    .await
    .expect("second run");
    assert!(
        matches!(again, Provisioned::AlreadyProvisioned(_)),
        "the helper claimed to have written something, got {again:?}"
    );
    assert_eq!(
        keyring
            .retrieve(&AccountKey::new(ADDRESS))
            .await
            .expect("the credential")
            .expose(),
        "the one that works",
        "a re-run of the headless helper overwrote a working password, which \
         is the failure its inertness exists to prevent"
    );

    let connection = database.connect().await.expect("checkout");
    let id = AccountRepository::new(&connection)
        .list()
        .await
        .expect("read the accounts")
        .first()
        .expect("the account the first run wrote")
        .id;
    drop(connection);

    repair(&database, &keyring, id, Password::new("the rotated one"))
        .await
        .expect("a person repairing their own account");
    assert_eq!(
        keyring
            .retrieve(&AccountKey::new(ADDRESS))
            .await
            .expect("the credential")
            .expose(),
        "the rotated one",
        "the repair route left the retired password in place, so an account \
         whose provider rotated its app password has no way back"
    );
}

#[tokio::test]
async fn a_repair_names_an_account_that_would_never_read_a_typed_password() {
    let database = test_support::temp().await;
    let keyring = MemorySecretStore::new();

    let mut account = account_from(&settings());
    account.auth = AuthMethod::XOAuth2;
    let id = {
        let connection = database.connect().await.expect("checkout");
        AccountRepository::new(&connection)
            .create(&mut account)
            .await
            .expect("the account row")
    };

    let error = repair(&database, &keyring, id, Password::new("not a token"))
        .await
        .expect_err("a token account refuses a typed password");
    assert!(
        matches!(error, ProvisionError::Unrepairable(_)),
        "the refusal came back as something a caller would retry, got {error:?}"
    );
    assert!(
        format!("{error}").to_lowercase().contains("browser"),
        "the refusal has to name the route that does work, got: {error}"
    );
    assert!(
        keyring.retrieve(&AccountKey::new(ADDRESS)).await.is_err(),
        "a password was written under the address of an account that signs \
         in with a token"
    );
}
