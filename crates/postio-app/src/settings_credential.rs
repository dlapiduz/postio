//! Updating an account's credential, from the classic window's settings:
//! the shared dialog (`postio_widgets::present::settings::credential`, T233)
//! over this window, its rows read again once the credential is written.

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

/// [`install`], for a window whose store's owner may be another process.
pub async fn install_for(window: &Window, frontend: &Frontend, id: AccountId) {
    let refresh = {
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
    open(window, frontend, id, Box::new(refresh)).await;
}

/// The dialog, with `saved` run once the credential is written.
pub(crate) async fn open(
    window: &Window,
    frontend: &Frontend,
    id: AccountId,
    saved: Box<dyn Fn()>,
) {
    // POSTIO-GLIB-SAFE: reading the account is a client call, a oneshot
    // receive; the host answers on its own runtime.
    postio_widgets::present::settings::credential::update(
        window,
        &frontend.client,
        id,
        crate::onboarding::open_with(Arc::new(
            postio_account::oauth::browser::SystemBrowserOpener,
        )),
        saved,
    )
    .await;
}
