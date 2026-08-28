//! Acceptance tests for postio-sk9 — credential storage via the Secret Service
//! keyring.
//!
//! None of these touch the network, and none of them need a live Secret
//! Service: the trait is exercised through the in-memory double. The one test
//! that wants a real keyring session is `#[ignore]`.

use postio_imap::secret::{
    AccountKey, CommandSecretStore, MemorySecretStore, Password, SecretError, SecretSource,
    SecretStore,
};

fn key() -> AccountKey {
    AccountKey::new("ada@example.com")
}

// --- storage round trip -------------------------------------------------

#[tokio::test]
async fn password_round_trips_through_the_store() {
    let store = MemorySecretStore::new();
    store
        .store(&key(), &Password::new("app-specific-hunter2"))
        .await
        .unwrap();

    let got = store.retrieve(&key()).await.unwrap();
    assert_eq!(got.expose(), "app-specific-hunter2");
}

#[tokio::test]
async fn stored_password_survives_a_restart() {
    // A "restart" is a brand new store handle over the same backing storage,
    // which is what a keyring gives us across process lifetimes.
    let backing = MemorySecretStore::new();
    backing
        .store(&key(), &Password::new("survives"))
        .await
        .unwrap();

    let reopened = backing.reopen();
    assert_eq!(
        reopened.retrieve(&key()).await.unwrap().expose(),
        "survives"
    );
}

#[tokio::test]
async fn each_account_keeps_its_own_password() {
    let store = MemorySecretStore::new();
    let a = AccountKey::new("a@example.com");
    let b = AccountKey::new("b@example.org");

    store.store(&a, &Password::new("alpha")).await.unwrap();
    store.store(&b, &Password::new("bravo")).await.unwrap();

    assert_eq!(store.retrieve(&a).await.unwrap().expose(), "alpha");
    assert_eq!(store.retrieve(&b).await.unwrap().expose(), "bravo");
}

#[tokio::test]
async fn storing_twice_replaces_the_password() {
    let store = MemorySecretStore::new();
    store.store(&key(), &Password::new("old")).await.unwrap();
    store.store(&key(), &Password::new("new")).await.unwrap();

    assert_eq!(store.retrieve(&key()).await.unwrap().expose(), "new");
}

#[tokio::test]
async fn retrieving_an_unknown_account_is_not_found() {
    let store = MemorySecretStore::new();
    let err = store.retrieve(&key()).await.unwrap_err();
    assert!(
        matches!(err, SecretError::NotFound { .. }),
        "expected NotFound, got {err:?}"
    );
}

#[tokio::test]
async fn delete_removes_the_password() {
    let store = MemorySecretStore::new();
    store.store(&key(), &Password::new("gone")).await.unwrap();
    store.delete(&key()).await.unwrap();

    assert!(matches!(
        store.retrieve(&key()).await.unwrap_err(),
        SecretError::NotFound { .. }
    ));
}

#[tokio::test]
async fn deleting_an_unknown_account_is_not_an_error() {
    let store = MemorySecretStore::new();
    store.delete(&key()).await.unwrap();
}

// --- locked keyring -----------------------------------------------------

#[tokio::test]
async fn locked_keyring_produces_an_actionable_error_and_never_panics() {
    let store = MemorySecretStore::locked();

    for err in [
        store.retrieve(&key()).await.unwrap_err(),
        store.store(&key(), &Password::new("x")).await.unwrap_err(),
        store.delete(&key()).await.unwrap_err(),
    ] {
        assert!(
            matches!(err, SecretError::Locked { .. }),
            "expected Locked, got {err:?}"
        );

        let msg = err.to_string();
        assert!(msg.contains("locked"), "not descriptive: {msg}");
        // Actionable: it must tell the user what to do next.
        assert!(
            msg.contains("Unlock") || msg.contains("unlock"),
            "not actionable: {msg}"
        );
        // And it must never suggest the plaintext escape route.
        assert!(!msg.to_lowercase().contains("plain text config"));
    }
}

// --- secrets never leak -------------------------------------------------

#[test]
fn password_is_redacted_in_debug_and_display() {
    let password = Password::new("super-secret-app-password");

    assert!(!format!("{password:?}").contains("super-secret-app-password"));
    assert!(!format!("{password}").contains("super-secret-app-password"));
    assert!(format!("{password:?}").contains("redacted"));
}

#[test]
fn account_key_debug_carries_no_password() {
    // The key is safe to log; it names the account only.
    let rendered = format!("{:?}", key());
    assert!(rendered.contains("ada@example.com"));
}

#[test]
fn secret_source_defaults_to_the_keyring() {
    assert_eq!(SecretSource::default(), SecretSource::Keyring);
}

#[test]
fn keyring_source_parses_from_config() {
    let parsed: SecretSource = toml::from_str(r#"type = "keyring""#).unwrap();
    assert_eq!(parsed, SecretSource::Keyring);
}

#[test]
fn command_source_parses_from_config() {
    let parsed: SecretSource =
        toml::from_str("type = \"command\"\nargv = [\"pass\", \"show\", \"icloud\"]").unwrap();
    assert_eq!(
        parsed,
        SecretSource::Command {
            argv: vec!["pass".into(), "show".into(), "icloud".into()],
        }
    );
}

#[test]
fn a_plaintext_password_in_config_is_rejected() {
    // This is the himalaya `backend.auth.raw` shape that motivated the bead.
    // Postio must refuse to parse it rather than quietly honour it.
    for plaintext in [
        r#"type = "raw"
raw = "app-specific-password""#,
        r#"type = "keyring"
raw = "app-specific-password""#,
        r#"type = "keyring"
password = "app-specific-password""#,
    ] {
        let parsed = toml::from_str::<SecretSource>(plaintext);
        assert!(
            parsed.is_err(),
            "plaintext config was accepted: {plaintext:?}"
        );
    }
}

#[test]
fn a_serialized_secret_source_carries_no_secret() {
    // Whatever we round-trip into config.toml holds a *reference* to the
    // secret, never the secret itself.
    let rendered = toml::to_string(&SecretSource::Command {
        argv: vec!["pass".into(), "show".into(), "icloud".into()],
    })
    .unwrap();

    assert!(rendered.contains("command"));
    assert!(!rendered.contains("app-specific"));

    let rendered = toml::to_string(&SecretSource::Keyring).unwrap();
    assert!(!rendered.contains("password"));
}

// --- the "run a command" escape hatch -----------------------------------

#[tokio::test]
async fn command_store_returns_the_command_output() {
    let store = CommandSecretStore::new(["printf", "%s", "from-pass"]);
    assert_eq!(store.retrieve(&key()).await.unwrap().expose(), "from-pass");
}

#[tokio::test]
async fn command_store_trims_the_trailing_newline() {
    // `pass show` and friends print a trailing newline that is not part of
    // the password.
    let store = CommandSecretStore::new(["sh", "-c", "echo secret-with-newline"]);
    assert_eq!(
        store.retrieve(&key()).await.unwrap().expose(),
        "secret-with-newline"
    );
}

#[tokio::test]
async fn command_store_reports_a_failing_command() {
    let store = CommandSecretStore::new(["sh", "-c", "echo 'gpg: decryption failed' >&2; exit 2"]);
    let err = store.retrieve(&key()).await.unwrap_err();

    let msg = err.to_string();
    assert!(matches!(err, SecretError::Command { .. }), "got {err:?}");
    assert!(msg.contains("decryption failed"), "no stderr in: {msg}");
}

#[tokio::test]
async fn command_store_reports_a_missing_program() {
    let store = CommandSecretStore::new(["postio-no-such-program-xyzzy"]);
    assert!(matches!(
        store.retrieve(&key()).await.unwrap_err(),
        SecretError::Command { .. }
    ));
}

#[tokio::test]
async fn command_store_rejects_an_empty_command() {
    let store = CommandSecretStore::new(Vec::<String>::new());
    assert!(matches!(
        store.retrieve(&key()).await.unwrap_err(),
        SecretError::Command { .. }
    ));
}

#[tokio::test]
async fn command_store_is_read_only() {
    let store = CommandSecretStore::new(["printf", "%s", "x"]);
    assert!(
        store
            .store(&key(), &Password::new("nope"))
            .await
            .unwrap_err()
            .to_string()
            .contains("read-only")
    );
    assert!(
        store
            .delete(&key())
            .await
            .unwrap_err()
            .to_string()
            .contains("read-only")
    );
}

#[test]
fn a_secret_source_builds_its_store() {
    // The UI/config layer holds a `SecretSource` and never picks a concrete
    // store itself.
    let store = SecretSource::Command {
        argv: vec!["printf".into(), "%s".into(), "x".into()],
    }
    .build();
    assert_eq!(store.describe(), "command");

    // `SecretSource::Keyring` means "wherever this system keeps secrets", so
    // what it builds is per-platform: the Secret Service on freedesktop, the
    // Keychain on macOS. Asserting the *name* rather than skipping keeps both
    // arms real — a build that quietly fell back to the wrong one would fail
    // here rather than at the first D-Bus call on a machine with no D-Bus.
    let default = SecretSource::default().build();
    #[cfg(target_os = "macos")]
    assert_eq!(default.describe(), "keychain");
    #[cfg(not(target_os = "macos"))]
    assert_eq!(default.describe(), "keyring");
}

// --- live keyring (needs a real Secret Service session) -----------------

#[tokio::test]
#[ignore = "needs a live Secret Service session"]
async fn live_keyring_round_trip() {
    use postio_imap::secret::KeyringSecretStore;

    let store = KeyringSecretStore::new();
    let key = AccountKey::new("postio-test@example.org");

    store
        .store(&key, &Password::new("postio-live-test"))
        .await
        .unwrap();
    assert_eq!(
        store.retrieve(&key).await.unwrap().expose(),
        "postio-live-test"
    );
    store.delete(&key).await.unwrap();
    assert!(matches!(
        store.retrieve(&key).await.unwrap_err(),
        SecretError::NotFound { .. }
    ));
}

/// `$XDG_DATA_HOME`, falling back to `$HOME/.local/share` when unset —
/// mirroring `postio-config`'s `config_dir_from`. Under Flatpak this
/// distinction matters: the sandboxed `$HOME` is a private, non-persistent
/// location, and `XDG_DATA_HOME`/`XDG_CONFIG_HOME` are set to the real
/// persistent app directory instead of being bind-mounted under `$HOME` —
/// so joining onto `$HOME` directly finds nothing to scan there.
fn xdg_data_home_from<F>(env: F) -> String
where
    F: Fn(&str) -> Option<String>,
{
    env("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{}/.local/share", env("HOME").expect("HOME")))
}

/// `$XDG_CONFIG_HOME`, falling back to `$HOME/.config` when unset. See
/// [`xdg_data_home_from`].
fn xdg_config_home_from<F>(env: F) -> String
where
    F: Fn(&str) -> Option<String>,
{
    env("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{}/.config", env("HOME").expect("HOME")))
}

fn xdg_data_home() -> String {
    xdg_data_home_from(|key| std::env::var(key).ok())
}

fn xdg_config_home() -> String {
    xdg_config_home_from(|key| std::env::var(key).ok())
}

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let pairs: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

#[test]
fn xdg_data_home_prefers_the_xdg_var_over_home() {
    let dir = xdg_data_home_from(env_of(&[
        ("XDG_DATA_HOME", "/sandbox/data"),
        ("HOME", "/home/p"),
    ]));
    assert_eq!(dir, "/sandbox/data");
}

#[test]
fn xdg_data_home_falls_back_to_home_local_share_when_unset() {
    let dir = xdg_data_home_from(env_of(&[("HOME", "/home/p")]));
    assert_eq!(dir, "/home/p/.local/share");
}

#[test]
fn xdg_data_home_ignores_an_empty_xdg_var() {
    let dir = xdg_data_home_from(env_of(&[("XDG_DATA_HOME", ""), ("HOME", "/home/p")]));
    assert_eq!(dir, "/home/p/.local/share");
}

#[test]
fn xdg_config_home_prefers_the_xdg_var_over_home() {
    let dir = xdg_config_home_from(env_of(&[
        ("XDG_CONFIG_HOME", "/sandbox/config"),
        ("HOME", "/home/p"),
    ]));
    assert_eq!(dir, "/sandbox/config");
}

#[test]
fn xdg_config_home_falls_back_to_home_config_when_unset() {
    let dir = xdg_config_home_from(env_of(&[("HOME", "/home/p")]));
    assert_eq!(dir, "/home/p/.config");
}

#[tokio::test]
#[ignore = "needs a live Secret Service session"]
async fn live_keyring_never_writes_the_password_in_plaintext() {
    use postio_imap::secret::KeyringSecretStore;

    // Distinctive enough that a hit in a keyring file could only have come
    // from this test.
    const CANARY: &str = "postio-plaintext-canary-8f3a1c";

    let store = KeyringSecretStore::new();
    let key = AccountKey::new("postio-canary@example.org");
    store.store(&key, &Password::new(CANARY)).await.unwrap();

    // Give the daemon a moment to flush its encrypted store.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let data_home = xdg_data_home();
    let config_home = xdg_config_home();
    let mut scanned = 0usize;
    for dir in [
        format!("{data_home}/keyrings"),
        format!("{data_home}/postio"),
        format!("{config_home}/postio"),
    ] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            scanned += 1;
            let bytes = std::fs::read(&path).unwrap_or_default();
            assert!(
                !bytes
                    .windows(CANARY.len())
                    .any(|window| window == CANARY.as_bytes()),
                "the password appears in plaintext in {}",
                path.display()
            );
        }
    }

    store.delete(&key).await.unwrap();
    assert!(scanned > 0, "no candidate files were scanned");
}
