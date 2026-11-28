//! Feeds the privacy pane's unsubscribe-activation log (#971) and read-receipt count (#970),
//! refreshed whenever the panel comes on screen. Neither is account-scoped: the remote-image allow
//! list is one file shared by every account.

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;

use crate::settings::SettingsPanel;

pub async fn install(panel: &SettingsPanel, client: Client) {
    // A no-op while the panel is not on screen (see `refresh`); kept because `install` is also how
    // a showing panel gets its first read.
    refresh(panel, &client).await;
    let weak = panel.downgrade();
    let asking = client.clone();
    panel.connect_gnupg_keyring(move |on| {
        let weak: glib::WeakRef<SettingsPanel> = weak.clone();
        let client = asking.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a oneshot receive; the host reads on its own runtime.
            let state = client.gnupg_keyring(on).await;
            if let (Some(panel), Ok(state)) = (weak.upgrade(), state) {
                panel.set_gnupg_keyring(&state);
            }
        });
    });
    // Weak: the panel owns this handler (#1072).
    let weak = panel.downgrade();
    panel.connect_map(move |_| {
        let weak: glib::WeakRef<SettingsPanel> = weak.clone();
        let client = client.clone();
        postio_core::blocking::now(async move {
            if let Some(panel) = weak.upgrade() {
                refresh(&panel, &client).await;
            }
        })
    });
}

async fn refresh(panel: &SettingsPanel, client: &Client) {
    // Only when the pane is on screen: `read_receipt_requested_count` is `count(*)` over every
    // message with no index to narrow it, and an app installs this while its window is still being
    // fed (#1479: 1249.7 ms to a first frame against a 500 ms budget). Nothing is lost by waiting:
    // `map` reads fresh the moment somebody looks.
    if !panel.is_visible() {
        return;
    }
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    let log = match client.privacy_log().await {
        Ok(log) => log,
        Err(error) => {
            tracing::warn!(%error, "could not read the privacy pane's log");
            return;
        }
    };
    panel.set_unsubscribe_activations(log.activations);
    panel.set_read_receipt_count(log.read_receipts);
}
