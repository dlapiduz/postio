//! A store another process has open is refused, in a sentence that says so.
//!
//! Only one Postio may have the store open at a time -- the desktop app or
//! the terminal, never both (specs/005-tui-frontend). The engine enforces
//! that with a lock on the file; this is what the second one hears.
//!
//! Its own binary rather than a `storage_suite` module because it needs a
//! second process: the engine's lock is a POSIX record lock, which a process
//! never conflicts with itself on. The child is this same test binary, run
//! again with [`HOLD`] naming the store it is to hold open.

#![allow(clippy::disallowed_methods)] // the crate's own code prepares through `sql::statement`; a test may reach the engine directly

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use postio_storage::{
    Store,
    key::{Purpose, StoreKey},
};

/// Set in the child: the store it holds open until its stdin closes.
const HOLD: &str = "POSTIO_TEST_HOLD_STORE";

fn key() -> postio_storage::key::Subkey {
    StoreKey::from_bytes([0x2a; 32]).derive(Purpose::Database)
}

#[tokio::test]
async fn a_store_open_in_another_process_is_refused_as_in_use() {
    if let Some(path) = std::env::var_os(HOLD) {
        let _held = Store::open(&path, &key())
            .await
            .expect("the child opens it");
        println!("held");
        std::io::stdout().flush().unwrap();
        // Held until the parent closes our stdin.
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("postio.db");
    drop(Store::open(&path, &key()).await.expect("a fresh store"));

    let mut child = Command::new(std::env::current_exe().expect("this test"))
        .args([
            "a_store_open_in_another_process_is_refused_as_in_use",
            "--exact",
            "--nocapture",
        ])
        .env(HOLD, &path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the child runs");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    assert!(
        lines.any(|line| line.is_ok_and(|line| line == "held")),
        "the child never said it holds the store"
    );

    let refused = Store::open(&path, &key()).await;
    let Err(error) = refused else {
        panic!("a store another process has open must not open here too");
    };
    assert!(
        matches!(error, postio_storage::Error::InUse),
        "the engine's own words reached the caller: {error}"
    );
    assert_eq!(
        error.to_string(),
        "Postio is already open in another window. Close it to open Postio here."
    );

    drop(child.stdin.take());
    assert!(child.wait().expect("the child ends").success());
    Store::open(&path, &key())
        .await
        .expect("once the other has closed it, it opens here");
}
