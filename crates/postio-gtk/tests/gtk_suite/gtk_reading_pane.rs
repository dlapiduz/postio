//! The reading pane holds two things, and never both at once.
//!
//! `postio-y39y`: nothing mounted a [`Reader`] into `shell().reader()`, so the
//! running application could list mail and not read it — the pane was only
//! ever filled by the composer taking it over. This is the window half: the
//! reader is mounted, it shows a message, and it gets out of the way when the
//! composer wants the pane.
//!
//! Skips without a display. Nothing here touches the network.
//!
//! # What this deliberately does not re-assert
//!
//! Remote-image *sanitizing*. [`Window::show_message`] is a call to
//! `Reader::render` and there is no second way in, so the sanitizing, the
//! banner and the per-sender allow list are exactly what `gtk_reader.rs`
//! already proves them to be against the `.eml` corpus. Repeating those
//! assertions here would mean waiting on a real `WebKitWebView` load —
//! `gtk_reader.rs` allows it five seconds — to re-test somebody else's
//! subject. If a bypass is ever introduced it will be by growing a second
//! path into the pane, which is what the mounting assertions below would
//! catch.
//!
//! What *is* new below is the count that sanitizing produces reaching the
//! parts panel: `postio-m2ex`'s `Window::reader()` wires
//! `Reader::connect_rendered` to `PartsPanel::set_held_back`, and that one
//! line of glue has no other test — `gtk_reader.rs` drives a bare `Reader`
//! with no `Window` around it, and `gtk_parts.rs` drives a bare `PartsPanel`
//! with no `Reader` around it.
//!
//! # Why this points the window at a scratch allow list
//!
//! #215: it did not, and so the count it asserts on was whatever the
//! developer's own `$XDG_STATE_HOME/postio/remote-images.ini` said. A machine
//! where the test's sender had a standing "always allow" rendered the body
//! with its remote images *permitted*, held nothing back, and failed here
//! forever — on every commit and every branch, because the cause was not in
//! the tree. A test that asserts on blocking must own the list that decides
//! what is blocked.
//!
//! One test function, for the reason `gtk_style.rs` gives.

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use postio_gtk::reader::RemoteImageAllowList;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::AccountId;
use postio_model::{Draft, MessageBody};

pub fn the_reading_pane_shows_a_message_and_yields_it_to_the_composer() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    // Before anything asks for the reader, which is when it is built and when
    // it reads this file. Empty, so the sender below is blocked because this
    // test says so and not because of what is on the machine.
    let allowlist = scratch_allowlist();
    window.set_allowlist_path(&allowlist);
    window.present();
    pump();

    // ── nothing to read yet ──────────────────────────────────────────────
    assert!(
        !window.reading(),
        "an empty pane shows its empty state, not a blank reader"
    );

    // ── a message fills it ───────────────────────────────────────────────
    window.show_message(&body(), Some("ada@example.com"));
    pump();

    assert!(window.reading(), "opening a message fills the reading pane");
    assert!(
        window.reader().widget().is_visible(),
        "and the reader is the widget actually on screen"
    );
    assert!(
        in_the_reading_pane(&window, &window.reader().widget()),
        "mounted into the pane the PLATE layout gives the reader, not \
         somewhere of its own"
    );

    // ── the parts panel hears how much the reader is holding back ───────
    //
    // `postio-m2ex`: `Reader::connect_rendered` fires with the render's
    // blocked-reference count, and `Window::reader()` is supposed to wire
    // that straight to `PartsPanel::set_held_back`. This is the one seam
    // `gtk_reader.rs` cannot prove on its own -- it drives a bare `Reader`,
    // never a `Window` -- so a build that forgot to connect the two would
    // pass every other test in this file and in that one.
    window.open_parts("multipart/mixed", &[]);
    pump();
    assert!(
        blocked_tag(&window).is_visible(),
        "the reader held a remote image back and the panel never heard about it"
    );

    // And it *follows* the reader rather than latching on the first render.
    // Allow-listing the sender re-renders with nothing held back, and a badge
    // still claiming otherwise would be exactly the stale one the wiring's own
    // comment says this connection exists to prevent.
    window.reader().click_always_allow();
    pump();
    assert!(
        !blocked_tag(&window).is_visible(),
        "the sender is allow-listed now, so nothing is held back — and the \
         panel has to hear about that render too, not just the first"
    );

    // #215's other half: the exception went to the file this test named, so a
    // window under test never edits the developer's own standing allow list.
    assert!(
        RemoteImageAllowList::load_from(&allowlist).is_allowed("ada@example.com"),
        "the window's reader should persist to the path it was given"
    );

    window.close_parts();
    pump();

    // ── the composer takes the pane, and the reader gets out of the way ──
    window.composer().open(Draft::new(AccountId::new(1)));
    pump();

    assert!(
        !window.reading(),
        "a reply drawn on top of the message being replied to is the bug \
         this swap exists to prevent"
    );
    assert!(!window.reader().widget().is_visible());

    // ── and gives it back ────────────────────────────────────────────────
    window.composer().close();
    pump();

    assert!(
        window.reading(),
        "closing the composer puts the message back, rather than leaving the \
         pane empty until the user clicks something again"
    );
    assert!(window.reader().widget().is_visible());

    // ── clearing empties it ──────────────────────────────────────────────
    window.clear_reader();
    pump();

    assert!(!window.reading());
    assert!(!window.reader().widget().is_visible());

    window.destroy();
}

/// An allow-list file of this test's own, under the process's temp dir.
///
/// Never `$XDG_STATE_HOME` — see the module docs. Starts absent, which
/// [`RemoteImageAllowList::load_from`] reads as "nobody is allow-listed".
fn scratch_allowlist() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("postio-reading-pane-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory should be creatable");
    let path = dir.join("remote-images.ini");
    let _ = std::fs::remove_file(&path);
    path
}

/// A plain-text body with a remote image in its HTML, so the blocking has
/// something to block.
fn body() -> MessageBody {
    MessageBody {
        text: Some("The tide gate interlock proposal, for review.".to_string()),
        html: Some(
            "<p>The tide gate interlock proposal, for review.</p>\
             <img src=\"https://tracker.example.com/pixel.gif\">"
                .to_string(),
        ),
    }
}

/// Whether `widget` is a descendant of the pane the shell gives the reader.
fn in_the_reading_pane(window: &Window, widget: &gtk::Widget) -> bool {
    let pane = window.shell().reader();
    let mut node = Some(widget.clone());
    while let Some(current) = node {
        if current == *pane.upcast_ref::<gtk::Widget>() {
            return true;
        }
        node = current.parent();
    }
    false
}

/// The parts panel's "remote blocked" tag in the header.
fn blocked_tag(window: &Window) -> gtk::Widget {
    find(window.parts().upcast_ref::<gtk::Widget>(), &|widget| {
        widget.has_css_class("postio-parts-blocked")
    })
    .expect("the panel has a blocked tag")
}

/// Depth-first search of a widget tree.
fn find(widget: &gtk::Widget, wanted: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if wanted(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, wanted) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn pump() {
    let context = glib::MainContext::default();
    for _ in 0..40 {
        while context.iteration(false) {}
    }
}
