//! Updating an account's credential, from the settings panel (#464).
//!
//! Reuses the account form's presenter -- the same form, the same connection
//! test, the same "credential first, then the row" write order
//! ([`crate::present::onboarding`]) -- because updating a credential is
//! exactly what the form's `Status::Reauthenticate` already does when a
//! broken one is found at startup or on Focus's sign-in banner. This is a
//! second, manual way in, for an account whose credential is not broken but
//! has simply changed (a rotated app-specific password, most of all).
//!
//! # Why a dialog, not the window's content
//!
//! A first run replaces the window's content because there is nothing
//! behind it to go back to. Here there is: a running application, with the
//! account's own engine already syncing. ADR 0012 Q1 already decided any
//! second onboarding surface is `AdwDialog` over the shell, not a
//! window-content swap, and that decision applies unchanged to this surface
//! (ADR 0005 Q6a).
//!
//! # Why `on_saved` only closes
//!
//! Reauthenticating an account that already has an engine needs none of a
//! first run's bootstrap -- same account, same connection, only the
//! credential (and whatever server settings came with it) changed. Closing
//! the dialog and refreshing the settings panel's rows (a disabled account's
//! own submission turns it back on) is the whole of it.

use gtk::prelude::IsA;
use postio_client::Client;
use postio_model::ids::AccountId;

/// Opens a dialog over `parent` letting the person re-enter `id`'s
/// credential (and, since the same form carries them, its server settings).
/// `open_link` opens a browser sign-in's consent link, as the app opens one;
/// `on_saved` runs once the credential is written. Does nothing if the
/// account is gone by the time this runs.
pub async fn update(
    parent: &impl IsA<gtk::Widget>,
    client: &Client,
    id: AccountId,
    open_link: impl Fn(&str) + 'static,
    on_saved: impl Fn() + 'static,
) {
    // POSTIO-GLIB-SAFE: reading the account is a client call, a oneshot
    // receive; the host answers on its own runtime.
    crate::present::onboarding::update_credential(
        parent,
        client,
        |account| account.id == id,
        open_link,
        on_saved,
    )
    .await;
}
