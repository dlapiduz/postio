//! Issue #878, on top of #870's persistence: an OAuth account's row shows a
//! real token validity, read from the keyring through the composition root
//! rather than a placeholder.
//!
//! `gtk_settings_accounts.rs` (in `postio-gtk`) proves the panel's own
//! `set_token_expiries` seam draws the line correctly once given a value.
//! This proves the other half: that `settings_accounts::refresh` actually
//! finds an OAuth account, reads its persisted expiry from a real
//! `SecretStore`, and hands the panel back exactly that.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use std::sync::Arc;
use std::time::Duration;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use postio_account::oauth::exchange::TokenResponse;
use postio_account::oauth::token_source::OwnClientTokenSource;
use postio_account::secret::{AccountKey, MemorySecretStore, Password};
use postio_app::feed_the_window;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::EmailAddress;
use postio_session::Wiring;
use postio_storage::repository::AccountRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn an_oauth_accounts_row_shows_its_real_persisted_expiry() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    seed_small(&database, 41);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    // A second, OAuth account: `seed_small`'s own account is a password one,
    // and there is nothing to read a validity line off of it with.
    let address = "grace@example.com";
    let connection = database.connection().expect("a connection");
    let mut second =
        postio_model::Account::new("Grace", EmailAddress::new(None::<String>, address));
    second.auth = postio_model::account::AuthMethod::OAuth2;
    second.oauth = Some(postio_model::account::OAuthConfig {
        client_id: "postio-test-client".to_owned(),
        token_url: "https://example.com/token".to_owned(),
        authorize_url: "https://example.com/authorize".to_owned(),
        scopes: "mail".to_owned(),
    });
    AccountRepository::new(&connection)
        .create(&mut second)
        .expect("insert the OAuth account");
    drop(connection);

    // What #870's own persistence actually writes, through its real public
    // seam rather than a hand-rolled stand-in for it -- `seed` is exactly
    // what a completed sign-in calls.
    let secrets: Arc<dyn postio_account::secret::SecretStore> = Arc::new(MemorySecretStore::new());
    let source = OwnClientTokenSource::new(
        secrets.clone(),
        "https://example.com/token".parse().unwrap(),
        "postio-test-client",
        None,
    );
    let runtime = tokio::runtime::Runtime::new().expect("a runtime for the seed call");
    runtime
        .block_on(source.seed(
            &AccountKey::new(address),
            TokenResponse {
                access_token: Password::new("an-access-token"),
                refresh_token: Some(Password::new("a-refresh-token")),
                expires_in: Some(Duration::from_secs(41 * 24 * 60 * 60)),
                token_type: "Bearer".to_string(),
                scope: None,
            },
        ))
        .expect("seeding the token succeeds");

    let (bridge, _replies) =
        postio_core::bridge::Bridge::new(postio_core::bridge::handler_fn(|_, _| async {}))
            .expect("a runtime");
    let (sink, _events) = postio_core::bridge::event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    )
    .with_secrets(secrets);

    let window = Window::default();
    window.present();
    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let panel = window.settings();

    assert!(
        settle_until(|| rows(&panel).len() == 2),
        "expected both accounts drawn as rows, got {} row(s)",
        rows(&panel).len()
    );

    // `window.rs` builds the settings panel as a hidden overlay until asked
    // for -- `is_visible()` (which `validity_in` needs, the same way
    // `gtk_settings_accounts.rs`'s own `weight_in` does) checks the whole
    // ancestor chain, not just the label's own property, so nothing below
    // would ever read as visible without this.
    window.toggle_settings();
    assert!(
        frames(&window, 2),
        "the compositor never painted the settings panel"
    );

    // Re-found on every poll rather than captured once: `set_token_expiries`
    // rebuilds the rows from scratch the same way `set_accounts` does, so a
    // row fetched before that redraw is a detached widget the panel has
    // already replaced, and would never pick up anything.
    let oauth_row = |panel: &postio_gtk::settings::SettingsPanel| {
        rows(panel).into_iter().find(|row| {
            collect(row.upcast_ref::<gtk::Widget>(), "")
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Label>().ok())
                .any(|label| label.text().contains(address))
        })
    };

    assert!(
        settle_until(|| oauth_row(&panel).is_some_and(|row| validity_in(&row).is_some())),
        "the OAuth account's row never picked up a validity line"
    );
    let validity =
        validity_in(&oauth_row(&panel).expect("the row is still there")).expect("checked above");
    assert!(
        validity.starts_with("token valid 4") && validity.ends_with('d'),
        "expected roughly 41 days out, from the real value seed() persisted: {validity:?}"
    );

    bridge.shutdown();
}

fn validity_in(row: &gtk::ListBoxRow) -> Option<String> {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-validity",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .filter(|label| label.is_visible())
    .map(|label| label.text().to_string())
}

fn rows(panel: &postio_gtk::settings::SettingsPanel) -> Vec<gtk::ListBoxRow> {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-row",
    )
    .into_iter()
    .filter_map(|w| w.downcast().ok())
    .collect()
}

/// Run the main loop until `window` has actually painted `count` frames --
/// copied from `settings_accounts_wiring.rs` rather than shared, matching
/// that file's own reason: no dependency between the two.
fn frames(window: &Window, count: u32) -> bool {
    let left = std::rc::Rc::new(std::cell::Cell::new(count));
    window.add_tick_callback({
        let left = left.clone();
        move |_, _| {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        }
    });
    let context = glib::MainContext::default();
    let heartbeat = glib::timeout_add_local(std::time::Duration::from_millis(10), || {
        glib::ControlFlow::Continue
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while left.get() > 0 && std::time::Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
    left.get() == 0
}

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget), depth first -- copied from `settings_accounts_wiring.rs`
/// rather than shared, matching that file's own reason for copying it: no
/// dependency between the two.
fn collect(widget: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if class.is_empty() || widget.has_css_class(class) {
        found.push(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        found.extend(collect(&current, class));
        child = current.next_sibling();
    }
    found
}
