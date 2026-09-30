//! The reader costs no web process, and what it holds does not grow with
//! the messages viewed (spec 006 FR-027, SC-006, T144).
//!
//! Under WebKit every reader was a web process, and ADR 0032's whole
//! argument was about keeping their number down. The reader draws with
//! `postio-render` now, in this process, so moving through ten
//! conversations must start none -- counted the way `gtk_reader` used to
//! count them: this process's descendants named `WebKitWebProces` (Linux
//! cuts `comm` to fifteen characters). The composer is still WebKit, and
//! the app warms it on an idle turn, so the count is taken against a
//! baseline after that.
//!
//! Each move is also held to its render counts, and after the tenth the
//! tile cache and the live snapshots are what they were after the first.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; set before the app starts.

use std::time::Duration;

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_storage::seed::seed_small_with_bodies;
use postio_storage::{BlobStore, test_support};

/// This process's WebKit web processes, by pid.
fn web_processes() -> Vec<i32> {
    let me = std::process::id() as i32;
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .is_ok_and(|comm| comm.trim() == "WebKitWebProces")
                && descends_from(*pid, me)
        })
        .collect()
}

/// Whether `pid`'s parent chain reaches `ancestor`: WebKit puts its
/// processes under a `bwrap` sandbox, so they are grandchildren.
fn descends_from(mut pid: i32, ancestor: i32) -> bool {
    for _ in 0..16 {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        // The parent is the field after the parenthesised name.
        let Some(parent) = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(1))
            .and_then(|parent| parent.parse::<i32>().ok())
        else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        if parent <= 1 {
            return false;
        }
        pid = parent;
    }
    false
}

/// Turn the loop for `duration`, spending all of it.
///
/// POSTIO-FIXED-DEADLINE: gives a process that should not start, and a
/// render that should not happen, every chance to.
async fn spend(duration: Duration) {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub fn ten_conversations_start_no_web_process_and_hold_what_one_holds() {
    crate::gtk_case(async {
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

        let database = test_support::memory().await;
        let report = seed_small_with_bodies(&database, 29).await;
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let _ = report;

        let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
        let (sink, _events) = event_channel();
        let wiring = Wiring::new(
            database.clone(),
            blobs,
            bridge.handle(),
            sink,
            bridge.commands(),
        );
        let window = Window::default();
        window.set_default_size(1200, 800);
        window.present();
        while glib::MainContext::default().iteration(false) {}
        let _wired = feed_the_window(&window, &wiring)
            .await
            .expect("the store has an account");
        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() >= 11).await,
            "the seed needs eleven conversations to move through"
        );

        // The reader on screen and the snapshot it shows.
        let on_screen = || {
            window
                .conversation()
                .document_reader()
                .filter(|reader| reader.widget().is_mapped())
                .unwrap_or_else(|| window.reader())
        };
        let generation = || on_screen().view().document().map(|d| d.generation);
        let settle_on_new = async |before: Option<u64>| {
            assert!(
                settle_until(async || generation().is_some() && generation() != before).await,
                "the move never drew a new conversation"
            );
            // Anything a move does late -- a body, a refill -- has its turn.
            spend(Duration::from_millis(300)).await;
        };

        // ── the first conversation, and the baseline ─────────────────────
        list.first_row();
        settle_on_new(None).await;
        // The composer's warm-up is the one WebKit process this app starts
        // unasked; it lands on an idle turn, so wait it out before counting.
        spend(Duration::from_secs(2)).await;
        let baseline = web_processes();
        // The control: the counter sees the composer's own process, so the
        // "none started" below is a count that could have been one.
        assert!(
            !baseline.is_empty(),
            "the composer's warm-up started no WebKit process this counter can \
             see, so a zero below would prove nothing"
        );
        let tiles_after_one = on_screen().view().tile_bytes();
        let live_after_one = postio_render::live_documents();

        // ── nine more, each one render ───────────────────────────────────
        for step in 2..=10 {
            let before = generation();
            let (renders, counts) = postio_ui::test_support::snapshot_counts();
            window.handle_key(gdk::Key::j, gdk::ModifierType::empty());
            settle_on_new(before).await;
            let (renders_after, counts_after) = postio_ui::test_support::snapshot_counts();
            assert_eq!(
                renders_after - renders,
                1,
                "conversation {step} took {} renders",
                renders_after - renders
            );
            assert!(
                counts_after.style_passes - counts.style_passes <= 2,
                "conversation {step} took {} style passes",
                counts_after.style_passes - counts.style_passes
            );
        }

        let now = web_processes();
        assert!(
            now.iter().all(|pid| baseline.contains(pid)),
            "moving through conversations started WebKit processes: {now:?} \
             against {baseline:?} before"
        );
        assert_eq!(
            postio_render::live_documents(),
            live_after_one,
            "ten conversations left more snapshots alive than one"
        );
        let tiles = on_screen().view().tile_bytes();
        assert!(
            tiles <= tiles_after_one.max(1) * 2,
            "the tile cache grew with the conversations viewed: {tiles} bytes \
             against {tiles_after_one} after the first"
        );

        bridge.shutdown();
    });
}
