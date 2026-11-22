//! Feeds the settings panel's connection list from the egress log (#151), refreshed whenever the
//! panel comes on screen.

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;

use crate::settings::SettingsPanel;

const EGRESS_ROWS: u32 = postio_ui::privacy::CONNECTION_ROWS;

pub fn install(panel: &SettingsPanel, client: Client) {
    // Not read now: the panel is hidden at startup and `map` reads it when shown; a read here made
    // the first frame wait on
    // fifty invisible rows. Weak: the panel owns this handler (#1072).
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
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    match client.egress_log(EGRESS_ROWS).await {
        Ok(entries) => panel.set_egress(entries),
        Err(error) => tracing::warn!(%error, "could not read the egress log"),
    }
}
