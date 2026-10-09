//! A session over one of Focus's demo stores (specs/009-focus-macos T004).
//!
//! The Mac app is photographed and compared with its design over the same
//! seeded stores the GTK app is (`postio-demo`): today's inbox in the
//! references' shape, thirty conversations, a long newsletter. They live in
//! memory, so opening one reads no Keychain and touches nobody's mail.
//!
//! Only a build with the `demo` feature can open one; any other build says
//! so, which is what lets the Swift side call this unconditionally.

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
    /// Say what sync would have said about the demo's account, as the
    /// engine says it, so screens 16 to 19 can be photographed over a store
    /// that never syncs (specs/009-focus-macos T101): `offline`, `auth` (the
    /// server refused its password), `first-sync` (a pass a third of the way
    /// through) or `synced` (a pass that finished). `false` for any other
    /// word, and always in a build without demos, which invents nothing.
    pub fn demo_state(&self, state: String) -> bool {
        show(self, &state)
    }
}

/// The events `state` stands for, emitted for the demo's first account.
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
        // Screen 17's numbers.
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

/// `seed`, or `seed:screen` -- `05`/`06` the composer's people and the
/// Harbor thread to reply to (`postio_demo::compose_demo`); a store with
/// the row screen 04 opens
/// refiled as one of the handoff's HTML bodies (`postio_demo::treatment_demo`:
/// `27` the newsletter on paper, `28` the work mail in app colours), which
/// the message window's screens are photographed over; or a file that
/// configures what a screen needs (`22`/`23` a model for the digest's
/// summary, `25` a vault, [`demo_config`]).
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
    // Screens 05 and 06: the people To completes to, and the Harbor
    // thread's recipients and body, as GTK's `shot` adds them.
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
    let options = crate::SessionOptions::in_memory_with(database).with_config_for_test(&config);
    // The search seed's files have their bytes on this machine, so the
    // attachment indexer the session starts reads them (spec 010 step 9):
    // its rows name the blobs a store under the test keys gives the bytes.
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

/// A model on this computer that nothing answers at: a socket path that
/// does not exist, so configuring it connects to nothing, anywhere. Only
/// the digest summary is switched on, and the demo's summary is already
/// written (`postio_demo`), so nothing ever asks it anything; the inbox
/// keeps the built-in detector.
#[cfg(feature = "demo")]
const DEMO_MODEL: &str = "\n[focus.model]\nendpoint = \"unix:/nonexistent/postio-demo-model.sock\"\n\
model = \"demo\"\nneeds_action = false\ndigest_summary = true\nlike_this = false\n";

/// The demo's file for `screen`, and the vault it names, if any: the
/// demo's own (`postio_demo::config`), with a model for the digest's
/// summary on screens 22 and 23 (C6), and a throwaway vault with projects
/// on screen 25 (C9), as GTK's `shot` makes one.
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

/// The vault the demo session writes into, kept for the life of the
/// process: it is a throwaway, but the session reads it until it quits.
#[cfg(feature = "demo")]
static DEMO_VAULT: std::sync::Mutex<Option<tempfile::TempDir>> = std::sync::Mutex::new(None);

#[cfg(not(feature = "demo"))]
fn open(_seed: &str) -> Result<Arc<Session>, SessionError> {
    Err(SessionError::StoreUnavailable {
        message: "This build of Postio has no demo stores.".to_owned(),
    })
}

#[cfg(all(test, feature = "demo"))]
mod tests {
    use super::*;

    /// Screen 05's To completes to the people `compose_demo` adds, from
    /// the shared rule's four letters (`MIN_COMPLETION_PREFIX`).
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

    /// The row the inbox's Harbor draft stands in, refiled as one of the
    /// handoff's HTML bodies, as GTK's screens 27-31 have it: `small:27` is
    /// the newsletter that paints its own page, and the message window's
    /// paper screens are photographed over it.
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

    /// Screen 22 is the digest on its summary, which the Mac opens only
    /// with a model configured for it (C6); screen 25 is capture, which
    /// opens only with a vault (C9). `small:22`/`small:23` and `small:25`
    /// say so in the demo's file -- a model nothing can reach, and a
    /// throwaway vault with projects -- and leave the store as seeded.
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

    /// Whether the session says an event `wanted` picks, draining what it
    /// says as the Mac's loop does (the controller hears the engine's
    /// events only as they are drained).
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

    /// Screens 16 to 19 are photographed over the demo: it never syncs, so
    /// what sync would have said is said for it, as the engine says it.
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
