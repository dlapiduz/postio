//! A session over one of Focus's demo stores, in memory: it reads no Keychain and touches nobody's
//! mail.
//! Without the `demo` feature a build says so, so Swift can call this unconditionally.

use std::sync::Arc;

use crate::session::{Session, SessionError};

#[uniffi::export]
impl Session {
    /// A session over the demo store `seed` names ("small",
    /// "thirty-threads", ...), in memory.
    #[uniffi::constructor]
    pub fn open_demo(seed: String) -> Result<Arc<Session>, SessionError> {
        open(&seed)
    }
}

#[uniffi::export]
impl Session {
    /// The file a demo imports contacts from, in place of the open panel;
    /// empty in a build without demos, which invents nothing.
    pub fn demo_contacts_file(&self) -> Vec<u8> {
        contacts_file()
    }
}

#[cfg(feature = "demo")]
fn contacts_file() -> Vec<u8> {
    postio_demo::IMPORT_VCF.as_bytes().to_vec()
}

#[cfg(not(feature = "demo"))]
fn contacts_file() -> Vec<u8> {
    Vec::new()
}

#[uniffi::export]
impl Session {
    /// `offline`, `auth`, `first-sync` or `synced`; `false` for any other word, and always without
    /// demos.
    pub fn demo_state(&self, state: String) -> bool {
        show(self, &state)
    }
}

#[uniffi::export]
impl Session {
    /// `reply` files Ana's reply in the Harbor thread, then announces `NewMail` and
    /// `ThreadChanged`;
    /// `false` for any other word, and always without demos.
    pub fn demo_arrive(&self, what: String) -> bool {
        arrive(self, &what)
    }
}

#[cfg(feature = "demo")]
fn arrive(session: &Session, what: &str) -> bool {
    use postio_core::Event;
    if what != "reply" {
        return false;
    }
    let Some(account) = crate::session::blocking(session.accounts())
        .first()
        .map(|account| postio_model::AccountId::from(account.id))
    else {
        return false;
    };
    let Some((database, _)) = session.store_and_blobs() else {
        return false;
    };
    let Ok(arrival) =
        crate::session::blocking(postio_demo::deliver_harbor_reply(&database, account))
    else {
        return false;
    };
    session.emit_for_test(Event::NewMail {
        account,
        mailbox: arrival.mailbox,
        messages: vec![arrival.message],
    });
    session.emit_for_test(Event::ThreadChanged {
        account,
        thread: arrival.thread,
        members_added: vec![arrival.message],
    });
    true
}

#[cfg(not(feature = "demo"))]
fn arrive(_session: &Session, _what: &str) -> bool {
    false
}

#[cfg(feature = "demo")]
fn show(session: &Session, state: &str) -> bool {
    use postio_core::{ConnectionState, Event, FailureReason};
    let Some(account) = crate::session::blocking(session.accounts())
        .first()
        .map(|account| postio_model::AccountId::from(account.id))
    else {
        return false;
    };
    let connection = |state| Event::ConnectionChanged { account, state };
    let progress = |done, total| Event::SyncProgress {
        account,
        done,
        total,
    };
    let events = match state {
        "offline" => vec![connection(ConnectionState::Offline)],
        "auth" => vec![connection(ConnectionState::Failing {
            reason: FailureReason::Auth,
        })],
        "first-sync" => vec![
            connection(ConnectionState::Online),
            progress(12_408, 18_204),
        ],
        "synced" => vec![connection(ConnectionState::Online), progress(1, 1)],
        _ => return false,
    };
    for event in events {
        session.emit_for_test(event);
    }
    true
}

#[cfg(not(feature = "demo"))]
fn show(_session: &Session, _state: &str) -> bool {
    false
}

/// `seed`, or `seed:screen`: `05`/`06` composer people and the Harbor thread, `27`/`28` HTML-body
/// treatments, `22`/`23` a model for the digest summary, `25` a vault.
#[cfg(feature = "demo")]
fn open(name: &str) -> Result<Arc<Session>, SessionError> {
    let (seed, screen) = match name.split_once(':') {
        Some((seed, screen)) => (seed, Some(screen)),
        None => (name, None),
    };
    let seed = postio_demo::Seed::from_id(seed).ok_or_else(|| SessionError::StoreUnavailable {
        message: format!("There is no demo store called \u{201c}{name}\u{201d}."),
    })?;
    let (database, account) = crate::session::blocking(postio_demo::seeded(seed));
    if let Some(screen @ ("27" | "28" | "29" | "30" | "31")) = screen {
        crate::session::blocking(postio_demo::treatment_demo(&database, account, screen));
    }
    if matches!(screen, Some("05" | "06")) {
        crate::session::blocking(postio_demo::compose_demo(&database, account));
    }
    let (config, vault) = demo_config(screen).map_err(|error| SessionError::StoreUnavailable {
        message: format!("The demo's vault could not be made: {error}"),
    })?;
    if let Some(vault) = vault
        && let Ok(mut kept) = DEMO_VAULT.lock()
    {
        *kept = Some(vault);
    }
    // Reads the demo's own file, never the installed one.
    let file = demo_config_file();
    std::fs::write(&file, &config).map_err(|error| SessionError::StoreUnavailable {
        message: format!("The demo's settings could not be written: {error}"),
    })?;
    let options = crate::SessionOptions::in_memory_with(database).with_config_file_for_test(&file);
    // `POSTIO_DEMO_AGENTS=<socket>` has the demo answer agents there (ADR 0041); off unless asked,
    // since the demo otherwise writes nothing to disk.
    let options = match std::env::var_os("POSTIO_DEMO_AGENTS").filter(|path| !path.is_empty()) {
        Some(socket) => options.answering_agents_at(socket),
        None => options,
    };
    // The search seed's files have bytes on this machine, so the attachment indexer reads them.
    let options = if seed == postio_demo::Seed::Search {
        let scratch = tempfile::tempdir().map_err(|error| SessionError::StoreUnavailable {
            message: format!("The demo's files could not be stored: {error}"),
        })?;
        let blobs = postio_storage::BlobStore::open(
            scratch.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .and_then(|blobs| postio_demo::store_search_blobs(&blobs).map(|_| blobs))
        .map_err(|error| SessionError::StoreUnavailable {
            message: format!("The demo's files could not be stored: {error}"),
        })?;
        options.with_blobs_for_test(blobs, scratch)
    } else {
        options
    };
    Session::open(options)
}

/// A model on a socket path that does not exist, so configuring it connects to nothing.
#[cfg(feature = "demo")]
const DEMO_MODEL: &str = "\n[focus.model]\nendpoint = \"unix:/nonexistent/postio-demo-model.sock\"\n\
model = \"demo\"\nneeds_action = false\ndigest_summary = true\nlike_this = false\n";

/// The demo's file for `screen`, and the vault it names, if any.
#[cfg(feature = "demo")]
fn demo_config(screen: Option<&str>) -> std::io::Result<(String, Option<tempfile::TempDir>)> {
    let base = postio_demo::config();
    match screen {
        Some("22" | "23") => Ok((format!("{base}{DEMO_MODEL}"), None)),
        Some("25") => {
            let vault = postio_demo::demo_vault()?;
            let text = format!(
                "{base}\n[focus.vault]\npath = \"{}\"\nprojects = \"Projects\"\n",
                vault.path().display()
            );
            Ok((text, Some(vault)))
        }
        _ => Ok((base, None)),
    }
}

/// Kept for the life of the process: the session reads it until it quits.
#[cfg(feature = "demo")]
static DEMO_VAULT: std::sync::Mutex<Option<tempfile::TempDir>> = std::sync::Mutex::new(None);

/// The demo's own `config.toml` when this is a demo process, so a demo never reads, seeds or saves
/// anybody's settings. `None` otherwise, and always without demos.
pub(crate) fn config_file() -> Option<std::path::PathBuf> {
    #[cfg(feature = "demo")]
    {
        config_file_in(&|name| std::env::var(name).ok())
    }
    #[cfg(not(feature = "demo"))]
    {
        None
    }
}

#[cfg(feature = "demo")]
static DEMO_CONFIG: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();

#[cfg(feature = "demo")]
fn config_file_in(env: &dyn Fn(&str) -> Option<String>) -> Option<std::path::PathBuf> {
    if DEMO_CONFIG.get().is_none() && env("POSTIO_DEMO").is_none_or(|seed| seed.is_empty()) {
        return None;
    }
    Some(demo_config_file())
}

/// Made if absent. With no throwaway directory, a path nothing can write: falling back to the
/// installed file must never happen.
#[cfg(feature = "demo")]
fn demo_config_file() -> std::path::PathBuf {
    let made = DEMO_CONFIG
        .get()
        .map(|dir| Ok(dir.path().to_path_buf()))
        .unwrap_or_else(|| {
            let dir = tempfile::Builder::new()
                .prefix("postio-demo-config")
                .tempdir()?;
            std::fs::write(
                dir.path().join(postio_config::paths::CONFIG_FILE_NAME),
                postio_demo::config(),
            )?;
            Ok::<_, std::io::Error>(DEMO_CONFIG.get_or_init(|| dir).path().to_path_buf())
        });
    match made {
        Ok(dir) => dir.join(postio_config::paths::CONFIG_FILE_NAME),
        Err(error) => {
            tracing::warn!(%error, "the demo has no settings file; Settings will show none");
            std::path::PathBuf::from("/nonexistent/postio-demo/config.toml")
        }
    }
}

#[cfg(not(feature = "demo"))]
fn open(_seed: &str) -> Result<Arc<Session>, SessionError> {
    Err(SessionError::StoreUnavailable {
        message: "This build of Postio has no demo stores.".to_owned(),
    })
}

#[cfg(all(test, feature = "demo"))]
mod tests {
    use super::*;

    #[test]
    fn screen_05s_people_complete_the_to_field() {
        let session = Session::open_demo("small:05".to_owned()).expect("the demo opens");
        let draft = crate::session::blocking(session.new_draft()).expect("a draft");
        let said: Vec<String> = session
            .recipient_suggestions(draft.account, "grac".to_owned(), 8, Vec::new())
            .into_iter()
            .map(|suggestion| suggestion.label)
            .collect();
        assert!(
            said.iter().any(|label| label.contains("grace@example.org")),
            "{said:?}"
        );
        session.shutdown();
    }

    #[test]
    fn the_small_demo_opens_on_focus_s_inbox() {
        let session = Session::open_demo("small".to_owned()).expect("the demo opens");
        let counts = session.focus_counts().expect("its counts");
        assert!(counts.conversations > 5, "{counts:?}");
        session.shutdown();
    }

    #[test]
    fn a_screen_after_the_seed_refiles_the_opened_row() {
        let session = Session::open_demo("small:27".to_owned()).expect("the demo opens");
        session.open_focus(crate::FocusScopeFfi::Inbox);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let found = loop {
            let found = (0..12).any(|row| {
                session
                    .focus_row_at(row)
                    .is_some_and(|row| row.subject == "Issue 48: The quiet season")
            });
            if found || std::time::Instant::now() > deadline {
                break found;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert!(found, "the newsletter is in the inbox");
        session.shutdown();
    }

    /// Screens 22 and 23 need a model configured, 25 a vault: the demo's file says so and the store
    /// stays as seeded.
    #[test]
    fn the_digest_and_capture_screens_have_a_model_and_a_vault() {
        let (text, vault) = demo_config(Some("22")).expect("a config");
        let config = postio_config::Config::from_toml_str(&text).expect("it parses");
        assert!(
            config
                .focus
                .model_for(postio_config::model::ModelFeature::DigestSummary)
                .is_some(),
            "{text}"
        );
        assert!(
            config
                .focus
                .model_for(postio_config::model::ModelFeature::NeedsAction)
                .is_none(),
            "the inbox keeps the built-in detector: {text}"
        );
        assert!(vault.is_none());

        let (text, vault) = demo_config(Some("25")).expect("a config");
        let config = postio_config::Config::from_toml_str(&text).expect("it parses");
        let vault = vault.expect("a vault");
        let configured = config.focus.vault.expect("[focus.vault]");
        assert_eq!(configured.root().as_deref(), Some(vault.path()));
        assert!(vault.path().join("Projects/Harbor.md").exists());
        assert!(config.focus.model.is_none(), "{text}");

        let (text, vault) = demo_config(None).expect("a config");
        assert_eq!(text, postio_demo::config());
        assert!(vault.is_none());
    }

    /// The demo touches nobody's files: every ask for `config.toml` gets the demo's throwaway file.
    #[test]
    fn a_demo_s_config_file_is_its_own_never_the_installed_one() {
        let session = Session::open_demo("small:25".to_owned()).expect("the demo opens");
        let path = crate::settings::settings_path().expect("a path");
        let installed = postio_config::paths::config_path().expect("the installed path");
        assert!(
            !std::path::Path::new(&path).starts_with(installed.parent().expect("its directory")),
            "{path}"
        );
        let text = crate::settings::settings_load(path.clone());
        let config = postio_config::Config::from_toml_str(&text).expect("the demo's file parses");
        assert!(
            config.focus.vault.is_some(),
            "the file the session reads: {text}"
        );
        session.shutdown();
    }

    /// Before any demo has opened, `POSTIO_DEMO` alone is enough.
    #[test]
    fn asking_for_a_demo_is_enough_to_keep_the_installed_file_out_of_it() {
        if DEMO_CONFIG.get().is_none() {
            assert_eq!(config_file_in(&|_| None), None, "no demo, no redirect");
            assert_eq!(config_file_in(&|_| Some(String::new())), None);
        }
        let asked = |name: &str| (name == "POSTIO_DEMO").then(|| "small".to_owned());
        let file = config_file_in(&asked).expect("a demo's file");
        let installed = postio_config::paths::config_path().expect("the installed path");
        assert!(!file.starts_with(installed.parent().expect("its directory")));
    }

    /// Whether the session says an event `wanted` picks, draining as the Mac's loop does.
    fn says(session: &Session, mut wanted: impl FnMut(&crate::UiEvent) -> bool) -> bool {
        crate::session::blocking(async {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
            while let Ok(Some(event)) =
                tokio::time::timeout_at(deadline, session.next_event()).await
            {
                if wanted(&event) {
                    return true;
                }
            }
            false
        })
    }

    #[test]
    fn a_demo_can_be_shown_offline_signed_out_or_syncing() {
        let session = Session::open_demo("small".to_owned()).expect("the demo opens");
        assert!(session.demo_state("offline".to_owned()));
        assert!(
            says(&session, |event| matches!(
                event,
                crate::UiEvent::FocusBanner { banner: Some(banner) } if banner.heading == "You're offline"
            )),
            "the offline banner"
        );
        assert!(session.demo_state("auth".to_owned()));
        assert!(
            says(&session, |event| matches!(
                event,
                crate::UiEvent::FocusBanner { banner: Some(banner) } if banner.error && banner.account.is_some()
            )),
            "the sign-in banner, naming the demo's account"
        );
        session.shutdown();

        let session = Session::open_demo("small".to_owned()).expect("the demo opens");
        assert!(session.demo_state("first-sync".to_owned()));
        assert!(
            says(&session, |event| matches!(
                event,
                crate::UiEvent::FocusBanner { banner: Some(banner) } if banner.progress.is_some()
            )),
            "the first sync, with its progress"
        );
        session.shutdown();

        let session = Session::open_demo("small".to_owned()).expect("the demo opens");
        assert!(session.demo_state("synced".to_owned()));
        assert!(
            says(&session, |event| matches!(
                event,
                crate::UiEvent::FocusSyncLabel {
                    mark: crate::SyncMarkFfi::Synced,
                    ..
                }
            )),
            "a pass that finished says when"
        );
        assert!(
            !session.demo_state("no-such-state".to_owned()),
            "an unknown state says so"
        );
        session.shutdown();
    }

    #[test]
    fn an_unknown_demo_says_so() {
        assert!(Session::open_demo("no-such-seed".to_owned()).is_err());
    }
}

#[cfg(all(test, not(feature = "demo")))]
mod without {
    use super::*;

    #[test]
    fn a_build_without_demos_refuses_one() {
        assert!(Session::open_demo("small".to_owned()).is_err());
    }

    #[test]
    fn a_build_without_demos_invents_no_state() {
        let session =
            Session::open(crate::SessionOptions::in_memory()).expect("an in-memory session");
        assert!(!session.demo_state("offline".to_owned()));
        session.shutdown();
    }
}
