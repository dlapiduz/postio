//! The account the helper writes is the account the macOS app opens (#649).
//!
//! `postio-session`'s own tests prove that `provision` writes a row and a
//! credential. They cannot prove the thing that actually matters, which is
//! that the store it wrote is the store the frontend opens — and this
//! repository has shipped that exact gap four times, most memorably a mail
//! client that could not read mail while every layer's tests passed.
//!
//! So the assertion here is at the boundary, in the number the Swift side
//! reads: `PostioSession.configuredAccounts`. Zero is the empty screen a
//! person sees on a fresh Mac; one is the sentence #649 exists to make true.
//! A helper that wrote a perfectly good account somewhere else would satisfy
//! every test in `postio-session` and change nothing a user could see.
//!
//! Nothing here touches the real keyring: the secret store is injected, and
//! the same instance is reused across both sessions so this is one machine
//! opening its own store twice rather than two unrelated first runs. A test
//! that reached for the login keyring would prompt on a developer's machine
//! and hang on a headless one.

use std::sync::Arc;

use postio_ffi::{Session, SessionOptions};
use postio_account::discovery::{AccountSettings, Encryption, ServerSettings, SettingsSource};
use postio_account::secret::{MemorySecretStore, Password, SecretStore};
use postio_session::provision::{Provisioned, account_from, provision};

const ADDRESS: &str = "ada@example.com";

/// Settings as a preset row hands them over.
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

#[test]
fn a_session_finds_the_account_the_provisioning_helper_wrote() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio.db");
    let keyring = Arc::new(MemorySecretStore::new());
    let secrets: Arc<dyn SecretStore> = keyring.clone();

    // A fresh Mac. This is what the shell shows before anything is
    // provisioned, and asserting it first is what stops the test below
    // passing over a store that was never empty.
    let before = Session::open(SessionOptions::at(&path).with_secrets(secrets.clone()))
        .expect("a session over a fresh store");
    assert_eq!(
        before.configured_accounts(),
        0,
        "a store nobody has provisioned has no accounts in it"
    );
    before.shutdown();
    drop(before);

    // What `postio-provision` does, over the store at the path this
    // application opens. `open_store_at` rather than `open_store` for the one
    // reason a test may differ from the binary: a store per test rather than
    // per machine. The path itself is the same value on both sides.
    let key = postio_session::store_key_blocking(secrets.as_ref())
        .expect("the key the first session minted");
    let (database, _blobs) =
        postio_session::open_store_at(&path, &key).expect("the store the session just made");
    let outcome = tokio::runtime::Runtime::new()
        .expect("a runtime")
        .block_on(provision(
            &database,
            secrets.as_ref(),
            account_from(&settings()),
            Password::new("an app-specific password"),
        ))
        .expect("provisioning succeeds");
    assert!(matches!(outcome, Provisioned::Created(_)));
    drop(database);

    // Relaunching the app, which is step three of the documented path in
    // macos/CLAUDE.md.
    let after = Session::open(SessionOptions::at(&path).with_secrets(secrets))
        .expect("a session over the provisioned store");
    assert_eq!(
        after.configured_accounts(),
        1,
        "the helper wrote an account the frontend cannot see, which is the \
         whole failure mode this test exists for"
    );
    after.shutdown();
}
