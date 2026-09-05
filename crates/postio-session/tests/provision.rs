//! Adding the first account without an onboarding screen (#649).
//!
//! macOS has no way to configure an account: onboarding is GTK, top to
//! bottom, and a Mac has no Linux Postio to have set things up with. #649
//! decided the first slice is the cheapest thing that closes the gap — a
//! headless provisioning path — with native onboarding deferred behind it.
//!
//! The premise that slice was written on has since moved. `[accounts.<id>]`
//! in `config.toml` was retired by #470 because nothing read it; accounts
//! live in the encrypted store, written once by onboarding. So "hand-write
//! eight lines of TOML" is not a path that exists, and what stands in for it
//! has to make the same two writes onboarding makes.
//!
//! **The order of those two writes is the whole of the risk**, and it is
//! recorded in `postio-app`'s own `persist`: the credential first, then the
//! row. 0.1.0 did it the other way and `postio-67` is what that cost — a
//! keyring write that failed after the row was committed left an account with
//! no reachable password, which could not sync, could not authenticate, and
//! could not be repaired from inside the application, because
//! `first_account().is_some()` meant onboarding never ran again. The failure
//! the safe order leaves behind is a credential no row names, which nothing
//! reads and which is rolled back anyway.

use postio_account::discovery::{AccountSettings, Encryption, ServerSettings, SettingsSource};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::account::{AuthMethod, TransportSecurity};
use postio_session::provision::{Provisioned, account_from, provision};
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;

const ADDRESS: &str = "ada@example.com";

/// Settings as a preset row would hand them over: implicit TLS on both, the
/// login the same as the address.
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
    // The sentence #649 is about: a store this helper touched has an account
    // in it, and the password is in the keyring rather than anywhere on disk.
    let database = test_support::temp();
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

    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection)
        .list_enabled()
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
    // `postio-67`, as a test. The row must not be committed until the
    // credential is safely stored, because an account row with no reachable
    // password is a state the application cannot get out of: it will not
    // sync, and its existence is what stops onboarding running again.
    //
    // The example this replaced (`postio-app/examples/provision.rs`) wrote
    // the row first and the credential second, so a locked keyring left
    // exactly that wreck behind.
    let database = test_support::temp();
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

    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection)
        .list_enabled()
        .expect("read the accounts");
    assert!(
        accounts.is_empty(),
        "an account whose password never landed is worse than no account: it \
         cannot sync and it stops onboarding ever running again"
    );
}

#[tokio::test]
async fn provisioning_an_address_that_is_already_there_leaves_it_alone() {
    // Re-running the helper is the ordinary case -- a script, a second
    // attempt after a typo elsewhere -- and it must not produce a second row
    // for the same address, which would leave `first_account` choosing
    // between two. Nor may it overwrite a credential that already works: a
    // re-run with the wrong password in the environment would otherwise
    // break an account that was syncing perfectly well.
    let database = test_support::temp();
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

    let connection = database.connection().expect("checkout");
    assert_eq!(
        AccountRepository::new(&connection)
            .list_enabled()
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
    // The mapping, which is the one place discovery's vocabulary meets the
    // store's. `Encryption` and `TransportSecurity` are the same idea spelled
    // twice, and a mapping that flattened both to TLS would silently dial the
    // implicit-TLS port with STARTTLS expected -- a connection failure the
    // user would read as a wrong password.
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
    // iCloud custom domains, and every provider whose login is not the
    // address mail arrives at. The address is the identity; the login is the
    // credential's name, and sending the wrong one fails authentication with
    // a message about the password.
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
