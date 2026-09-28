//! What Postio Focus's verbs read of `[focus]`, and where they write the
//! person's corrections to it (spec 007, contracts/config.md).
//!
//! **The person's decisions live in `config.toml`, not the store**: a sender
//! restored from Filtered, a marker kind stopped by repeated dismissal, a
//! digest rule. A write goes through `postio_config::focus_edit`, which
//! touches only the entry it is about, and then
//! `postio_config::save::write_atomically`, the save an editor makes: so the
//! change reaches every running surface through the watcher exactly as
//! `$EDITOR`'s would, and nothing else in the file moves.
//!
//! It is empty until the host that runs Focus mode installs it
//! (`postio_host::Host::enable_focus`). The classic app's and the terminal's
//! verbs never have one, and a Focus verb that needs one says it cannot run
//! there.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use postio_config::FocusConfig;

/// Told `[focus]` as it reads after a write, so the filing pass and Focus's
/// tasks follow the file at once rather than at the watcher's next reload.
pub type OnFocusWritten = Arc<dyn Fn(&FocusConfig) + Send + Sync>;

/// Focus mode's settings, as its verbs share them: cheap to clone, one for
/// the whole store, filled when Focus mode is on.
#[derive(Clone, Default)]
pub struct FocusSettings {
    installed: Arc<RwLock<Option<Installed>>>,
    /// Held across a read-modify-write of the file, so two corrections made
    /// at once -- a command and a rule saved from a dialog -- cannot each
    /// write over the other's.
    writing: Arc<tokio::sync::Mutex<()>>,
}

impl std::fmt::Debug for FocusSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FocusSettings")
            .field("on", &self.is_on())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct Installed {
    path: Option<PathBuf>,
    config: FocusConfig,
    written: OnFocusWritten,
}

impl FocusSettings {
    /// Focus mode is on: its verbs read `config`, and write to the file at
    /// `path`, telling `written` what `[focus]` then says. `None` for the
    /// path is a Focus that does not know where its file is: its verbs that
    /// write say so.
    pub fn install(&self, path: Option<PathBuf>, config: FocusConfig, written: OnFocusWritten) {
        *self.installed.write().expect("never poisoned") = Some(Installed {
            path,
            config,
            written,
        });
    }

    /// Whether Focus mode is on in this process.
    pub fn is_on(&self) -> bool {
        self.installed.read().expect("never poisoned").is_some()
    }

    /// `[focus]` as Focus's verbs read it, when Focus mode is on.
    pub fn config(&self) -> Option<FocusConfig> {
        self.installed
            .read()
            .expect("never poisoned")
            .as_ref()
            .map(|installed| installed.config.clone())
    }

    /// Rewrite `config.toml` through `edit`, which is handed the file's text
    /// and answers its new text, or `None` when the file already says what
    /// was asked. Answers whether anything was written.
    ///
    /// A file that does not read is the person's own edit, mid-way, and is
    /// never written over: the write is refused with a sentence saying so.
    /// Neither the file's text nor what was asked is ever logged.
    pub async fn write(
        &self,
        edit: impl FnOnce(&str) -> Result<Option<String>, String> + Send + 'static,
    ) -> Result<bool, String> {
        let _writing = self.writing.lock().await;
        let (path, written) = {
            let installed = self.installed.read().expect("never poisoned");
            let installed = installed
                .as_ref()
                .ok_or("Postio Focus is not running, so there is nowhere to write that")?;
            let path = installed
                .path
                .clone()
                .ok_or("Postio does not know where config.toml is, so it cannot remember that")?;
            (path, Arc::clone(&installed.written))
        };
        let focus = tokio::task::spawn_blocking(move || write_file(&path, edit))
            .await
            .map_err(|_| "Postio could not write config.toml".to_owned())??;
        let Some(focus) = focus else {
            return Ok(false);
        };
        if let Some(installed) = self.installed.write().expect("never poisoned").as_mut() {
            installed.config = focus.clone();
        }
        written(&focus);
        Ok(true)
    }
}

/// Read the file at `path`, edit it, and save it as an editor does: `[focus]`
/// as it now reads, or `None` when `edit` changed nothing.
fn write_file(
    path: &std::path::Path,
    edit: impl FnOnce(&str) -> Result<Option<String>, String>,
) -> Result<Option<FocusConfig>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err("Postio could not read config.toml".to_owned()),
    };
    if postio_config::Config::from_toml_str(&text).is_err() {
        return Err(
            "config.toml does not read right now, so Postio will not write to it; fix it and \
             try again"
                .to_owned(),
        );
    }
    let Some(edited) = edit(&text)? else {
        return Ok(None);
    };
    let config = postio_config::Config::from_toml_str(&edited)
        .map_err(|_| "Postio could not write that to config.toml".to_owned())?;
    postio_config::save::write_atomically(path, &edited)
        .map_err(|_| "Postio could not save config.toml".to_owned())?;
    Ok(Some(config.focus))
}
