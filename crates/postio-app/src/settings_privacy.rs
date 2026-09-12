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

use gtk::glib;
use gtk::prelude::*;
use postio_gtk::window::Window;
use postio_storage::Database;
use postio_storage::repository::{AccountRepository, MessageRepository, UnsubscribeRepository};

use crate::Wiring;

/// Wire the privacy pane's unsubscribe-activation list and read-receipt
/// count to the store.
pub fn install(window: &Window, wiring: &Wiring) {
    // A no-op at startup, where the panel is not on screen -- see
    // [`refresh`]. Kept anyway, because `install` is also how a window that
    // *is* showing the panel gets its first read, and a call that costs a
    // visibility check is not worth reasoning about a second time.
    refresh(window, &wiring.database);
    // Weak: the window owns the settings panel that owns this handler, so a
    // strong clone is a cycle and the window never frees (#1072).
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
    // **Only when the pane is on screen.** Everything below this line is a
    // store read for a figure drawn in the privacy pane, and [`install`] runs
    // inside `feed_the_window` -- so every launch spent it before the first
    // frame, for a panel that may never be opened at all.
    //
    // `read_receipt_requested_count` is the one that made that matter. It is
    // `count(*)` over every message the account holds, with no index to
    // narrow it, on the thread that has to draw: #1479 measured a real store
    // at 1249.7 ms to a first frame against a 500 ms budget, with 1044.1 ms
    // of it after the window existed and none of it waiting on the network.
    // A scan of 81,000 rows was inside that. Counted over a seeded store at
    // two sizes it is 8,144 SQLite steps against 71,144 -- and identical
    // statement and row counts, because an aggregate returns one row however
    // many it reads, which is how it sat there with two counted budgets
    // already in the workspace and neither able to see it.
    //
    // Nothing is lost by waiting: [`install`] connects this to the panel's
    // own `map`, so the pane reads fresh the moment somebody looks at it --
    // the same trade `settings_accounts::refresh` makes for what an account's
    // mail weighs (#871), and `Window::open_settings` for the allow list.
    if !gtk::prelude::WidgetExt::is_visible(&window.settings()) {
        return;
    }
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
