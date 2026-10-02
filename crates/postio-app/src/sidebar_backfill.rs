//! Wires the classic sidebar's per-folder backfill toggle to the store (ADR
//! 0016, #350): the shared write
//! (`postio_widgets::present::settings::backfill`, T233), drawn again in the
//! sidebar from its answer so the context menu's wording is right the
//! moment it is reopened -- nothing emits `MailboxesChanged` for a local
//! column write.

use gtk::glib;
use postio_client::Client;
use postio_gtk::window::Window;

/// Wires `window`'s sidebar to the store's owner: skipping or resuming a
/// folder's background backfill from its own context menu.
pub async fn install(window: &Window, client: Client) {
    // Weak: the window owns the sidebar that owns this handler (#1072).
    let weak = glib::object::ObjectExt::downgrade(window);
    window
        .sidebar()
        .connect_backfill_exclusion_changed(move |id, excluded| {
            let weak: glib::WeakRef<Window> = weak.clone();
            let client = client.clone();
            postio_session::blocking::now(async move {
                let answer = postio_widgets::present::settings::backfill::set_excluded(
                    &client, id, excluded,
                )
                .await;
                if let Some(all) = answer.filter(|all| !all.is_empty())
                    && let Some(window) = weak.upgrade()
                {
                    window.sidebar().set_mailboxes(&all);
                }
            })
        });
}
