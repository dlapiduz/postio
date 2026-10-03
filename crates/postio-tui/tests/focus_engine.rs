//! The terminal runs Focus's engine while it holds the store (FR-186, C29).
//!
//! `postio_tui::run::engage_focus` is what `run` calls after the store opens
//! and before the first sync, and `follow_focus_config` is what re-applies
//! `[focus]` when the file changes. Both go through the same
//! `FocusSetup::from_config` `postio-focus` uses, so the two apps cannot
//! disagree about what Focus mode runs with.

use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_host::Host;
use postio_test_support::scaled;

/// A host over a fresh scratch store, as `run` has after `open`.
fn host_over(dir: &std::path::Path) -> Host {
    let key = postio_storage::key::StoreKey::generate();
    let (database, blobs) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime to open the store on")
        .block_on(postio_session::open_store_at(&dir.join("postio.db"), &key))
        .expect("the store opens");
    Host::start(database, blobs, |wiring| wiring).expect("a host")
}

#[test]
fn the_terminal_switches_focus_mode_on_before_it_syncs() {
    let dir = tempfile::tempdir().expect("a scratch store");
    let host = host_over(dir.path());
    assert!(!host.focus_enabled(), "nothing has asked for it yet");

    let handle = postio_tui::run::engage_focus(&host, &postio_config::Config::default(), None);

    assert!(host.focus_enabled(), "the filing pass is in every engine");
    assert!(handle.running(), "the body stage and the due timer run");
    host.stop();
}

#[test]
fn a_changed_focus_section_reaches_the_running_host() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# my own notes\n").expect("a config file");
    let host = Arc::new(host_over(dir.path()));
    let _following =
        postio_tui::run::follow_focus_config(&host, &path).expect("the file is watched");
    assert!(!host.focus_enabled());

    std::fs::write(&path, "[focus]\nfiltering = false\n").expect("an edit");

    let deadline = Instant::now() + scaled(Duration::from_secs(10));
    while !host.focus_enabled() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(host.focus_enabled(), "the edit reached enable_focus");
    host.stop();
}
