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
//! (`postio_host::Host::enable_focus`). The classic app's verbs never have
//! one, and a Focus verb that needs one says it cannot run
//! there.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use postio_config::{ContactRule, DigestRule, FocusConfig, NeverEntry, never_entry};
use postio_model::{ContactArrival, ContactSettings, EmailAddress, StoppedMarker};
use postio_storage::Store;
use postio_storage::repository::{ContactRepository, ContactSettingsRepository};

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
    /// What correspondents decided (ADR 0048), added to `[focus]` wherever
    /// it is read. Loaded from the store by [`FocusSettings::refresh`].
    contacts: Arc<RwLock<Vec<ContactRule>>>,
    /// Each sender's remembered treatment, `(address, "app" | "paper")`,
    /// read with the rules in the same turn: a reader that looks senders up
    /// in memory starts from it.
    treatments: Arc<RwLock<Vec<(String, String)>>>,
    /// Told `[focus]` whenever it changes, besides the one `written` that
    /// Focus mode installs: a frontend that keeps its own copy.
    watchers: Arc<Mutex<Vec<OnFocusWritten>>>,
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

    /// `[focus]` as Focus's verbs read it, when Focus mode is on: the file,
    /// and what correspondents decided beside it.
    pub fn config(&self) -> Option<FocusConfig> {
        self.installed
            .read()
            .expect("never poisoned")
            .as_ref()
            .map(|installed| self.overlaid(&installed.config))
    }

    /// `raw` -- `[focus]` as the file reads -- with what correspondents
    /// decided added: what the filing pass and Focus's tasks run with.
    pub fn overlaid(&self, raw: &FocusConfig) -> FocusConfig {
        raw.with_contacts(&self.contacts.read().expect("never poisoned"))
    }

    /// Read what correspondents decided from `database` and tell the filing
    /// pass and Focus's tasks, as a write to the file would.
    pub async fn refresh(&self, database: &Store) -> Result<(), String> {
        self.load_contacts(database).await?;
        self.publish();
        Ok(())
    }

    /// Read what correspondents decided from `database`, telling nobody:
    /// what Focus mode does before it installs.
    pub async fn load_contacts(&self, database: &Store) -> Result<(), String> {
        let failed = |_| "Postio could not read what you decided about senders".to_owned();
        // A warm reader, not a connection of its own: this is read as a window
        // opens, and opening a connection there is what its budget counts.
        let reader = database.read().await.map_err(failed)?;
        let treatments = ContactSettingsRepository::new(&reader)
            .treatments()
            .await
            .map_err(failed)?;
        *self.treatments.write().expect("never poisoned") = treatments;
        let rules = ContactSettingsRepository::new(&reader)
            .derived()
            .await
            .map_err(failed)?
            .into_iter()
            .map(|one| ContactRule {
                contact: one.contact.get(),
                name: one.name,
                addresses: one.addresses,
                settings: one.settings,
            })
            .collect();
        *self.contacts.write().expect("never poisoned") = rules;
        Ok(())
    }

    /// Each sender's remembered treatment, `(address, "app" | "paper")`, as
    /// last read ([`FocusSettings::load_contacts`]).
    pub fn treatments(&self) -> Vec<(String, String)> {
        self.treatments.read().expect("never poisoned").clone()
    }

    /// Be told `[focus]` -- the file with what people decided -- whenever a
    /// verb changes it: the filing pass hears through the host, and a
    /// frontend with a copy of its own through this.
    pub fn watch(&self, told: OnFocusWritten) {
        self.watchers.lock().expect("never poisoned").push(told);
    }

    fn tell_watchers(&self, config: &FocusConfig) {
        let watchers = self.watchers.lock().expect("never poisoned").clone();
        for told in watchers {
            told(config);
        }
    }

    /// Tell `written` and the watchers what `[focus]` reads as now.
    fn publish(&self) {
        let told = {
            let installed = self.installed.read().expect("never poisoned");
            installed.as_ref().map(|installed| {
                (
                    self.overlaid(&installed.config),
                    Arc::clone(&installed.written),
                )
            })
        };
        if let Some((config, written)) = told {
            written(&config);
            self.tell_watchers(&config);
        }
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
        self.write_locked(edit).await
    }

    /// [`FocusSettings::write`], for a caller that holds the lock.
    async fn write_locked(
        &self,
        edit: impl FnOnce(&str) -> Result<Option<String>, String> + Send + 'static,
    ) -> Result<bool, String> {
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
        let overlaid = self.overlaid(&focus);
        written(&overlaid);
        self.tell_watchers(&overlaid);
        Ok(true)
    }
}

impl FocusSettings {
    /// Pin `entry` -- an address, or a whole domain as `@example.org` --
    /// so it is never filtered, or take the pin back. Answers whether
    /// anything changed.
    ///
    /// An address is the correspondent's, so a second address of theirs is
    /// covered too (ADR 0048). A domain is not a correspondent and stays in
    /// the file, and so does an address whose correspondent already has a
    /// digest: one arrival cannot be both, and the file can say both. Taking
    /// a pin back clears it wherever it was kept.
    pub async fn pin(&self, database: &Store, entry: &str, pinned: bool) -> Result<bool, String> {
        let _writing = self.writing.lock().await;
        let entry = entry.trim().to_owned();
        let Some(NeverEntry::Address(address)) = never_entry(&entry) else {
            let text = entry.clone();
            return self
                .write_locked(move |file| {
                    postio_config::focus_edit::set_never(file, &text, pinned)
                        .map_err(|_| "Postio could not write that sender to config.toml".to_owned())
                })
                .await;
        };
        let address = address.to_lowercase();
        let mut changed = false;
        if pinned {
            let (person, mut settings) = self.person(database, &address).await?;
            match settings.arrival {
                Some(ContactArrival::Inbox) => {}
                Some(ContactArrival::Digest) => {
                    let text = address.clone();
                    return self
                        .write_locked(move |file| {
                            postio_config::focus_edit::set_never(file, &text, true).map_err(|_| {
                                "Postio could not write that sender to config.toml".to_owned()
                            })
                        })
                        .await;
                }
                _ => {
                    settings.arrival = Some(ContactArrival::Inbox);
                    self.put(database, person, &settings).await?;
                    changed = true;
                }
            }
        } else {
            if let Some((person, mut settings)) = self.existing(database, &address).await?
                && settings.arrival == Some(ContactArrival::Inbox)
            {
                settings.arrival = None;
                self.put(database, person, &settings).await?;
                changed = true;
            }
            let text = address.clone();
            changed |= self
                .write_locked(move |file| {
                    postio_config::focus_edit::set_never(file, &text, false)
                        .map_err(|_| "Postio could not write that sender to config.toml".to_owned())
                })
                .await
                .unwrap_or(false);
        }
        if changed {
            self.refresh(database).await?;
        }
        Ok(changed)
    }

    /// Turn `kind` markers off for `sender` -- an address, or a domain --
    /// or back on. Answers whether anything changed. Kept as [`pin`] keeps
    /// pins.
    ///
    /// [`pin`]: FocusSettings::pin
    pub async fn stop_marker(
        &self,
        database: &Store,
        sender: &str,
        kind: &str,
        stopped: bool,
    ) -> Result<bool, String> {
        let _writing = self.writing.lock().await;
        let sender = sender.trim().to_owned();
        let kind_name = kind.trim().to_owned();
        let in_file = |sender: String, kind: String, present: bool| {
            move |file: &str| {
                postio_config::focus_edit::set_stop_marker(file, &sender, &kind, present)
                    .map_err(|_| "Postio could not write that correction to config.toml".to_owned())
            }
        };
        let (Some(NeverEntry::Address(address)), Some(marker)) =
            (never_entry(&sender), StoppedMarker::from_name(&kind_name))
        else {
            return self.write_locked(in_file(sender, kind_name, stopped)).await;
        };
        let address = address.to_lowercase();
        let mut changed = false;
        if stopped {
            let (person, mut settings) = self.person(database, &address).await?;
            if !settings.stops(marker) {
                settings.set_stop(marker, true);
                self.put(database, person, &settings).await?;
                changed = true;
            }
        } else {
            if let Some((person, mut settings)) = self.existing(database, &address).await?
                && settings.stops(marker)
            {
                settings.set_stop(marker, false);
                self.put(database, person, &settings).await?;
                changed = true;
            }
            changed |= self
                .write_locked(in_file(address, kind_name, false))
                .await
                .unwrap_or(false);
        }
        if changed {
            self.refresh(database).await?;
        }
        Ok(changed)
    }

    /// Write `rule`, replacing the rule called `replacing` when there is
    /// one, at `position` among the file's rules when it goes in the file.
    ///
    /// A rule that is one `from:` query for one address is that
    /// correspondent's digest and is kept on them, under the rule's own name;
    /// anything else (a domain, a list, several senders) is the file's. A
    /// rule replaced from either home leaves it.
    pub async fn put_digest(
        &self,
        database: &Store,
        replacing: Option<&str>,
        rule: &DigestRule,
        position: Option<usize>,
    ) -> Result<(), String> {
        let _writing = self.writing.lock().await;
        let config = self.config().ok_or("Digests need Postio Focus")?;
        let replaced = replacing.map(str::trim).filter(|name| !name.is_empty());
        let derived = replaced
            .and_then(|name| config.contact_digest(name))
            .cloned();
        let in_file = replaced.filter(|name| {
            self.installed
                .read()
                .expect("never poisoned")
                .as_ref()
                .is_some_and(|installed| {
                    installed
                        .config
                        .digests
                        .iter()
                        .any(|r| r.name.trim() == *name)
                })
        });

        if let Some(address) = postio_config::digest_address(rule) {
            let (person, mut settings) = self.person(database, &address).await?;
            let free = match (&settings.arrival, &settings.digest) {
                (Some(ContactArrival::Inbox), _) => false,
                (_, Some(other)) => Some(other.name.as_str()) == replaced,
                _ => true,
            };
            if free {
                settings.arrival = Some(ContactArrival::Digest);
                settings.digest = Some(postio_config::setting_of(rule));
                self.put(database, person, &settings).await?;
                if let Some(old) = derived.filter(|old| old.contact != person.get()) {
                    self.clear_digest(database, old.contact).await?;
                }
                if let Some(name) = in_file {
                    let name = name.to_owned();
                    self.write_locked(move |file| {
                        Ok(postio_config::focus_edit::remove_digest_rule(file, &name)
                            .map_err(|_| {
                                "Postio could not write that rule to config.toml".to_owned()
                            })?
                            .map(|(text, _, _)| text))
                    })
                    .await?;
                }
                return self.refresh(database).await;
            }
        }

        if let Some(old) = derived {
            self.clear_digest(database, old.contact).await?;
        }
        let rule = rule.clone();
        let name = in_file.map(str::to_owned);
        self.write_locked(move |file| {
            postio_config::focus_edit::put_digest_rule(file, name.as_deref(), &rule, position)
                .map(Some)
                .map_err(|_| "Postio could not write that rule to config.toml".to_owned())
        })
        .await?;
        self.refresh(database).await
    }

    /// Remove the rule called `name` from wherever it is kept, answering
    /// where it stood and what it was, so that undo can put it back.
    pub async fn remove_digest(
        &self,
        database: &Store,
        name: &str,
    ) -> Result<Option<(usize, DigestRule)>, String> {
        let _writing = self.writing.lock().await;
        let config = self.config().ok_or("Digests need Postio Focus")?;
        if let Some(owner) = config.contact_digest(name) {
            let at = config
                .digests
                .iter()
                .position(|r| r.name.trim() == name.trim());
            let kept = at.map(|at| (at, config.digests[at].clone()));
            self.clear_digest(database, owner.contact).await?;
            self.refresh(database).await?;
            return Ok(kept);
        }
        let removed: Arc<Mutex<Option<(usize, DigestRule)>>> = Arc::default();
        let slot = Arc::clone(&removed);
        let name = name.to_owned();
        self.write_locked(move |file| {
            let Some((text, at, rule)) = postio_config::focus_edit::remove_digest_rule(file, &name)
                .map_err(|_| "Postio could not write that rule to config.toml".to_owned())?
            else {
                return Ok(None);
            };
            *slot.lock().expect("never poisoned") = Some((at, rule));
            Ok(Some(text))
        })
        .await?;
        let removed = removed.lock().expect("never poisoned").take();
        Ok(removed)
    }

    /// The correspondent at `address`, made if need be, with what they
    /// decided.
    async fn person(
        &self,
        database: &Store,
        address: &str,
    ) -> Result<(postio_model::ContactId, ContactSettings), String> {
        let failed = |_| "Postio could not remember that about the sender".to_owned();
        let connection = database.connect().await.map_err(failed)?;
        let person = ContactRepository::new(&connection)
            .person_for_email(&EmailAddress::new(None::<String>, address))
            .await
            .map_err(failed)?;
        let settings = ContactSettingsRepository::new(&connection)
            .get(person)
            .await
            .map_err(failed)?;
        Ok((person, settings))
    }

    /// The correspondent at `address` and what they decided, when they
    /// exist: nobody is made to take something back.
    async fn existing(
        &self,
        database: &Store,
        address: &str,
    ) -> Result<Option<(postio_model::ContactId, ContactSettings)>, String> {
        let failed = |_| "Postio could not read what you decided about the sender".to_owned();
        let connection = database.connect().await.map_err(failed)?;
        let Some(person) = ContactRepository::new(&connection)
            .by_address(address)
            .await
            .map_err(failed)?
        else {
            return Ok(None);
        };
        let settings = ContactSettingsRepository::new(&connection)
            .get(person.id)
            .await
            .map_err(failed)?;
        Ok(Some((person.id, settings)))
    }

    async fn put(
        &self,
        database: &Store,
        person: postio_model::ContactId,
        settings: &ContactSettings,
    ) -> Result<(), String> {
        let failed = |_| "Postio could not remember that about the sender".to_owned();
        let connection = database.connect().await.map_err(failed)?;
        ContactSettingsRepository::new(&connection)
            .put(person, settings)
            .await
            .map_err(failed)
    }

    /// Take the digest off the correspondent with id `contact`.
    async fn clear_digest(&self, database: &Store, contact: i64) -> Result<(), String> {
        let person = postio_model::ContactId::new(contact);
        let failed = |_| "Postio could not remember that about the sender".to_owned();
        let connection = database.connect().await.map_err(failed)?;
        let repository = ContactSettingsRepository::new(&connection);
        let mut settings = repository.get(person).await.map_err(failed)?;
        settings.digest = None;
        if settings.arrival == Some(ContactArrival::Digest) {
            settings.arrival = None;
        }
        repository.put(person, &settings).await.map_err(failed)
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

/// How many messages one window of a sweep takes: a background batch, so a
/// sweep never holds the store's writer for long.
pub(crate) const SWEEP_WINDOW: u32 = 50;

/// The next window of `inbox`'s mail as Focus's inbox holds it, after
/// `after`, read as the filing pass reads a message: what a sweep and its
/// preview both walk, so they walk the same mail.
pub(crate) async fn sweep_window(
    connection: &postio_storage::Connection,
    inbox: postio_model::MailboxId,
    after: &mut Option<(chrono::DateTime<chrono::Utc>, postio_model::MessageId)>,
) -> Result<Vec<postio_model::Message>, postio_storage::Error> {
    let messages = postio_storage::repository::MessageRepository::new(connection);
    let window = messages
        .focus_inbox_window(inbox, *after, SWEEP_WINDOW)
        .await?;
    if let Some(last) = window.last() {
        *after = Some((last.1, last.0));
    }
    let mut rows = Vec::with_capacity(window.len());
    for (id, _) in window {
        if let Some(row) = messages.get(id).await? {
            rows.push(row);
        }
    }
    Ok(rows)
}

/// `rows` as the filing pass is handed them: in the inbox.
pub(crate) fn in_the_inbox(rows: &[postio_model::Message]) -> Vec<postio_sync::FiledMessage<'_>> {
    rows.iter()
        .map(|row| postio_sync::FiledMessage {
            message: row,
            thread: row.thread_id,
            role: postio_model::MailboxRole::Inbox,
        })
        .collect()
}

/// How many messages a sweep of every inbox would file away now, by
/// `config`'s filtering (spec 007 FR-118): the count the person sees before
/// `Command::SweepInbox` moves exactly that. Read only, a window at a
/// time, through the pass the sweep itself files with.
pub async fn sweep_preview(
    database: &postio_storage::Store,
    config: &FocusConfig,
) -> Result<u32, String> {
    let pass = postio_sync::FocusFiling::sweeping(config);
    let failed = |_| "Postio could not read the inbox to count what would move".to_owned();
    let reader = database.read().await.map_err(failed)?;
    let inboxes = postio_storage::repository::ThreadRepository::new(&reader)
        .unified_inboxes()
        .await
        .map_err(failed)?;
    let mut count = 0u32;
    for (_, inbox) in inboxes {
        let mut after = None;
        loop {
            let rows = sweep_window(&reader, inbox, &mut after)
                .await
                .map_err(failed)?;
            if rows.is_empty() {
                break;
            }
            let moving = pass
                .would_file_away(&reader, &in_the_inbox(&rows))
                .await
                .map_err(|_| "Postio could not count what would move".to_owned())?;
            count = count.saturating_add(u32::try_from(moving.len()).unwrap_or(u32::MAX));
        }
    }
    Ok(count)
}
