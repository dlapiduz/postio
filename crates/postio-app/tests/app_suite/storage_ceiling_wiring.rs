//! `[storage] max_bytes`, edited live, actually reaches the running store
//! (#929).
//!
//! `ConfigChanged::storage` was computed on every reload and read by
//! nothing: `postio_session::enforce_storage_ceiling` was proven correct in
//! isolation (`postio-session/tests/reclaim.rs`), and the file's own header
//! promised "changes here apply live", and neither half joined the other.
//! The assertion here is not that eviction works -- that is proven
//! elsewhere -- it is that *a store this application opened* has its
//! ceiling enforced the moment `config.toml` says a new one, with no
//! restart.
//!
//! Started with no `[storage]` section at all, so the startup reclaim pass
//! (`reclaim_disk`, which reads `Wiring::storage_ceiling` once) has nothing
//! to evict and cannot be mistaken for the live path this test is actually
//! about.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. Set before the app under test starts, which is the one
// moment it is sound -- the same reasoning `wiring.rs` records.

use crate::settle_until;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, test_support};

/// A store with `count` messages, oldest first, one second apart, each
/// holding a raw-source blob of `size` distinct bytes -- the same shape
/// `postio-session/tests/reclaim.rs`'s own `store_with_messages` uses, so
/// eviction here takes the same "oldest first" mail it is proven to there.
fn store_with_messages(
    database: &postio_storage::Database,
    blobs: &BlobStore,
    count: usize,
    size: usize,
) -> Vec<postio_model::ids::BlobId> {
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut written = Vec::new();
    for index in 0..count {
        let blob = blobs
            .put(&vec![b'a' + index as u8; size])
            .expect("put a raw source");
        let received = chrono::TimeZone::timestamp_opt(&chrono::Utc, 1_000 + index as i64, 0)
            .single()
            .expect("a timestamp");
        let mut message = postio_model::Message::new(account.id, inbox, received);
        message.server.uid = Some(postio_model::ids::Uid::new(index as u32 + 1));
        message.server.uid_validity = Some(postio_model::ids::UidValidity::new(1));
        message.raw_blob_id = Some(blob.clone());
        messages.create(&mut message).expect("create");
        written.push(blob);
    }
    written
}

pub fn editing_the_ceiling_live_evicts_a_running_stores_oldest_blobs() {
    let root = std::env::temp_dir().join(format!("postio-storage-ceiling-{}", std::process::id()));
    let state_dir = root.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let blob_dir = root.join("blobs");
    std::fs::create_dir_all(&blob_dir).unwrap();
    let blobs = BlobStore::open(blob_dir, &postio_storage::test_support::blob_keys())
        .expect("a blob store");
    let written = store_with_messages(&database, &blobs, 3, 40_000);

    let config_dir = root.join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let path = config_dir.join("config.toml");
    // No `[storage]` at all: the startup pass reads `None` and evicts
    // nothing, so anything evicted below can only be the live path.
    std::fs::write(&path, "").unwrap();

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(
        database,
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    postio_gtk::config::install_at(&window, &path);
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let feeds = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let _ = feeds;

    // ── the startup pass, with no ceiling set, evicted nothing ───────────
    assert!(
        settle_until(|| written.iter().all(|blob| blobs.contains(blob))),
        "with no [storage] section at all, startup should not have evicted \
         anything -- if it did, the assertions below prove nothing about \
         the live path"
    );

    // ── lowering the ceiling live evicts the oldest, keeps the newest ────
    let budget = blobs.len_of(&written[2]).expect("len") + 16;
    std::fs::write(&path, format!("[storage]\nmax_bytes = {budget}\n")).unwrap();
    assert!(
        settle_until(|| !blobs.contains(&written[0]) && !blobs.contains(&written[1])),
        "editing config.toml's [storage] section never reached the running \
         store -- ConfigChanged::storage has a listener nothing acts on"
    );
    assert!(
        blobs.contains(&written[2]),
        "the newest blob fit the budget and should have been kept"
    );

    // ── raising it again reaches the store too, and evicts nothing further
    //    (`reclaim.rs::a_store_under_its_ceiling_loses_nothing` is the
    //    lower-level proof that a generous budget takes nothing; this is
    //    only "the live raise reaches the pass and does not misbehave") ───
    std::fs::write(&path, "[storage]\nmax_bytes = 100000000\n").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        blobs.contains(&written[2]),
        "raising the ceiling must not evict what a lower one already kept"
    );

    let _ = std::fs::remove_dir_all(&root);
}
