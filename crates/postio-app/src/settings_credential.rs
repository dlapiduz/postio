//! Updating an account's credential, from the settings panel (#464).
//!
//! Reuses `onboarding`'s probe-then-persist machinery — the same form, the
//! same connection test, the same "credential first, then the row" write
//! order (`onboarding::submit`, `onboarding::persist`) — because updating a
//! credential is exactly what [`postio_gtk::onboarding::Status::Reauthenticate`]
//! already does when `startup_route` finds a broken one automatically. This
//! is a second, manual way in, for an account whose credential is not
//! broken but has simply changed (a rotated app-specific password, most of
//! all).
//!
//! # Why a dialog, not the window's content
//!
//! [`crate::onboarding::install`] replaces the window's content because at
//! first run or startup repair there is nothing behind it to go back to.
//! Here there is: a running application, with the account's own engine
//! already syncing. ADR 0012 Q1 already decided any second onboarding
//! surface is `AdwDialog` over the shell, not a window-content swap, and
//! that decision applies unchanged to this third surface (ADR 0005 Q6a).
//!
//! # Why `on_saved` only closes
//!
//! `onboarding::submit`'s `on_saved` is what [`crate::onboarding::install`]
//! uses to run the whole first-run bootstrap once an account is written.
//! Reauthenticating an account that already has an engine needs none of
//! that — same account, same connection, only the credential (and whatever
//! server settings came with it) changed. Closing the dialog and refreshing
//! the settings panel's rows (a disabled account's own submission turns it
//! back on, per `onboarding::configure`) is the whole of it.

use std::sync::Arc;

use adw::prelude::*;
use postio_gtk::onboarding::{Onboarding, Status};
use postio_gtk::window::Window;
use postio_imap::discovery::{DiscoveryTransport, PimalayaTransport};
use postio_model::ids::AccountId;
use postio_storage::repository::AccountRepository;

use crate::Wiring;
use crate::onboarding::{ProbeCancellation, configured, probe, submit};

/// Opens a dialog over `window` letting the user re-enter `id`'s credential
/// (and, since the same form carries them, its server settings). Does
/// nothing if the account is gone by the time this runs.
pub fn install(window: &Window, wiring: &Wiring, id: AccountId) {
    let Ok(connection) = wiring.database.connection() else {
        return;
    };
    let Ok(Some(account)) = AccountRepository::new(&connection).get(id) else {
        return;
    };
    drop(connection);

    let screen = Onboarding::new();
    screen.set_address(&account.address.address);
    screen.set_status(Status::Reauthenticate(configured(&account)));
    screen.focus_password();

    let dialog = adw::Dialog::builder()
        .title("Update credential")
        .content_width(420)
        .content_height(420)
        .child(&screen)
        .build();

    let cancellation = ProbeCancellation::default();
    // Walking away from the dialog stops whatever the probe was asking, the
    // same way it does in `crate::add_account` -- ADR 0012 Q3, which is
    // about any onboarding host that can be *left* rather than about the
    // add-account one in particular. Every way out lands here: `Esc`, the
    // close button, and the parent window going away under it (#57).
    dialog.connect_closed({
        let cancellation = cancellation.clone();
        move |_| cancellation.stop()
    });
    let transport: Arc<dyn DiscoveryTransport> =
        Arc::new(PimalayaTransport::new().with_egress(wiring.egress.clone()));

    let jmap = crate::onboarding::JmapOfferSlot::default();
    screen.connect_probe({
        let screen = screen.clone();
        let runtime = wiring.runtime.clone();
        let cancellation = cancellation.clone();
        let jmap = jmap.clone();
        move |address| {
            probe(
                &screen,
                &runtime,
                address,
                &cancellation,
                Arc::clone(&transport),
                jmap.clone(),
            )
        }
    });

    screen.connect_submit({
        let screen = screen.clone();
        let wiring = wiring.clone();
        let cancellation = cancellation.clone();
        let on_saved = {
            let window = window.clone();
            let wiring = wiring.clone();
            let dialog = dialog.clone();
            move || {
                dialog.close();
                crate::settings_accounts::refresh(&window, &wiring);
            }
        };
        move |submission| {
            cancellation.stop();
            submit(
                &screen,
                &wiring,
                submission.clone(),
                jmap.borrow().clone(),
                on_saved.clone(),
            )
        }
    });

    dialog.present(Some(window));
}
