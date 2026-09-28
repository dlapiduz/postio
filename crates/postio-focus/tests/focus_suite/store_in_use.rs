//! One store, either desktop app (US11 scenario 1, T063): Focus opening a
//! store another Postio has open says so, offers Try again, and leaves the
//! store byte for byte as it was.
//!
//! As in `postio-tui`'s `store_in_use.rs`, both Postios are this suite run
//! again over a scratch store (`POSTIO_STORE`): one with [`HOLD`] set, which
//! keeps the store open, and one with [`TRY`] set, which opens Focus's
//! window over it and says what the window shows. Two processes because the
//! engine's lock on the store is a POSIX record lock, which a process never
//! conflicts with itself on; and the store's path is read from the
//! environment, which this process may not set on itself.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};

use crate::support;

/// Set in the child that holds the store open: the store key, in hex.
const HOLD: &str = "POSTIO_FOCUS_TEST_HOLD_STORE_KEY";
/// Set in the child that opens Focus over it: the store key, in hex.
const TRY: &str = "POSTIO_FOCUS_TEST_TRY_STORE_KEY";

const NAME: &str =
    "store_in_use::a_store_another_postio_has_open_is_refused_with_try_again_and_left_alone";

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

/// This case, run again over the store at `store` with `role` set to `hex`.
fn again(store: &Path, role: &str, hex: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("this suite"));
    command
        .args([NAME, "--exact"])
        .env("POSTIO_STORE", store)
        .env(role, hex);
    command
}

/// What Focus's window says over the store at `store`: `answer:` and
/// `retry:` lines from the child.
fn try_to_open(store: &Path, hex: &str) -> (String, String) {
    let ran = again(store, TRY, hex).output().expect("the child runs");
    assert!(ran.status.success(), "{ran:?}");
    let out = String::from_utf8_lossy(&ran.stdout).to_string();
    let line = |prefix: &str| {
        out.lines()
            .find_map(|line| line.strip_prefix(prefix).map(str::to_owned))
            .unwrap_or_else(|| panic!("the child said no {prefix:?}: {ran:?}"))
    };
    (line("answer: "), line("retry: "))
}

/// Every file of the store at `store`, with its bytes.
fn store_bytes(store: &Path) -> Vec<(String, Vec<u8>)> {
    let name = store
        .file_name()
        .expect("a file")
        .to_string_lossy()
        .to_string();
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(store.parent().expect("a directory"))
        .expect("the directory reads")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&name))
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().to_string(),
                std::fs::read(entry.path()).expect("the file reads"),
            )
        })
        .collect();
    files.sort();
    files
}

/// The child that opens Focus: open the store behind a window, the way
/// `app::run` does, and say what the window shows.
fn open_focus(hex: &str) {
    crate::gtk_case(async {
        if !support::display() {
            println!("answer: no display");
            println!("retry: no display");
            return;
        }
        let window = postio_focus::window::FocusWindow::new(None);
        window.present();
        let opened: Rc<std::cell::RefCell<Option<postio_focus::startup::Session>>> = Rc::default();
        let secrets = keyring(hex);
        let open_again: Rc<dyn Fn() -> async_channel::Receiver<postio_focus::startup::Progress>> = {
            let secrets = Arc::clone(&secrets);
            Rc::new(move || postio_focus::startup::open_on_a_thread(None, Arc::clone(&secrets)))
        };
        postio_focus::startup::open(
            &window,
            open_again(),
            Rc::new(postio_config::Config::default()),
            None,
            Rc::clone(&open_again),
            {
                let opened = Rc::clone(&opened);
                Rc::new(move |session| {
                    opened.replace(Some(session));
                })
            },
        );
        assert!(
            crate::settle_until(async || {
                window.unavailable_reason().is_some() || opened.borrow().is_some()
            })
            .await,
            "Focus neither opened the store nor said why not"
        );
        match window.unavailable_reason() {
            Some(reason) => println!("answer: {reason}"),
            None => println!("answer: opened"),
        }
        let said = support::texts(&window);
        println!(
            "retry: {}",
            if said.contains(&"Try again".to_owned()) {
                "Try again"
            } else {
                "none"
            }
        );
        std::io::stdout().flush().expect("said");
        if let Some(session) = opened.take() {
            session.stop();
        }
    });
}

pub fn a_store_another_postio_has_open_is_refused_with_try_again_and_left_alone() {
    if let Ok(hex) = std::env::var(HOLD) {
        let host =
            postio_host::Host::open(None, keyring(&hex), &|_| {}).expect("the holder opens it");
        println!("held");
        std::io::stdout().flush().expect("said");
        // Held until the parent closes our stdin.
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        host.stop();
        return;
    }
    if let Ok(hex) = std::env::var(TRY) {
        open_focus(&hex);
        return;
    }
    if !support::display() {
        return;
    }

    let directory = tempfile::tempdir().expect("a scratch directory");
    let store = directory.path().join("postio.db");
    let hex = postio_storage::key::StoreKey::generate()
        .to_hex()
        .as_str()
        .to_owned();

    let mut holder = again(&store, HOLD, &hex)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the holder runs");
    let mut lines = BufReader::new(holder.stdout.take().expect("its stdout")).lines();
    assert!(
        lines.any(|line| line.is_ok_and(|line| line == "held")),
        "the holder never said it holds the store"
    );

    let before = store_bytes(&store);
    assert!(!before.is_empty(), "the holder made a store");
    let (answer, retry) = try_to_open(&store, &hex);
    assert_eq!(
        answer,
        "Postio is already open in another window. Close it to open Postio here."
    );
    assert_eq!(retry, "Try again");
    assert_eq!(
        store_bytes(&store),
        before,
        "the refused open left the store exactly as it was"
    );

    drop(holder.stdin.take());
    assert!(holder.wait().expect("the holder ends").success());
    assert_eq!(
        try_to_open(&store, &hex).0,
        "opened",
        "once the other has closed it, Focus opens it"
    );
}
