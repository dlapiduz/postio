//! A store another Postio has open: the window says to close it, and "Try
//! again" opens the store once it has been closed.
//!
//! One Postio at a time has the store -- the desktop app or the terminal --
//! and the engine's lock on the file is what enforces it (ADR 0041). This is
//! the desktop's half of that: `open_the_store` hearing the refusal and
//! `present` putting the unavailable screen up with the sentence, exactly as
//! `run`'s `activate` does, then the retry from it.
//!
//! The other Postio is this suite's binary run again as this case, with
//! [`HOLD`] set: it opens the store with the same key and keeps it until its
//! stdin closes. A second process because the lock is a POSIX record lock,
//! which a process never conflicts with itself on -- so nothing inside this
//! one could stand in for another app.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. Set before the code under test runs, which is the one
// moment it is sound.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::gdk;
use postio_account::secret::MemorySecretStore;
use postio_app::Installation;
use postio_gtk::unavailable::Unavailable;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

use crate::settle_until;

/// Set in the child that holds the store: the store key, in hex.
const HOLD: &str = "POSTIO_TEST_HOLD_STORE_KEY";

const NAME: &str =
    "store_in_use_window::a_store_another_postio_has_open_says_so_and_try_again_opens_it";

/// The whole sentence, as the person reads it.
const IN_USE: &str = "Postio is already open in another window. Close it to open Postio here.";

/// The child's whole life: open the store `POSTIO_STORE` names with the key
/// in `hex`, say so, and keep it until stdin closes.
fn hold(hex: &str) -> ! {
    let key = postio_storage::key::StoreKey::from_hex(hex).expect("a key in hex");
    let path = std::env::var_os("POSTIO_STORE").expect("the store to hold");
    let held = postio_session::blocking::now(postio_session::open_store_at(path, &key))
        .expect("the child opens the store");
    println!("held");
    std::io::stdout().flush().unwrap();
    let _ = std::io::stdin().read_to_end(&mut Vec::new());
    drop(held);
    // Not back into the harness: it would report a case this process was
    // never asked to judge.
    std::process::exit(0);
}

pub fn a_store_another_postio_has_open_says_so_and_try_again_opens_it() {
    if let Ok(hex) = std::env::var(HOLD) {
        hold(&hex);
    }
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        let store_dir = tempfile::tempdir().expect("a store directory");
        let store = store_dir.path().join("postio.db");
        // SAFETY: first statements of a single-threaded test, before anything
        // under test reads either.
        unsafe {
            std::env::set_var("XDG_STATE_HOME", state_dir.path());
            std::env::set_var("POSTIO_STORE", &store);
        }

        if adw::init().is_err() || gdk::Display::default().is_none() {
            eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
            return;
        }
        let display = gdk::Display::default().unwrap();
        fonts::install().expect("the embedded fonts should install");
        style::install(&display);
        app::install_icons(&display);

        // One keyring for both: the key this window will read is the one the
        // other Postio opened the store with, so a retry that gets past the
        // lock opens the store rather than meeting a wrong key.
        let secrets = Arc::new(MemorySecretStore::default());
        let hex = postio_session::store_key_blocking(secrets.as_ref())
            .expect("a store key")
            .to_hex()
            .as_str()
            .to_owned();

        let mut other = Command::new(std::env::current_exe().expect("this suite"))
            .args([NAME, "--exact"])
            .env(HOLD, &hex)
            .env("POSTIO_STORE", &store)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the other Postio starts");
        let mut lines = BufReader::new(other.stdout.take().unwrap()).lines();
        assert!(
            lines.any(|line| line.is_ok_and(|line| line == "held")),
            "the other Postio never said it has the store open"
        );

        let timeline = postio_gtk::startup::Timeline::start();
        let window = Window::default();
        window.set_timeline(timeline.clone());
        window.present();
        while gtk::glib::MainContext::default().iteration(false) {}

        let context = Rc::new(Installation::new(secrets));
        let opened = Rc::new(std::cell::RefCell::new(None));
        let fed = Rc::new(std::cell::Cell::new(false));
        postio_app::open_the_store(&window, &opened, &context, &fed, &timeline);

        assert!(
            settle_until(async || Unavailable::showing_in(&window).is_some()).await,
            "a store another Postio has open reached no screen"
        );
        let screen = Unavailable::showing_in(&window).expect("just seen");
        assert_eq!(
            screen.reason(),
            IN_USE,
            "the screen has to say to close the other one, in those words"
        );
        assert!(opened.borrow().is_none(), "and nothing was opened");

        // The other Postio closes; "Try again" now opens the store.
        drop(other.stdin.take());
        assert!(other.wait().expect("the other Postio ends").success());
        screen.retry();
        assert!(
            settle_until(async || opened.borrow().is_some()).await,
            "the retry did not open the store once it was free"
        );
        assert!(
            settle_until(async || Unavailable::showing_in(&window).is_none()).await,
            "the store opened and the in-use screen stayed up"
        );

        window.close();
        while gtk::glib::MainContext::default().iteration(false) {}
    });
}
