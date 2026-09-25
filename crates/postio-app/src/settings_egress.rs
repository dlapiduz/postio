//! Feeds the settings panel's connection list from the egress log (#151).
//!
//! The panel draws the rows (`SettingsPanel::set_egress`) without knowing
//! anything reads a store — the same split `settings_accounts.rs`
//! follows. Refreshed when the settings command runs, so the list a person
//! opens is current rather than a snapshot from launch.

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_gtk::window::Window;

/// How many connections the panel lists: `postio_ui::privacy`'s, which the
/// terminal's pane asks for too.
const EGRESS_ROWS: u32 = postio_ui::privacy::CONNECTION_ROWS;

/// Wire the settings panel's connection list to the store's owner.
pub async fn install(window: &Window, client: Client) {
    // Not read now: the panel is hidden at startup, and `map` below reads it
    // the moment it is shown -- a read here was the first frame waiting on
    // fifty rows nobody could see.
    // `map` fires every time the panel comes on screen — `Ctrl+comma`, the
    // menu, wherever — which is exactly "the moment the person looks".
    // `CommandId::Settings` never reaches `connect_command`: the window
    // answers it itself, and handled commands are not delivered twice.
    // Weak: the window owns the settings panel that owns this handler
    // (#1072).
    let weak = glib::object::ObjectExt::downgrade(window);
    window.settings().connect_map({
        move |_| {
            postio_session::blocking::now(async {
                if let Some(window) = weak.upgrade() {
                    refresh(&window, &client).await;
                }
            })
        }
    });
}

/// One call: the newest screenful, read by the store's owner.
async fn refresh(window: &Window, client: &Client) {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    match client.egress_log(EGRESS_ROWS).await {
        Ok(entries) => window.settings().set_egress(entries),
        Err(error) => tracing::warn!(%error, "could not read the egress log"),
    }
}
