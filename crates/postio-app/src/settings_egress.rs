//! Feeds the settings panel's connection list from the egress log (#151).
//!
//! The panel draws the rows (`SettingsPanel::set_egress`) without knowing
//! anything reads a database — the same split `settings_accounts.rs`
//! follows. Refreshed when the settings command runs, so the list a person
//! opens is current rather than a snapshot from launch.

use gtk::glib;
use gtk::prelude::*;
use postio_gtk::window::Window;
use postio_storage::Database;
use postio_storage::repository::EgressLogRepository;

use crate::Wiring;

/// How many connections the panel lists. An audit surface, not an archive:
/// the store keeps everything, and the newest screenful answers "what has
/// this thing been talking to".
const EGRESS_ROWS: u32 = 50;

/// Wire the settings panel's connection list to the store.
pub fn install(window: &Window, wiring: &Wiring) {
    refresh(window, &wiring.database);
    // `map` fires every time the panel comes on screen — `Ctrl+comma`, the
    // menu, wherever — which is exactly "the moment the person looks".
    // `CommandId::Settings` never reaches `connect_command`: the window
    // answers it itself, and handled commands are not delivered twice.
    // Weak: the window owns the settings panel that owns this handler
    // (#1072).
    let weak = glib::object::ObjectExt::downgrade(window);
    window.settings().connect_map({
        let database = wiring.database.clone();
        move |_| {
            if let Some(window) = weak.upgrade() {
                refresh(&window, &database);
            }
        }
    });
}

fn refresh(window: &Window, database: &Database) {
    let Ok(connection) = database.connection() else {
        return;
    };
    match EgressLogRepository::new(&connection).recent(EGRESS_ROWS) {
        Ok(entries) => window.settings().set_egress(entries),
        Err(error) => tracing::warn!(%error, "could not read the egress log"),
    }
}
