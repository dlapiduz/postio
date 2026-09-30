//! The terminal refuses to start over a store another Postio has open, and
//! says to close it.
//!
//! One Postio at a time has the store -- the desktop app or the terminal --
//! and whichever starts second is told so before it takes the terminal over
//! (specs/005-tui-frontend). `postio_tui::run::open` is the whole of that:
//! `run` prints what it answers and exits non-zero, before the alternate
//! screen.
//!
//! Both Postios are this test binary run again, pointed at a scratch store:
//! one with [`HOLD`] set, which keeps the store open, and one with [`TRY`],
//! which tries to open it and says how that went. Two processes because the
//! engine's lock on the store is a POSIX record lock, which a process never
//! conflicts with itself on.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};

/// Set in the child that holds the store open: the store key, in hex.
const HOLD: &str = "POSTIO_TEST_HOLD_STORE_KEY";
/// Set in the child that tries to open it: the store key, in hex.
const TRY: &str = "POSTIO_TEST_TRY_STORE_KEY";

const NAME: &str = "a_store_another_postio_has_open_is_refused_with_the_sentence_to_close_it";

/// A keyring holding `hex` as the store key.
fn keyring(hex: &str) -> Arc<dyn SecretStore> {
    let secrets = MemorySecretStore::default();
    postio_session::blocking::now(secrets.store(
        &AccountKey::new(postio_session::STORE_KEY_ENTRY),
        &Password::new(hex),
    ))
    .expect("the memory keyring takes it");
    Arc::new(secrets)
}

/// This test, run again over the store at `store` with `role` set to `hex`.
fn again(store: &Path, role: &str, hex: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("this test"));
    command
        .args([NAME, "--exact", "--nocapture"])
        .env("POSTIO_STORE", store)
        .env(role, hex);
    command
}

/// What the terminal says, opening the store `store` with `hex`: the line
/// `run` would print, or `opened`.
fn try_to_open(store: &Path, hex: &str) -> String {
    let ran = again(store, TRY, hex).output().expect("the child runs");
    assert!(ran.status.success(), "{ran:?}");
    String::from_utf8_lossy(&ran.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("answer: ").map(str::to_owned))
        .unwrap_or_else(|| panic!("the child said nothing: {ran:?}"))
}

#[test]
fn a_store_another_postio_has_open_is_refused_with_the_sentence_to_close_it() {
    if let Ok(hex) = std::env::var(HOLD) {
        let host = postio_tui::run::open(None, keyring(&hex)).expect("the child opens it");
        println!("held");
        std::io::stdout().flush().unwrap();
        // Held until the parent closes our stdin.
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        host.stop();
        return;
    }
    if let Ok(hex) = std::env::var(TRY) {
        match postio_tui::run::open(None, keyring(&hex)) {
            Ok(host) => {
                println!("answer: opened");
                host.stop();
            }
            Err(sentence) => println!("answer: {sentence}"),
        }
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("postio.db");
    let hex = postio_storage::key::StoreKey::generate()
        .to_hex()
        .as_str()
        .to_owned();

    let mut holder = again(&store, HOLD, &hex)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the child runs");
    let mut lines = BufReader::new(holder.stdout.take().unwrap()).lines();
    assert!(
        lines.any(|line| line.is_ok_and(|line| line == "held")),
        "the child never said it holds the store"
    );

    assert_eq!(
        try_to_open(&store, &hex),
        "postio-tui: Postio is already open in another window. Close it to open Postio here."
    );

    drop(holder.stdin.take());
    assert!(holder.wait().expect("the child ends").success());
    assert_eq!(
        try_to_open(&store, &hex),
        "opened",
        "once the other has closed it, the terminal opens it"
    );
}
