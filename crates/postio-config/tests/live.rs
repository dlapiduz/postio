//! Last-good fallback: an invalid edit must never take the running app down.
//!
//! Written before the implementation, per the TDD rule in `CLAUDE.md`.

use std::path::PathBuf;

use postio_config::live::{LiveConfig, Reload};
use postio_config::{Config, Density};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "postio-live-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
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

const GOOD: &str = "[ui]\ndensity = \"compact\"\n";
const ALSO_GOOD: &str = "[ui]\ndensity = \"comfortable\"\n";
const BROKEN: &str = "[ui\ndensity = \"compact\"\n";
const INVALID: &str = "[ui]\ndensity = \"enormous\"\n";

#[test]
fn a_missing_file_loads_defaults_and_is_valid() {
    let dir = TempDir::new("missing");
    let live = LiveConfig::load(&dir.path("config.toml"));
    assert_eq!(live.config(), &Config::default());
    assert!(live.status().is_valid());
}

#[test]
fn a_good_file_loads() {
    let dir = TempDir::new("good");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let live = LiveConfig::load(&path);
    assert_eq!(live.config().ui.density, Density::Compact);
    assert!(live.status().is_valid());
}

#[test]
fn a_broken_edit_keeps_the_last_good_config() {
    let dir = TempDir::new("broken");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let mut live = LiveConfig::load(&path);

    std::fs::write(&path, BROKEN).unwrap();
    let outcome = live.reload();

    assert_eq!(outcome, Reload::Rejected);
    assert_eq!(
        live.config().ui.density,
        Density::Compact,
        "the last good config must still be in force"
    );
    assert!(!live.status().is_valid());
    assert!(
        live.status().status().starts_with("line"),
        "{}",
        live.status().status()
    );
}

#[test]
fn a_semantically_invalid_edit_also_keeps_the_last_good_config() {
    let dir = TempDir::new("invalid");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let mut live = LiveConfig::load(&path);

    std::fs::write(&path, INVALID).unwrap();
    assert_eq!(live.reload(), Reload::Rejected);
    assert_eq!(live.config().ui.density, Density::Compact);
    assert!(!live.status().is_valid());
}

#[test]
fn fixing_the_file_applies_it_again() {
    let dir = TempDir::new("fixed");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let mut live = LiveConfig::load(&path);

    std::fs::write(&path, BROKEN).unwrap();
    assert_eq!(live.reload(), Reload::Rejected);

    std::fs::write(&path, ALSO_GOOD).unwrap();
    assert_eq!(live.reload(), Reload::Applied);
    assert_eq!(live.config().ui.density, Density::Comfortable);
    assert!(live.status().is_valid());
}

#[test]
fn reloading_an_unchanged_file_reports_no_change() {
    // The ConfigChanged diffing bead needs to know when nothing moved, so a
    // no-op save does not repaint the world.
    let dir = TempDir::new("unchanged");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let mut live = LiveConfig::load(&path);
    assert_eq!(live.reload(), Reload::Unchanged);
    assert!(live.status().is_valid());
}

#[test]
fn a_deleted_file_falls_back_to_defaults_and_stays_valid() {
    let dir = TempDir::new("deleted");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let mut live = LiveConfig::load(&path);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(live.reload(), Reload::Applied);
    assert_eq!(live.config(), &Config::default());
    assert!(live.status().is_valid());
}

#[test]
fn the_generation_counter_only_moves_when_the_config_does() {
    let dir = TempDir::new("generation");
    let path = dir.path("config.toml");
    std::fs::write(&path, GOOD).unwrap();
    let mut live = LiveConfig::load(&path);
    let first = live.generation();

    live.reload();
    assert_eq!(
        live.generation(),
        first,
        "an identical file changes nothing"
    );

    std::fs::write(&path, BROKEN).unwrap();
    live.reload();
    assert_eq!(live.generation(), first, "a rejected file changes nothing");

    std::fs::write(&path, ALSO_GOOD).unwrap();
    live.reload();
    assert!(live.generation() > first);
}
