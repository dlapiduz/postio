//! Updating an account's credential reuses the account form's presenter
//! ([`crate::present::onboarding`]). It is a dialog over the running shell, not a window-content
//! swap (ADR 0012 Q1), and `on_saved` only closes: the engine is already syncing.

use gtk::prelude::IsA;
use postio_client::Client;
use postio_model::ids::AccountId;

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
