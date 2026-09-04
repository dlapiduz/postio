//! Feeds the privacy pane's unsubscribe-activation log (#971) and
//! read-receipt count (#970) from storage.
//!
//! `SettingsPanel::set_unsubscribe_activations`/`set_read_receipt_count` draw
//! without knowing anything reads a database — the same split
//! `settings_egress.rs` follows for the connection list in the same panel,
//! refreshed the same way: whenever the panel comes on screen, not watched
//! live, since `postio-gtk` has no SQL of its own to watch a table with.
//!
//! Neither is account-scoped: the remote-image allow list this pane also
//! shows is a single file shared by every account, and both the activation
//! log and the receipt count follow that same shape rather than adding a
//! distinction the pane draws nowhere else.

use gtk::prelude::*;
use postio_gtk::window::Window;
use postio_storage::Database;
use postio_storage::repository::{AccountRepository, MessageRepository, UnsubscribeRepository};

use crate::Wiring;

/// Wire the privacy pane's unsubscribe-activation list and read-receipt
/// count to the store.
pub fn install(window: &Window, wiring: &Wiring) {
    refresh(window, &wiring.database);
    window.settings().connect_map({
        let window = window.clone();
        let database = wiring.database.clone();
        move |_| refresh(&window, &database)
    });
}

fn refresh(window: &Window, database: &Database) {
    let Ok(connection) = database.connection() else {
        return;
    };
    let accounts = AccountRepository::new(&connection)
        .list()
        .unwrap_or_default();

    let log = UnsubscribeRepository::new(&connection);
    let mut activations: Vec<_> = accounts
        .iter()
        .flat_map(|account| {
            log.for_account(account.id).unwrap_or_else(|error| {
                tracing::warn!(%error, "could not read the unsubscribe-activation log");
                Vec::new()
            })
        })
        .collect();
    // Each account's own rows already come back newest-first; merging more
    // than one account means re-sorting the combined list the same way.
    activations.sort_by_key(|activation| std::cmp::Reverse(activation.activated_at));
    window.settings().set_unsubscribe_activations(activations);

    let messages = MessageRepository::new(&connection);
    let read_receipt_count: u64 = accounts
        .iter()
        .map(|account| {
            messages
                .read_receipt_requested_count(account.id)
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "could not count read-receipt requests");
                    0
                })
        })
        .sum();
    window.settings().set_read_receipt_count(read_receipt_count);
}
