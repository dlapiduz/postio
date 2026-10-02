//! First run: the account form as the window's whole content.
//!
//! `postio_widgets::onboarding` draws canvas 3e and knows nothing about mail;
//! `postio_widgets::present::onboarding` joins it to the store's host, which
//! probes, proves and writes (ADR 0041, specs/007-postio-focus T165). What is
//! left here is the classic app's own: the form replacing an empty window
//! rather than floating over one, and bringing the window up over the new
//! account once its history is chosen. The sync-window step itself is the
//! presenter's (`Presenter::ask_sync_window`), run by Focus's first run too.
//!
//! # Nothing here blocks the UI
//!
//! The probe and the connection test are network work, done on the host's
//! runtime and answered over the client; the screen stays live throughout
//! and says which of the two it is waiting on.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use postio_account::discovery::DiscoveryTransport;
use postio_core::CommandId;
use postio_core::bridge::EventStream;
use postio_core::state::SharedState;
use postio_gtk::onboarding::{Onboarding, Status};
use postio_gtk::window::Window;
use postio_model::Account;
#[cfg(test)]
use postio_storage::Store;
#[cfg(test)]
use postio_storage::repository::AccountRepository;
use postio_widgets::present::onboarding::Presenter;

use crate::Wiring;
pub(crate) use postio_session::onboarding::configured;

/// Whether this installation has an account yet.
///
/// A store that cannot be read counts as "no account": the screen is the only
/// way forward from there anyway, and refusing to show it would leave a
/// window with nothing in it and no way to fix that.
///
/// Only its test reads it: which screen a launch opens on is
/// `startup_route`'s question, answered at the composition root.
#[cfg(test)]
pub async fn needed(database: &Store) -> bool {
    let Ok(connection) = database.connect().await else {
        return true;
    };
    AccountRepository::new(&connection)
        .list_enabled()
        .await
        .map(|accounts| accounts.is_empty())
        .unwrap_or(true)
}

/// Put the first-run screen in the window and wire it to the store's host.
///
/// The screen becomes the window's whole content — there is nothing behind it
/// to go back to, so an overlay over an empty three-pane shell would be
/// pretending otherwise. `set_content` rather than anything in
/// `postio_gtk::window`, so the frontend needs no first-run concept at all.
///
/// On success the original content goes back and the application starts as it
/// would have if the account had been there all along.
///
/// `state`, `wired` and `events` are the same three pieces `run()`'s
/// `activate` handler already holds once an account is there from the
/// start — passed through so a screen that just created one can finish the
/// exact same sequence: install every command handler and drain the two
/// event queues, not merely feed the panes. Skipping that would leave a
/// window with mail in it and no key that does anything, the same shape of
/// bug `postio-bl2` is named for.
///
/// `transport` is where the host looks a new address's servers up, and
/// `opener` how a browser sign-in's consent link is opened: supplied rather
/// than constructed, so a test drives the same screen over a mock and a
/// fake browser (#282).
///
/// # `repairing`
///
/// `Some` when this is not a first run: [`crate::startup_route`] found an
/// account the keyring will not give up a password for, and sent it back
/// here rather than into a window that cannot sync. The screen arrives
/// knowing the address and the servers, says which one thing is missing,
/// and puts the cursor in the field for it. Before `postio-67` that state
/// had nowhere to go at all: onboarding only ran when the store held no
/// account, so an account with a broken credential was permanent.
#[allow(clippy::too_many_arguments)]
pub async fn install(
    window: &Window,
    wiring: &Wiring,
    state: SharedState,
    wired: Vec<CommandId>,
    events: Rc<RefCell<Option<EventStream>>>,
    notifier: crate::notifications::Notifier,
    repairing: Option<Account>,
    transport: Arc<dyn DiscoveryTransport>,
    opener: Arc<dyn postio_account::oauth::BrowserOpener>,
) {
    // The probe and the writes are the store owner's (ADR 0041). This
    // screen is reached before any window is fed, so it connects its own
    // client, to a host over the same wiring that looks servers up through
    // `transport`.
    let frontend = crate::frontend::Frontend::in_process(&wiring.clone().with_discovery(transport));
    // Once the account is written, the same sequence `run()`'s `activate`
    // handler runs when an account is there from the start.
    let open = {
        let window = window.clone();
        let wiring = wiring.clone();
        move || {
            postio_session::blocking::now(crate::open_account(
                &window, &wiring, &state, &wired, &events, &notifier,
            ))
        }
    };
    install_for(window, &frontend, repairing, opener, open);
}

/// [`install`], over `frontend`'s client: `open` is what brings the window
/// up over the account once it is written.
pub fn install_for(
    window: &Window,
    frontend: &crate::frontend::Frontend,
    repairing: Option<Account>,
    opener: Arc<dyn postio_account::oauth::BrowserOpener>,
    open: impl Fn() + Clone + 'static,
) {
    let screen = Onboarding::new();
    let previous = window.content();
    // Under the window's chrome, not instead of it.
    //
    // `set_content` replaces everything, and everything is where this window
    // keeps its header bar — so the wizard had no title bar and no close
    // button, and a first run could only be left by killing the process
    // (there is no `quit` command either). Giving the *screen* a header
    // instead put a title bar inside the wizard's own content, which draws as
    // part of the wizard rather than as the window's and looks wrong.
    //
    // So the screen goes under chrome of its own, whose top bar is the
    // window's title bar for as long as the wizard is up —
    // `widgets::under_window_chrome` says why it is flat and title-less.
    window.set_content(Some(&postio_gtk::widgets::under_window_chrome(&screen)));
    match &repairing {
        Some(account) => {
            screen.set_address(&account.address.address);
            screen.set_status(Status::Reauthenticate(configured(account)));
            if account.oauth.is_none() {
                screen.focus_password();
            }
        }
        // A fresh form starts at its first field, which is the name.
        None => screen.focus_name(),
    }

    let presenter = Presenter::drive(&screen, &frontend.client, open_with(opener));

    // What finishes the screen for good: swap the window content back and
    // start the application over the new account, the exact sequence
    // `run()`'s `activate` handler runs when an account is there from the
    // start — installing the command handlers and draining the event
    // queues, not only feeding the panes. Without that a window fed here
    // would show mail and answer no key, which is the shape of bug
    // `postio-bl2` is named for.
    //
    // Held back from the save itself by the sync-window step (#876, run by
    // `Presenter::ask_sync_window`): the
    // account and its credential are already written by the time that step
    // shows, so `Status::Saved` is real, but the window this closure swaps
    // to should not appear until the user has chosen how far back to sync.
    // That is also why the host was asked to wait rather than start the
    // account's sync on save.
    let finish = {
        let window = window.clone();
        move || {
            window.set_content(previous.as_ref());
            open();
        }
    };
    presenter.ask_sync_window(move |_| finish());
}

/// How the classic app opens a browser sign-in's consent link: through
/// `opener`, the one the composition root was handed.
pub(crate) fn open_with(opener: Arc<dyn postio_account::oauth::BrowserOpener>) -> impl Fn(&str) {
    move |url| {
        // POSTIO-CONSENT: the consent link of a browser sign-in the person
        // began by pressing "Sign in with your browser", opened once, when
        // the host has bound the loopback listener waiting for its redirect.
        // Never on render and never retried on its own (ADR 0006 Q3).
        let opened = url
            .parse::<postio_account::oauth::Url>()
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
            .and_then(|url| opener.open(&url));
        if let Err(error) = opened {
            tracing::warn!(%error, "could not open the sign-in link");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_fresh_store_needs_onboarding_and_one_with_an_account_does_not() {
        let database = postio_storage::test_support::memory().await;
        assert!(needed(&database).await, "nothing has been provisioned yet");

        let connection = database.connect().await.expect("a connection");
        let _account = postio_storage::test_support::account(&connection).await;
        drop(connection);
        assert!(
            !needed(&database).await,
            "an account exists, so the screen is done"
        );
    }
}
