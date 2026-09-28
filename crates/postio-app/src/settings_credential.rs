//! Updating an account's credential, from the settings panel (#464).
//!
//! Reuses the account form's presenter — the same form, the same connection
//! test, the same "credential first, then the row" write order
//! (`postio_widgets::present::onboarding`, `onboarding::persist`) — because updating a
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
//! The saved-account hook is what [`crate::onboarding::install`] uses to
//! run the whole first-run bootstrap once an account is written.
//! Reauthenticating an account that already has an engine needs none of
//! that — same account, same connection, only the credential (and whatever
//! server settings came with it) changed. Closing the dialog and refreshing
//! the settings panel's rows (a disabled account's own submission turns it
//! back on, per `onboarding::configure`) is the whole of it.

use std::sync::Arc;

use postio_gtk::window::Window;
use postio_model::ids::AccountId;

use crate::Wiring;
use crate::frontend::Frontend;

/// Opens a dialog over `window` letting the user re-enter `id`'s credential
/// (and, since the same form carries them, its server settings). Does
/// nothing if the account is gone by the time this runs.
pub async fn install(window: &Window, wiring: &Wiring, id: AccountId) {
    // The row and the writes are the store owner's (ADR 0041), asked through
    // a client of this dialog's own, over the same wiring.
    install_for(window, &Frontend::in_process(wiring), id).await;
}

/// [`install`], for a window whose store's owner may be another process:
/// the row, the proof and the writes go through `frontend`'s client.
///
/// The dialog is `postio_widgets::present::onboarding::update_credential`,
/// the one Focus's sign-in banner opens too; what is the classic app's is
/// refreshing its settings panel once the credential is written.
pub async fn install_for(window: &Window, frontend: &Frontend, id: AccountId) {
    let on_saved = {
        let window = gtk::glib::object::ObjectExt::downgrade(window);
        let frontend = frontend.clone();
        move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            postio_session::blocking::now(crate::settings_accounts::refresh(
                &window,
                &frontend,
                &frontend.client,
            ));
        }
    };
    // POSTIO-GLIB-SAFE: reading the account is a client call, a oneshot
    // receive; the host answers on its own runtime.
    postio_widgets::present::onboarding::update_credential(
        window,
        &frontend.client,
        |account| account.id == id,
        crate::onboarding::open_with(Arc::new(
            postio_account::oauth::browser::SystemBrowserOpener,
        )),
        on_saved,
    )
    .await;
}
