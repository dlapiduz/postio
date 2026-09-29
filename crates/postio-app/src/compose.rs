//! Wires the store's owner into the composer's seams.
//!
//! `postio-widgets::composer` builds a widget that edits a
//! [`Draft`](postio_model::Draft) and calls back through a handful of seams —
//! `connect_save`, `connect_recipient_suggestions`, `connect_reply_source` —
//! without knowing anything persists it. Every seam is answered by
//! [`postio_widgets::present::compose`] (`shared` below), through a
//! [`postio_client::Client`] of the host that owns the store (ADR 0041), the
//! same requests the terminal's composer makes -- and, since
//! specs/007-postio-focus T022, the same functions Focus's composer answers
//! its seams with (ADR 0043: `postio-widgets` may not depend on
//! `postio-session` or `postio-host`, so nothing reaching either can live
//! there, and nothing here does either any more).
//!
//! What is left here is the classic window's own glue around that shared
//! presenter: building the host this composer's own suites seed
//! ([`install`]), the seams only this window's chrome needs (a `mailto:`
//! link, the sidebar's selected mailbox, marking the conversation pane's own
//! messages from the account's identities), and the one seam that is a
//! window control rather than a composer seam -- a Drafts row's activation,
//! wired to whatever [`shared::install_resume`] hands back.
//!
//! # Carrying the draft's id forward
//!
//! [`Composer::connect_save`] hands its handler `&mut Draft` for exactly one
//! reason: `DraftRepository::save` is idempotent on `Draft::id`, inserting
//! once and updating forever after, and the composer has to learn whatever id
//! the first save assigned or every later autosave would insert a second row.
//! `Composer::save` writes that id back onto its own draft; `shared` keeps its
//! own record of the same id only for the one thing the composer cannot tell
//! it after the fact — which row to delete when the draft is dropped.
//!
//! The writes themselves are ordered by the host: each client has its own
//! `DraftWriter`, and a save, send or discard is handed over when the client
//! is asked, so they land in the order the composer made them (ADR 0021).

use std::cell::Cell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_gtk::composer::Composer;
use postio_gtk::window::Window;
use postio_model::DraftId;
use postio_model::ids::AccountId;
use postio_storage::{BlobStore, Store};
use postio_widgets::present::compose as shared;

/// How the composer tells the rest of the window something happened.
///
/// A callback rather than the `Feeds` themselves: what this module needs is
/// "say so", and handing it the panes would let it reach into them. It is also
/// what lets a composer test run without building a message list to ignore.
pub type Announce = shared::Announce;

/// Wires `window`'s composer to `database` for `account`: autosave with
/// crash recovery, recipient completion from contacts, replying to whatever
/// the reading pane is showing, and attaching files into `blobs`.
///
/// `showing` is the reading pane's own record of which message is on screen
/// ([`crate::reading::Showing`]), which is what `e`, `E` and `f` have to act
/// on. It is passed in rather than derived here for the reason #325 records.
///
/// The store is reached through a host of its own over a wiring built here,
/// on `runtime`: this is the signature the composer's suites call with a
/// store they seeded. The window's own composition root calls
/// [`install_with`] with the client it already holds, so a running app
/// builds no second host for its composer.
///
/// That host's news goes nowhere -- nobody reads the wiring built here -- so
/// `announce` is what tells the list a send moved a row.
pub async fn install(
    window: &Window,
    account: AccountId,
    database: Store,
    blobs: BlobStore,
    runtime: tokio::runtime::Handle,
    showing: crate::reading::Showing,
    announce: Announce,
) {
    let (sink, _unheard) = postio_core::bridge::event_channel();
    let (commands, _unsent) = postio_core::bridge::command_channel();
    let wiring = postio_session::Wiring::new(database, blobs, runtime.clone(), sink, commands);
    let client = postio_host::Host::over(wiring).connect(postio_client::protocol::ClientKind::Gtk);
    install_with(window, account, client, runtime, showing, Some(announce)).await;
}

/// [`install`] over a client the window already holds.
///
/// `announce` is for a host whose news the window does not hear; `None` when
/// it does, since the host tells every frontend a send moved a row and a
/// second announcement would reload the list twice.
pub(crate) async fn install_with(
    window: &Window,
    account: AccountId,
    client: Client,
    _runtime: tokio::runtime::Handle,
    showing: crate::reading::Showing,
    announce: Option<Announce>,
) {
    let composer = window.composer();
    composer.set_account(account);
    install_mailto(window, &composer, account);

    if let Some(found) = shared::install_identities(&composer, &client, account).await {
        // The conversation pane needs the same fact for a different reason:
        // it marks the user's own messages with an outline rather than a
        // fill (#1241). One read of the account row answers both.
        let addresses: Vec<_> = found
            .identities
            .iter()
            .map(|identity| identity.address.clone())
            .collect();
        window.conversation().set_own_addresses(&addresses);
    }
    // Which mailbox is selected is the sidebar's, read fresh on every
    // compose: the sidebar's selection changes on every click, where the
    // account's identities and named signatures change only through the
    // settings panel (see `shared::install_signature_default`).
    let sidebar = window.sidebar();
    shared::install_signature_default(&composer, &client, account, move || sidebar.selected());

    let last_id = shared::install_autosave(&composer, &client, None);
    // Only after a crash (#491): see `shared::recover_draft`. This is its
    // one caller here, before anything else consults the marker it flips.
    shared::recover_draft(&composer, &client, account, &last_id).await;
    shared::install_send(&composer, &client, Rc::clone(&last_id), account, announce);
    install_resume_from_list(window, &composer, &client, last_id);
    shared::install_recipients(&composer, &client, account);
    shared::install_reply_source(&composer, &client, {
        let showing = showing.clone();
        Rc::new(move || showing.get())
    });
    shared::install_attach(&composer, &client);
    shared::install_inline_images(&composer, &client);
}

/// A `mailto:` link opens the composer on a draft for `account`.
///
/// The link arrives at the window (`postio_gtk::app` hands every URI the
/// desktop passes to `Window::deliver_mailto`), and this is the half that
/// knows which account a new message is from. Connecting here is also what
/// releases a link that arrived *before* the store was fed — a cold launch
/// from a browser — which the window holds until somebody can act on it.
///
/// `Composer::open`, not `resume`: one composition at a time is the
/// composer's rule, so a link arriving mid-composition puts the keyboard
/// back in the draft already open rather than replacing what was typed.
/// That is said in the log, at info, because from the browser's side the
/// click did nothing.
fn install_mailto(window: &Window, composer: &Composer, account: AccountId) {
    let composer = composer.clone();
    window.connect_mailto(move |mailto| {
        if composer.is_open() {
            tracing::info!("a mailto link arrived while a composition was open; kept the open one");
        }
        composer.open(mailto.into_draft(account));
    });
}

/// Activating a draft's row in the Drafts folder opens it in the composer.
///
/// # Why activation and not the cursor
///
/// The reading pane follows the cursor — `j` over a row previews it, and
/// nothing waits for Return (#70). Taking the pane away from the reader every
/// time the cursor crossed a draft would make scrolling through the Drafts
/// folder open and close the composer under the user. So the cursor previews
/// and Return opens, which is what Return means on every other row too.
///
/// # Why the reader is the wrong answer here
///
/// A draft's row is a snapshot of a buffer the composer owns, and the reader
/// cannot edit it. Before #166 a draft's row could only ever be that snapshot
/// — a dead end with a signpost. The row now leads back to the draft, so it
/// leads to the thing that can actually be done with it.
///
/// A row with no local draft behind it is another client's draft. It still
/// opens in the reader — there is no buffer to resume, and adopting somebody
/// else's draft into one is a decision with its own questions (what becomes
/// of their server copy? whose autosave wins?) that #175 chose to leave
/// unopened for v1 rather than resolve as a side effect of this path. What
/// changed under #175 is that the reader no longer pretends it is an
/// ordinary, readable message: [`load_body_or_reason`](postio_session::reading::load_body_or_reason) recognises `\Draft`
/// with no local buffer and reports [`postio_gtk::reader::Absent::ForeignDraft`]
/// instead, whatever the body's own download state is. See
/// `docs/engineering-notes.md`.
/// What the composer says once opening a queued draft has cancelled its
/// pending send (#433).
const SEND_CANCELLED: &str = "send cancelled — you're editing this draft again";

/// Wires the row to [`shared::install_resume`], and the classic window's own
/// way of saying what came of it: a toast for a cancelled send
/// ([`SEND_CANCELLED`]), the composer's own status line for a failed one.
/// Focus says both through its dialog's footer instead
/// (`postio_focus::compose::seams`); the note itself is the same
/// [`shared::ResumeNote`] either way.
///
/// Only a row still on its way -- `row.send_state` is set exactly for a
/// message this app's own Drafts and Outbox rows carry that state for --
/// ever names a resumable draft; any other row's activation is the list's
/// own, not the composer's.
fn install_resume_from_list(
    window: &Window,
    composer: &Composer,
    client: &Client,
    last_id: Rc<Cell<Option<DraftId>>>,
) {
    // Weak: the window owns the list that owns this handler (#1072), and the
    // composer it owns too -- both outlive the note, but the closure does
    // not assume it.
    let window_weak = glib::object::ObjectExt::downgrade(window);
    let composer_weak = composer.downgrade();
    let on_note: shared::OnResumeNote = Rc::new(move |note| {
        let (Some(window), Some(composer)) = (window_weak.upgrade(), composer_weak.upgrade())
        else {
            return;
        };
        match note {
            shared::ResumeNote::Cancelled => window.show_action_completed(SEND_CANCELLED, false),
            shared::ResumeNote::Failed(reason) => {
                composer.set_status(&format!("Not sent — {reason}"));
            }
        }
    });
    let resume = shared::install_resume(composer, client, last_id, Some(on_note));
    window.list().connect_activated(move |row| {
        if row.send_state.is_some() {
            resume(row.id);
        }
    });
}

// `load_body`, `Body`, `load_body_or_reason` and `read_blob_text` moved to
// `postio_session::reading` (#608): the macOS frontend needs the same six-way
// answer about why a body is missing, and a second copy of it would reproduce
// #70's blank column rather than the fix. `load_body`'s last reader here was
// the search preview, which asks the host for it now (`Req::StoredBody`).
pub(crate) use postio_session::reading::Body;

#[cfg(test)]
mod tests {
    //! One test, and it is the point of the whole module: a draft
    //! autosaved before the process stops does not need the process to stop
    //! *cleanly* to come back.
    //!
    //! `postio-app` is a binary crate with no library target, so an
    //! integration test under `tests/` cannot link against `compose::install`
    //! at all — this has to be an inline `#[cfg(test)]` unit test in the same
    //! module, which is also why it is the *only* GTK-touching test in this
    //! crate: `adw::init()` and a display are process-wide state, and
    //! `cargo test` runs every unit test in one process unless told
    //! otherwise.
    //!
    //! **A second one is not a judgement call, it is a red run.** One was
    //! added, passed locally for weeks, and panicked the moment a runner
    //! gave both tests a display at once. A new GTK-touching scenario
    //! becomes another call from the one `#[test]` below, never another
    //! `#[test]`; `check-no-gtk-init-in-unit-tests.py` now enforces the
    //! single init this file is allowed.
    //!
    //! POSTIO-GTK-INIT: the paragraph above is the argument. A binary crate
    //! has nothing for `tests/` to link against, so this one cannot move out
    //! the way `postio-gtk`'s toast tests did. See issue #41 and
    //! `scripts/checks/check-no-gtk-init-in-unit-tests.py`.
    use postio_session::reading::load_body_or_reason;

    use gtk::gdk;

    use super::*;
    use chrono::Utc;
    use postio_model::{Draft, DraftState, EmailAddress};
    use postio_storage::repository::{AccountRepository, DraftRepository, MessageRepository};

    fn settle() {
        while gtk::glib::MainContext::default().iteration(false) {}
    }

    /// A real account row, since `DraftRepository::save`'s first insert
    /// requires one to reference.
    async fn seed_account(database: &Store) -> AccountId {
        let connection = database.connect().await.unwrap();
        let mut account = postio_model::Account::new(
            "Test",
            EmailAddress::new(None::<String>, "ada@example.com"),
        );
        AccountRepository::new(&connection)
            .create(&mut account)
            .await
            .unwrap();
        account.id
    }

    // ── `load_body_or_reason` ────────────────────────────────────────────
    //
    // Pure data: no display, no `adw::init()`. See the module doc above for
    // why a GTK-touching test does not belong beside these.

    #[tokio::test(flavor = "multi_thread")]
    async fn a_message_with_no_body_yet_names_offline_only_when_the_engine_is() {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.unwrap();
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        message.sync.body_state = postio_model::BodyState::HeadersOnly;
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .unwrap();
        drop(connection);

        let connection = database.connect().await.unwrap();

        assert!(
            matches!(
                load_body_or_reason(&connection, id, false).await,
                Body::Absent(postio_gtk::reader::Absent::Partial)
            ),
            "online and not yet fetched is the ordinary backfill wait"
        );
        assert!(
            matches!(
                load_body_or_reason(&connection, id, true).await,
                Body::Absent(postio_gtk::reader::Absent::Offline)
            ),
            "offline and not yet fetched has to say so, not promise a backfill \
             that cannot run"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_message_whose_body_already_arrived_ignores_whether_the_engine_is_offline() {
        // Offline-ness only changes the story for a body that has not landed
        // yet. A message with real bytes on disk must read the same whether
        // or not the engine happens to be connected right now.
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.unwrap();
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        message.sync.body_state = postio_model::BodyState::Full;
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .unwrap();
        drop(connection);

        let connection = database.connect().await.unwrap();

        // No blobs were ever named for it, so this is the "fetched, naming
        // no blobs" case -- `Absent::Empty` -- either way.
        assert!(matches!(
            load_body_or_reason(&connection, id, false).await,
            Body::Absent(postio_gtk::reader::Absent::Empty)
        ));
        assert!(matches!(
            load_body_or_reason(&connection, id, true).await,
            Body::Absent(postio_gtk::reader::Absent::Empty)
        ));
    }

    /// #175: a draft's row with `\Draft` set but no local `Draft` buffer
    /// behind it (`DraftRepository::by_message` is `None`) was written by
    /// another client. Even once its body backfills, opening it must not
    /// look like an ordinary, readable message -- there is nothing here that
    /// can be edited, and pretending otherwise is the dead end #175 exists
    /// to close.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_foreign_drafts_row_says_so_even_once_its_body_has_arrived() {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.unwrap();
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        message.flags.insert(postio_model::Flag::Draft);
        message.sync.body_state = postio_model::BodyState::Full;
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .unwrap();
        drop(connection);

        let connection = database.connect().await.unwrap();

        // No local `DraftRepository` row exists for this message, which is
        // exactly what makes it another client's draft rather than one this
        // machine is editing.
        assert!(matches!(
            load_body_or_reason(&connection, id, false).await,
            Body::Absent(postio_gtk::reader::Absent::ForeignDraft)
        ));
    }

    /// Both halves of #491's rule, in one `#[test]` because they cannot be
    /// two.
    ///
    /// They were two, briefly, and it cost a red CI run: `adw::init()` is
    /// process-wide and belongs to the thread that called it, `cargo test`
    /// gives every `#[test]` a thread of its own, and the second one to
    /// reach `init` panics with "Attempted to initialize GTK from two
    /// different threads". Locally the pair passed — whichever ran second
    /// found the display already gone and took the skip branch — so the
    /// race only ever showed on a runner. Two tests here cannot be made
    /// safe by ordering or a mutex: GTK objects belong to the initializing
    /// thread, so the second test has no thread it is allowed to use them
    /// from. One test, two scenarios, run in sequence, is the shape that
    /// works.
    ///
    /// The scenarios are twins — the same two runs of the app, differing
    /// only in whether the first one exited cleanly, which is precisely the
    /// question recovery has to answer.
    #[tokio::test(flavor = "multi_thread")]
    async fn recovery_reopens_a_crashed_draft_and_leaves_a_parked_one_alone() {
        if !gui_ready() {
            eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
            return;
        }

        a_crashed_session_reopens_its_draft().await;
        a_clean_exit_leaves_the_next_start_on_the_inbox().await;
    }

    /// `adw::init()` and the style/icon setup the scenarios share.
    ///
    /// Returns whether there is a display to draw on; see the caller for why
    /// exactly one test may call this.
    fn gui_ready() -> bool {
        if adw::init().is_err() || gdk::Display::default().is_none() {
            return false;
        }
        let display = gdk::Display::default().unwrap();
        postio_gtk::fonts::install().expect("the embedded fonts should install");
        postio_gtk::style::install(&display);
        postio_gtk::app::install_icons(&display);
        true
    }

    async fn a_clean_exit_leaves_the_next_start_on_the_inbox() {
        // #491, reported directly: "i reopened the app and it opened in a
        // compose window from a draft. cold start should start with the
        // inbox". `DraftState::Editing` is not evidence of a crash — Esc on
        // a draft with content parks it in exactly that state on purpose —
        // so recovery has to ask how the last session *ended*, not what the
        // drafts table holds.
        let state_dir_guard = tempfile::tempdir().expect("a state directory");
        let state_dir = state_dir_guard.path();
        // SAFETY: the scenarios run in sequence on one thread, and each
        // needs its own state directory — the clean-exit marker is the
        // variable under test, so they cannot share one. Nothing else in
        // this binary reads `XDG_STATE_HOME`.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("XDG_STATE_HOME", state_dir)
        };

        let db_path = state_dir.join("postio.db");
        let blobs_path = state_dir.join("blobs");
        let account = seed_account(
            &Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap(),
        )
        .await;
        // The ambient one: this test is a `#[tokio::test]`, and building a
        // second runtime inside one panics on drop.
        let runtime = tokio::runtime::Handle::current();

        // ── Run one: type, park the draft with Esc, exit cleanly ─────────
        {
            let database = Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap();
            let blobs =
                BlobStore::open(&blobs_path, &postio_storage::test_support::blob_keys()).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database.clone(),
                blobs,
                runtime.clone(),
                crate::reading::Showing::default(),
                // These tests are about the composer's own behaviour, not
                // about what the list does afterwards; nobody is listening.
                Rc::new(|_: &postio_core::Event| {}),
            )
            .await;
            let composer = window.composer();
            composer.open(Draft::new(account));
            settle();
            composer.test_set_subject("Finish this on Thursday");
            settle();
            composer.save();
            // Esc: a deliberate close that keeps the row Editing, "ready to
            // recover it right back".
            composer.close();
            settle();
            // The orderly exit path `run()` takes after `application.run()`
            // returns.
            postio_session::end_session(&database).await;
        }

        // ── Run two: the draft is parked, not in the way ─────────────────
        {
            let database = Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap();
            let blobs =
                BlobStore::open(&blobs_path, &postio_storage::test_support::blob_keys()).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database.clone(),
                blobs,
                runtime.clone(),
                crate::reading::Showing::default(),
                // These tests are about the composer's own behaviour, not
                // about what the list does afterwards; nobody is listening.
                Rc::new(|_: &postio_core::Event| {}),
            )
            .await;
            settle();

            assert!(
                !window.composer().is_open(),
                "a cleanly-exited session's parked draft must not take over                  the next start — the inbox is the first thing a mail client                  shows"
            );
            // Never lost: the row is still in Drafts, exactly as autosaved,
            // reachable through the Drafts folder's own resume path.
            let connection = database.connect().await.unwrap();
            let parked = DraftRepository::new(&connection)
                .list_for_account(account)
                .await
                .expect("drafts read");
            assert_eq!(parked.len(), 1);
            assert_eq!(parked[0].subject, "Finish this on Thursday");
            assert_eq!(parked[0].state, DraftState::Editing);
        }
    }

    async fn a_crashed_session_reopens_its_draft() {
        let state_dir_guard = tempfile::tempdir().expect("a state directory");
        let state_dir = state_dir_guard.path();
        // SAFETY: as above — sequential, and its own directory.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("XDG_STATE_HOME", state_dir)
        };

        let db_path = state_dir.join("postio.db");
        let blobs_path = state_dir.join("blobs");
        let account = seed_account(
            &Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap(),
        )
        .await;
        // Only `install_attach` ever spawns onto this; nothing in this test
        // attaches a file, so it exists purely to give `install` a handle.
        // The ambient one: this test is a `#[tokio::test]`, and building a
        // second runtime inside one panics on drop.
        let runtime = tokio::runtime::Handle::current();

        // ── Run one: open, type, autosave, and stop cold ─────────────────
        //
        // No `composer.close()`, no clean shutdown of anything — a crash
        // does not call those either. Going out of scope at the end of this
        // block is the whole simulation: the transaction `save()` already
        // committed is what has to survive it, not an orderly exit.
        {
            let database = Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap();
            let blobs =
                BlobStore::open(&blobs_path, &postio_storage::test_support::blob_keys()).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database,
                blobs,
                runtime.clone(),
                crate::reading::Showing::default(),
                // These tests are about the composer's own behaviour, not
                // about what the list does afterwards; nobody is listening.
                Rc::new(|_: &postio_core::Event| {}),
            )
            .await;
            let composer = window.composer();
            composer.open(Draft::new(account));
            settle();
            composer.test_set_subject("Q3 numbers, one more time");
            settle();
            // The debounce is real product behaviour and is already proven
            // in `gtk_composer_autosave.rs`; calling `save()` directly here
            // keeps this test about recovery, not about timing.
            composer.save();
        }

        // ── Run two: a fresh window, a fresh database handle, same file ──
        {
            let database = Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap();
            let blobs =
                BlobStore::open(&blobs_path, &postio_storage::test_support::blob_keys()).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database,
                blobs,
                runtime.clone(),
                crate::reading::Showing::default(),
                // These tests are about the composer's own behaviour, not
                // about what the list does afterwards; nobody is listening.
                Rc::new(|_: &postio_core::Event| {}),
            )
            .await;
            settle();

            let composer = window.composer();
            assert!(
                composer.is_open(),
                "a recovered draft should be sitting in the reading pane, not waiting to be asked for"
            );
            assert_eq!(composer.draft().subject, "Q3 numbers, one more time");
        }
    }
}
