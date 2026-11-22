//! Watches the containing directory, not the file: editors save to a temporary file and rename it
//! over
//! the target, which would leave a watch on the old inode deaf. A debounce coalesces one save's
//! burst of
//! events into one reload. Parsing happens on the watcher's thread, never the UI's.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::error::{ConfigError, Result};
use crate::validate::{self, Checked};

pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchOptions {
    pub debounce: Duration,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            debounce: DEFAULT_DEBOUNCE,
        }
    }
}

/// Dropping it stops the watch and joins the worker, so no callback can arrive after the watcher is
/// gone.
#[derive(Debug)]
pub struct ConfigWatcher {
    path: PathBuf,
    watcher: Option<RecommendedWatcher>,
    worker: Option<JoinHandle<()>>,
}

impl ConfigWatcher {
    /// `on_change` runs on the watcher's thread, once per settled save, with the file already
    /// reparsed and validated.
    pub fn new<F>(path: &Path, on_change: F) -> Result<Self>
    where
        F: FnMut(Checked) + Send + 'static,
    {
        Self::with_options(path, WatchOptions::default(), on_change)
    }

    /// Creates the parent directory: watching it is how a first-run `config.toml` is noticed when
    /// it appears.
    pub fn with_options<F>(path: &Path, options: WatchOptions, mut on_change: F) -> Result<Self>
    where
        F: FnMut(Checked) + Send + 'static,
    {
        let directory = path.parent().filter(|p| !p.as_os_str().is_empty());
        let directory = directory.unwrap_or_else(|| Path::new("."));
        let file_name = path
            .file_name()
            .ok_or_else(|| watch_error(path, "it does not name a file"))?
            .to_os_string();

        std::fs::create_dir_all(directory)
            .map_err(|err| watch_error(directory, &err.to_string()))?;
        let directory = directory
            .canonicalize()
            .map_err(|err| watch_error(directory, &err.to_string()))?;
        let watched = directory.join(&file_name);

        let (tx, rx) = mpsc::channel();
        let mut watcher =
            notify::recommended_watcher(tx).map_err(|err| watch_error(path, &err.to_string()))?;
        watcher
            .watch(&directory, RecursiveMode::NonRecursive)
            .map_err(|err| watch_error(&directory, &err.to_string()))?;

        let debounce = options.debounce;
        let worker = std::thread::Builder::new()
            .name("postio-config-watch".to_string())
            .spawn(move || run(&rx, &watched, debounce, &mut on_change))
            .map_err(|err| watch_error(path, &err.to_string()))?;

        Ok(Self {
            path: path.to_path_buf(),
            watcher: Some(watcher),
            worker: Some(worker),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        self.watcher = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn watch_error(path: &Path, message: &str) -> ConfigError {
    ConfigError::Watch {
        path: path.to_path_buf(),
        message: message.to_string(),
    }
}

const IDLE: Duration = Duration::from_secs(3600);

fn run<F>(
    rx: &mpsc::Receiver<notify::Result<Event>>,
    path: &Path,
    window: Duration,
    on_change: &mut F,
) where
    F: FnMut(Checked),
{
    let mut debounce = Debounce::new(window);
    loop {
        let timeout = debounce.wait(Instant::now()).unwrap_or(IDLE);
        match rx.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                if touches(&event, path) {
                    debounce.touch(Instant::now());
                }
            }
            // A backend hiccup drops one event, not the watch; the next write
            // still arrives, and a reload always re-reads the whole file.
            Ok(Err(_)) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if debounce.take_due(Instant::now()) {
            on_change(validate::check_path(path));
        }
    }
}

/// Reads are ignored: the inotify backend reports `open`, and reloading opens the file, so counting
/// access would make the watcher feed itself.
fn touches(event: &Event, path: &Path) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.paths.iter().any(|candidate| candidate == path)
}

/// A plain state machine over an injected `Instant`, tested without sleeping.
#[derive(Debug)]
struct Debounce {
    window: Duration,
    due: Option<Instant>,
}

impl Debounce {
    fn new(window: Duration) -> Self {
        Self { window, due: None }
    }

    fn touch(&mut self, now: Instant) {
        self.due = Some(now + self.window);
    }

    fn wait(&self, now: Instant) -> Option<Duration> {
        self.due.map(|due| due.saturating_duration_since(now))
    }

    fn take_due(&mut self, now: Instant) -> bool {
        match self.due {
            Some(due) if due <= now => {
                self.due = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notify::event::{
        AccessKind, AccessMode, CreateKind, DataChange, ModifyKind, RemoveKind, RenameMode,
    };

    use super::*;

    fn at(base: Instant, millis: u64) -> Instant {
        base + Duration::from_millis(millis)
    }

    #[test]
    fn a_burst_of_events_is_one_reload() {
        let base = Instant::now();
        let mut debounce = Debounce::new(Duration::from_millis(100));
        for millis in [0, 5, 9, 30] {
            debounce.touch(at(base, millis));
            assert!(!debounce.take_due(at(base, millis)), "not yet at {millis}");
        }
        assert!(!debounce.take_due(at(base, 129)), "still settling");
        assert!(debounce.take_due(at(base, 130)), "one reload, once quiet");
        assert!(!debounce.take_due(at(base, 500)), "and only one");
    }

    #[test]
    fn a_later_save_reloads_again() {
        let base = Instant::now();
        let mut debounce = Debounce::new(Duration::from_millis(100));
        debounce.touch(at(base, 0));
        assert!(debounce.take_due(at(base, 100)));
        debounce.touch(at(base, 900));
        assert!(!debounce.take_due(at(base, 950)));
        assert!(debounce.take_due(at(base, 1000)));
    }

    #[test]
    fn an_idle_debounce_has_nothing_to_wait_for() {
        let base = Instant::now();
        let mut debounce = Debounce::new(Duration::from_millis(100));
        assert_eq!(debounce.wait(base), None);
        debounce.touch(base);
        assert_eq!(debounce.wait(base), Some(Duration::from_millis(100)));
        assert_eq!(debounce.wait(at(base, 60)), Some(Duration::from_millis(40)));
        assert_eq!(
            debounce.wait(at(base, 500)),
            Some(Duration::ZERO),
            "an overdue reload must not wait"
        );
    }

    fn event(kind: EventKind, paths: &[&str]) -> Event {
        Event {
            kind,
            paths: paths.iter().map(PathBuf::from).collect(),
            attrs: Default::default(),
        }
    }

    const CONFIG: &str = "/home/p/.config/postio/config.toml";

    #[test]
    fn writes_creations_renames_and_deletions_all_count() {
        let path = Path::new(CONFIG);
        for kind in [
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            EventKind::Modify(ModifyKind::Metadata(notify::event::MetadataKind::Any)),
            EventKind::Create(CreateKind::File),
            EventKind::Remove(RemoveKind::File),
            EventKind::Any,
        ] {
            assert!(touches(&event(kind, &[CONFIG]), path), "{kind:?}");
        }
    }

    #[test]
    fn an_editors_scratch_file_is_not_our_file() {
        let path = Path::new(CONFIG);
        for other in [
            "/home/p/.config/postio/config.toml.tmp",
            "/home/p/.config/postio/config.toml~",
            "/home/p/.config/postio/.config.toml.swp",
            "/home/p/.config/postio/4913",
            "/home/p/.config/postio/other.toml",
        ] {
            assert!(
                !touches(&event(EventKind::Create(CreateKind::File), &[other]), path),
                "{other}"
            );
        }
    }

    #[test]
    fn a_rename_that_mentions_our_file_counts_whichever_side_it_is_on() {
        let path = Path::new(CONFIG);
        let both = event(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["/home/p/.config/postio/config.toml.tmp", CONFIG],
        );
        assert!(touches(&both, path));
    }

    #[test]
    fn reading_the_file_is_not_a_change() {
        let path = Path::new(CONFIG);
        for kind in [
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            EventKind::Access(AccessKind::Read),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
        ] {
            assert!(!touches(&event(kind, &[CONFIG]), path), "{kind:?}");
        }
    }
}
