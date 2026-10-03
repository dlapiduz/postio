//! Zoom in the open message (T242, row 28; spec 006 FR-021): `mod+plus`,
//! `mod+minus` and `mod+0` in the dialog and in the pane, kept in
//! `[reader] zoom` and applied to the next message. The classic app's
//! `zoom_persists`, ported.

use gtk::gdk::ModifierType;

use crate::support::{self, Fixture};

const CONTROL: ModifierType = ModifierType::CONTROL_MASK;

pub fn the_zoom_keys_work_in_the_dialog_and_the_pane_and_the_level_is_kept() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        for pane in [false, true] {
            let fixture = Fixture::empty().await;
            for (minutes, subject) in [(10, "First"), (20, "Second")] {
                let (message, _) = fixture
                    .file(("Lena Park", "lena@example.org"), subject, "x", minutes)
                    .await;
                fixture.write_body(message, "A body to read.").await;
            }
            let (window, _directory, path) =
                crate::settings::open_under(&fixture, "[reader]\nzoom = 100\n").await;
            if pane {
                window.set_focus_config(postio_config::FocusConfig {
                    reading: postio_config::Reading::Pane,
                    ..postio_config::FocusConfig::default()
                });
                crate::settle();
            }
            support::keys(&window, &["j"]);
            support::press(&window, "Return", ModifierType::empty());
            let reading = window.reading().expect("open");
            let zoom = || reading.reader().zoom();
            assert!(
                crate::settle_until(async || reading.body_text().contains("A body")).await,
                "the body never arrived (pane: {pane})"
            );

            support::press(&window, "plus", CONTROL);
            assert!(
                crate::settle_until(async || zoom() == 110).await,
                "mod+plus: {} (pane: {pane})",
                zoom()
            );
            // A zoom a person chose is written to the file, and only it.
            assert!(
                crate::settle_until(async || std::fs::read_to_string(&path)
                    .is_ok_and(|text| text.contains("zoom = 110")))
                .await,
                "zooming in was never written to config.toml: {:?} (pane: {pane})",
                std::fs::read_to_string(&path)
            );
            // ... and is the next message's too.
            support::keys(&window, &["j"]);
            assert!(
                crate::settle_until(async || reading.title() == "Second").await,
                "j did not step (pane: {pane})"
            );
            assert_eq!(zoom(), 110, "the next message lost the zoom (pane: {pane})");

            support::press(&window, "plus", CONTROL);
            support::press(&window, "minus", CONTROL);
            assert!(
                crate::settle_until(async || zoom() == 110).await,
                "mod+plus then mod+minus: {} (pane: {pane})",
                zoom()
            );
            support::press(&window, "minus", CONTROL);
            assert_eq!(zoom(), 100, "mod+minus (pane: {pane})");
            support::press(&window, "plus", CONTROL);
            support::press(&window, "plus", CONTROL);
            support::press(&window, "0", CONTROL);
            assert_eq!(zoom(), 100, "mod+0 is actual size (pane: {pane})");
            assert!(
                crate::settle_until(async || std::fs::read_to_string(&path)
                    .is_ok_and(|text| text.contains("zoom = 100")))
                .await,
                "the reset was never written (pane: {pane}): {:?}",
                std::fs::read_to_string(&path)
            );
        }
    });
}
