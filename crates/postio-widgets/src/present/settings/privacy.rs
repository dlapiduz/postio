//! Feeds the privacy pane's unsubscribe-activation log (#971) and
//! read-receipt count (#970) from the store's owner.
//!
//! `SettingsPanel::set_unsubscribe_activations`/`set_read_receipt_count` draw
//! without knowing anything reads a store -- the same split [`super::egress`]
//! follows for the connection list in the same pane, refreshed the same way:
//! whenever the panel comes on screen, not watched live.
//!
//! Neither is account-scoped: the remote-image allow list this pane also
//! shows is a single file shared by every account, and both the activation
//! log and the receipt count follow that same shape rather than adding a
//! distinction the pane draws nowhere else.

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;

use crate::settings::SettingsPanel;

/// Wire `panel`'s unsubscribe-activation list and read-receipt count to the
/// store's owner.
pub async fn install(panel: &SettingsPanel, client: Client) {
    // A no-op while the panel is not on screen -- see [`refresh`]. Kept
    // anyway, because `install` is also how a panel that *is* showing gets
    // its first read, and a call that costs a visibility check is not worth
    // reasoning about a second time.
    refresh(panel, &client).await;
    // Weak: the panel owns this handler, so a strong clone is a cycle and
    // the panel never frees (#1072).
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
    // **Only when the pane is on screen.** Everything below this line is a
    // store read for a figure drawn in the privacy pane, and an app installs
    // this while its window is still being fed -- so every launch spent it
    // before the first frame, for a panel that may never be opened at all.
    //
    // `read_receipt_requested_count` is the one that made that matter. It is
    // `count(*)` over every message the account holds, with no index to
    // narrow it, on the thread that has to draw: #1479 measured a real store
    // at 1249.7 ms to a first frame against a 500 ms budget. Counted over a
    // seeded store at two sizes it is 8,144 SQLite steps against 71,144 --
    // and identical statement and row counts, because an aggregate returns
    // one row however many it reads.
    //
    // Nothing is lost by waiting: [`install`] connects this to the panel's
    // own `map`, so the pane reads fresh the moment somebody looks at it.
    if !panel.is_visible() {
        return;
    }
    // One call for both figures: the host reads every account's log and
    // count on one connection.
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
