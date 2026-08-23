//! Live-reload tests: what the watcher does when the file changes underneath.
//!
//! Written before the implementation, per the TDD rule in `CLAUDE.md`.
//!
//! These are the only tests in this crate that touch real file events, so they
//! are written to wait for an outcome rather than to sleep for a fixed time:
//! every wait is a `recv_timeout` that either gets its event or fails the test.
//! The debounce logic itself is pure and unit-tested with an injected clock.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use postio_config::live::{LiveConfig, Reload};
use postio_config::watch::{ConfigWatcher, WatchOptions};
use postio_config::{Checked, Density};

/// Short enough to keep the suite quick, long enough to coalesce one save.
const DEBOUNCE: Duration = Duration::from_millis(60);
/// Generous: a loaded CI box can take a while to deliver an inotify event.
const EXPECT: Duration = Duration::from_secs(5);
/// How long "and nothing else happened" is worth waiting for.
const QUIET: Duration = Duration::from_millis(400);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("postio-watch-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn watch(path: &std::path::Path) -> (ConfigWatcher, Receiver<Checked>) {
    let (tx, rx) = mpsc::channel();
    let watcher =
        ConfigWatcher::with_options(path, WatchOptions { debounce: DEBOUNCE }, move |checked| {
            let _ = tx.send(checked);
        })
        .expect("the watcher must start");
    (watcher, rx)
}

fn expect_one(rx: &Receiver<Checked>) -> Checked {
    let checked = rx.recv_timeout(EXPECT).expect("expected a reload");
    assert!(
        matches!(rx.recv_timeout(QUIET), Err(RecvTimeoutError::Timeout)),
        "one save must produce exactly one reload"
    );
    checked
}

const GOOD: &str = "[ui]\ndensity = \"compact\"\n";
const ALSO_GOOD: &str = "[ui]\ndensity = \"comfortable\"\n";
const BROKEN: &str = "[ui\ndensity = \"compact\"\n";

#[test]
fn a_save_triggers_exactly_one_reload() {
    let dir = TempDir::new("save");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let (_watcher, rx) = watch(&path);

    std::fs::write(&path, ALSO_GOOD).unwrap();

    let checked = expect_one(&rx);
    assert_eq!(
        checked.config.expect("a config").ui.density,
        Density::Comfortable
    );
}

#[test]
fn a_burst_of_writes_is_coalesced_into_one_reload() {
    // What an editor actually does: truncate, write, flush, chmod, ...
    let dir = TempDir::new("burst");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let (_watcher, rx) = watch(&path);

    for _ in 0..5 {
        std::fs::write(&path, ALSO_GOOD).unwrap();
    }

    let checked = expect_one(&rx);
    assert_eq!(
        checked.config.expect("a config").ui.density,
        Density::Comfortable
    );
}

#[test]
fn an_atomic_rename_over_the_file_is_detected() {
    // The write-then-rename dance that `$EDITOR` and every careful writer does.
    // A watcher registered on the file's inode would never see this.
    let dir = TempDir::new("rename");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let (_watcher, rx) = watch(&path);

    let temp = dir.path("config.toml.tmp");
    std::fs::write(&temp, ALSO_GOOD).unwrap();
    std::fs::rename(&temp, &path).unwrap();

    let checked = expect_one(&rx);
    assert_eq!(
        checked.config.expect("a config").ui.density,
        Density::Comfortable
    );
}

#[test]
fn a_second_rename_is_still_seen() {
    // Naive watchers survive exactly one replacement and then go deaf.
    let dir = TempDir::new("rename-twice");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let (_watcher, rx) = watch(&path);

    for (index, text) in [ALSO_GOOD, GOOD].into_iter().enumerate() {
        let temp = dir.path(&format!("config.toml.{index}.tmp"));
        std::fs::write(&temp, text).unwrap();
        std::fs::rename(&temp, &path).unwrap();
        let checked = expect_one(&rx);
        assert!(checked.config.is_some(), "reload {index}");
    }
}

#[test]
fn a_file_created_after_the_watcher_started_is_picked_up() {
    // First run: the directory exists but the file does not.
    let dir = TempDir::new("create");
    let path = dir.path("config.toml");
    let (_watcher, rx) = watch(&path);

    std::fs::write(&path, GOOD).unwrap();

    let checked = expect_one(&rx);
    assert_eq!(
        checked.config.expect("a config").ui.density,
        Density::Compact
    );
}

#[test]
fn deleting_the_file_falls_back_to_defaults() {
    let dir = TempDir::new("delete");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let (_watcher, rx) = watch(&path);

    std::fs::remove_file(&path).unwrap();

    let checked = expect_one(&rx);
    assert!(checked.validation.is_valid());
    assert_eq!(
        checked.config.expect("defaults"),
        postio_config::Config::default()
    );
}

#[test]
fn an_invalid_edit_surfaces_the_error_without_disrupting_the_app() {
    let dir = TempDir::new("invalid");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let mut live = LiveConfig::load(&path);
    let (_watcher, rx) = watch(&path);

    std::fs::write(&path, BROKEN).unwrap();

    let checked = expect_one(&rx);
    assert_eq!(live.apply(checked), Reload::Rejected);
    assert_eq!(
        live.config().ui.density,
        Density::Compact,
        "the running app keeps the last good config"
    );
    assert!(!live.status().is_valid());
    assert!(live.status().status().starts_with("line"));
}

#[test]
fn a_fixed_file_is_applied_again() {
    let dir = TempDir::new("fixed");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let mut live = LiveConfig::load(&path);
    let (_watcher, rx) = watch(&path);

    std::fs::write(&path, BROKEN).unwrap();
    assert_eq!(live.apply(expect_one(&rx)), Reload::Rejected);

    std::fs::write(&path, ALSO_GOOD).unwrap();
    assert_eq!(live.apply(expect_one(&rx)), Reload::Applied);
    assert_eq!(live.config().ui.density, Density::Comfortable);
}

#[test]
fn an_editors_scratch_file_is_ignored() {
    // vim writes `4913`, `.config.toml.swp` and `config.toml~` in the same
    // directory. None of them is our file.
    let dir = TempDir::new("scratch");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let (_watcher, rx) = watch(&path);

    std::fs::write(dir.path("4913"), "").unwrap();
    std::fs::write(dir.path(".config.toml.swp"), "swap").unwrap();
    std::fs::write(dir.path("config.toml~"), GOOD).unwrap();

    assert!(
        matches!(rx.recv_timeout(QUIET), Err(RecvTimeoutError::Timeout)),
        "a sibling file must not trigger a reload"
    );
}

#[test]
fn dropping_the_watcher_stops_the_reloads() {
    let dir = TempDir::new("drop");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let (watcher, rx) = watch(&path);
    drop(watcher);

    std::fs::write(&path, ALSO_GOOD).unwrap();

    // Dropping the watcher drops the callback too, so the channel is closed as
    // well as quiet; either way, no reload may arrive.
    assert!(
        rx.recv_timeout(QUIET).is_err(),
        "a dropped watcher must not keep reloading"
    );
}

#[test]
fn the_watcher_reports_the_path_it_watches() {
    let dir = TempDir::new("path");
    let path = dir.path("config.toml");
    let (watcher, _rx) = watch(&path);
    assert_eq!(watcher.path(), path);
}
