//! One remote-image allow list per app, shared by all its readers
//! (specs/007-postio-focus T020, research R2).
//!
//! The classic window builds a reader for the reading pane and one for each
//! message the conversation pane expands, and each loaded its own copy of the
//! allow list. So "Always allow" in one reader was news to every other: the
//! next conversation drawn by a reader built earlier asked again about a
//! sender the person had just allowed, and a revoke in settings reached no
//! reader already built. What is asserted is the banner a person sees.
//!
//! Skips without a display. Nothing here touches the network: the remote
//! image is at a reserved host and nothing fetches it.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::MessageBody;

use crate::pump;

const SENDER: &str = "ada@example.com";

fn a_window() -> Option<(Window, tempfile::TempDir)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    let dir = tempfile::tempdir().expect("a scratch directory");
    let window = Window::default();
    window.set_allowlist_path(&dir.path().join("remote-images.ini"));
    window.present();
    pump();
    Some((window, dir))
}

/// A message from `SENDER` with one remote image in it.
fn with_an_image() -> MessageBody {
    MessageBody {
        text: None,
        html: Some(
            "<p>The survey map is below.</p>\
             <img src=\"https://images.invalid/map.png\" alt=\"map\">"
                .to_owned(),
        ),
    }
}

pub fn two_readers_in_one_app_see_one_always_allow() {
    let Some((window, _dir)) = a_window() else {
        return;
    };
    let pane = window.reader();
    // Another reader the window builds, mounted the way the conversation
    // pane mounts the ones it asks for.
    let expanded = window.new_reader();
    let elsewhere = gtk::Window::new();
    elsewhere.set_default_size(800, 600);
    elsewhere.set_child(Some(&expanded.widget()));
    elsewhere.present();
    window.show_message(&with_an_image(), Some(SENDER));
    expanded.render(&with_an_image(), Some(SENDER));
    pump();
    assert!(pane.banner_visible(), "the image was not held back");
    assert!(expanded.banner_visible(), "the image was not held back");

    pane.click_always_allow();
    pump();
    assert!(
        !pane.banner_visible(),
        "always allowing the sender did not reach the reader it was chosen in"
    );

    expanded.render(&with_an_image(), Some(SENDER));
    pump();
    assert!(
        !expanded.banner_visible(),
        "the other reader in the same window still asks about a sender the \
         person has just always allowed"
    );
    elsewhere.destroy();
    window.destroy();
}

pub fn a_revoke_in_settings_reaches_the_apps_readers() {
    let Some((window, _dir)) = a_window() else {
        return;
    };
    let pane = window.reader();
    window.show_message(&with_an_image(), Some(SENDER));
    pump();
    assert!(pane.banner_visible(), "the image was not held back");
    pane.click_always_allow();
    pump();
    assert!(!pane.banner_visible(), "the sender was not always allowed");

    window.open_settings();
    pump();
    let revoke = revoke_button_for(&window.settings().upcast(), SENDER)
        .expect("the privacy pane lists the always-allowed sender");
    revoke.emit_clicked();
    pump();

    window.show_message(&with_an_image(), Some(SENDER));
    pump();
    assert!(
        pane.banner_visible(),
        "the sender's exception was revoked in settings, and the reader still \
         shows their remote images"
    );
    window.destroy();
}

/// The revoke button on `sender`'s row of the privacy pane under `root`.
fn revoke_button_for(root: &gtk::Widget, sender: &str) -> Option<gtk::Button> {
    let rows = with_class(root, "postio-settings-privacy-row");
    rows.into_iter().find_map(|row| {
        let names = with_class(&row, "postio-settings-privacy-sender");
        let named = names.iter().any(|label| {
            label
                .downcast_ref::<gtk::Label>()
                .is_some_and(|label| label.text() == sender)
        });
        named
            .then(|| with_class(&row, "postio-settings-privacy-revoke"))
            .and_then(|buttons| buttons.into_iter().find_map(|w| w.downcast().ok()))
    })
}

fn with_class(root: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if root.has_css_class(class) {
        found.push(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        found.extend(with_class(&current, class));
        child = current.next_sibling();
    }
    found
}
