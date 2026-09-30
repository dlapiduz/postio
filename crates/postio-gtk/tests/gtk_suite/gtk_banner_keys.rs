//! The reading pane's banners, as commands (`specs/005-tui-frontend` T044,
//! T048).
//!
//! "Show images" and "Unsubscribe" were buttons and nothing else: no key, no
//! palette entry, so a person without a pointer could not reach them, and a
//! frontend with no banners -- the terminal -- had nothing to bind. They are
//! registry commands now (`show_images` `i i`, `unsubscribe` `X`); the
//! window hands each to `Reader::run_banner_command`, and this is the proof
//! that it does what the button does, and nothing when the banner is not
//! there to offer it.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_core::CommandId;
use postio_gtk::reader::{BlobSource, Reader, RemoteImageAllowList};
use postio_model::test_corpus;

fn settle(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(20));
    while Instant::now() < deadline {
        crate::pump();
        if done() {
            return;
        }
    }
    panic!("timed out waiting for {what}");
}

pub fn the_banner_commands_do_what_their_buttons_do() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let window = gtk::Window::new();
    window.set_default_size(600, 500);
    let source: Rc<dyn BlobSource> = Rc::new(|_: &str| None);
    let allowlist = std::env::temp_dir()
        .join(format!("postio-banner-keys-{}", std::process::id()))
        .join("remote-images.ini");
    let reader = Reader::with_allowlist(source, RemoteImageAllowList::default(), allowlist);
    window.set_child(Some(&reader.widget()));
    window.present();
    crate::pump();

    // Nothing to offer yet: the commands do nothing.
    assert!(!reader.run_banner_command(CommandId::ShowImages));
    assert!(!reader.run_banner_command(CommandId::Unsubscribe));

    // ── show_images: the banner's "show once" ─────────────────────────────
    let tracking = test_corpus::load("html-tracking-pixel-remote-images");
    let parsed = postio_model::mime::parse(tracking.bytes());
    reader.render(&parsed.body, Some("orders@shop.example.org"));
    settle("the remote-image banner", || reader.banner_visible());
    assert!(reader.run_banner_command(CommandId::ShowImages));
    settle("the images to be shown", || !reader.banner_visible());

    // ── unsubscribe: the banner's button ────────────────────────────────────
    reader.set_unsubscribe(Some("newsletter.example.com"));
    let activated: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&activated);
    reader.connect_unsubscribe_activated(move |list| seen.borrow_mut().push(list.to_owned()));
    assert!(reader.run_banner_command(CommandId::Unsubscribe));
    assert_eq!(
        *activated.borrow(),
        vec!["newsletter.example.com".to_owned()]
    );

    window.destroy();
}
