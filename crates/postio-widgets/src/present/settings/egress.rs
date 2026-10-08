//! Feeds the settings panel's connection list from the egress log (#151).
//!
//! The panel draws the rows (`SettingsPanel::set_egress`) without knowing
//! anything reads a store -- the same split [`super::accounts`] follows.
//! Refreshed whenever the panel comes on screen, so the list a person opens
//! is current rather than a snapshot from launch.

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;

use crate::settings::SettingsPanel;

/// How many connections the panel lists: `postio_ui::privacy`'s, which the
/// terminal's pane asks for too.
const EGRESS_ROWS: u32 = postio_ui::privacy::CONNECTION_ROWS;

/// Wire `panel`'s connection list to the store's owner.
pub fn install(panel: &SettingsPanel, client: Client) {
    // Not read now: the panel is hidden at startup, and `map` below reads it
    // the moment it is shown -- a read here was the first frame waiting on
    // fifty rows nobody could see. `map` fires every time the panel comes on
    // screen, however it was opened, which is exactly "the moment the person
    // looks". Weak: the panel owns this handler (#1072).
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

/// One call: the newest screenful, read by the store's owner.
async fn refresh(panel: &SettingsPanel, client: &Client) {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    match client.egress_log(EGRESS_ROWS).await {
        Ok(entries) => panel.set_egress(entries),
        Err(error) => tracing::warn!(%error, "could not read the egress log"),
    }
}
