//! The reader's zoom outlives the reader (spec 006 FR-021, T130): a zoom a
//! person chose is written to `[reader] zoom`, the next reader starts at
//! it, and editing the file changes the reader that is open.
//!
//! The same shape as `storage_ceiling_wiring`: a window with its
//! configuration installed from a file this case owns, so both directions
//! -- the app writing the file, the file reaching the app -- run through
//! the real watcher.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::MessageBody;

pub fn a_zoom_is_saved_and_a_live_edit_applies() {
    crate::gtk_case(async {
        if adw::init().is_err() || gdk::Display::default().is_none() {
            eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
            return;
        }
        let display = gdk::Display::default().unwrap();
        fonts::install().expect("the embedded fonts should install");
        style::install(&display);
        app::install_icons(&display);

        let root =
            std::env::temp_dir().join(format!("postio-zoom-persists-{}", std::process::id()));
        let config_dir = root.join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        let path = config_dir.join("config.toml");
        std::fs::write(&path, "").unwrap();

        let window = Window::default();
        postio_gtk::config::install_at(&window, &path);
        window.present();
        while glib::MainContext::default().iteration(false) {}
        window.show_message(
            &MessageBody {
                text: Some("A message to read at a comfortable size.".to_owned()),
                html: None,
            },
            Some("ada@example.com"),
        );
        while glib::MainContext::default().iteration(false) {}
        assert_eq!(
            window.reader().zoom(),
            100,
            "an empty config is actual size"
        );

        // ── a zoom a person chose is written to the file ─────────────────
        assert_eq!(
            window.handle_key(gdk::Key::plus, gdk::ModifierType::CONTROL_MASK),
            glib::Propagation::Stop,
            "mod+plus is claimed"
        );
        assert!(
            settle_until(async || std::fs::read_to_string(&path)
                .is_ok_and(|text| text.contains("[reader]") && text.contains("zoom = 110")))
            .await,
            "zooming in was never written to config.toml: {:?}",
            std::fs::read_to_string(&path)
        );

        // ── the next reader starts at it ─────────────────────────────────
        assert!(
            settle_until(async || window.new_reader().zoom() == 110).await,
            "a new reader did not start at the saved zoom"
        );

        // ── and a live edit reaches the reader that is open ──────────────
        std::fs::write(&path, "[reader]\nzoom = 150\n").unwrap();
        assert!(
            settle_until(async || window.reader().zoom() == 150).await,
            "editing [reader] zoom never reached the open reader"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[reader]\nzoom = 150\n",
            "applying the file's zoom wrote it back"
        );

        window.destroy();
        let _ = std::fs::remove_dir_all(&root);
    });
}
