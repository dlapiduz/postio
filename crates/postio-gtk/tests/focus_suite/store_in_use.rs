//! Both Postios are this suite run again over a scratch store (`POSTIO_STORE`), as two processes:
//! the engine's lock is a POSIX record lock,
//! which a process never conflicts with itself on, and the store's path comes from the environment,
//! which this process may not set.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};

use crate::support;

const HOLD: &str = "POSTIO_FOCUS_TEST_HOLD_STORE_KEY";
const TRY: &str = "POSTIO_FOCUS_TEST_TRY_STORE_KEY";

const NAME: &str =
    "store_in_use::a_store_another_postio_has_open_is_refused_with_try_again_and_left_alone";

fn keyring(hex: &str) -> Arc<dyn SecretStore> {
    let secrets = MemorySecretStore::default();
    postio_session::blocking::now(secrets.store(
        &AccountKey::new(postio_session::STORE_KEY_ENTRY),
        &Password::new(hex),
    ))
    .expect("the memory keyring takes it");
    Arc::new(secrets)
}

fn again(store: &Path, role: &str, hex: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("this suite"));
    command
        .args([NAME, "--exact"])
        .env("POSTIO_STORE", store)
        .env(role, hex);
    command
}

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

fn open_focus(hex: &str) {
    crate::gtk_case(async {
        if !support::display() {
            println!("answer: no display");
            println!("retry: no display");
            return;
        }
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        let opened: Rc<std::cell::RefCell<Option<postio_gtk::startup::Session>>> = Rc::default();
        let opener = postio_gtk::startup::Opener::new(None, keyring(hex));
        postio_gtk::startup::open(
            &window,
            opener.open_on_a_thread(),
            Rc::new(postio_config::Config::default()),
            opener,
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
