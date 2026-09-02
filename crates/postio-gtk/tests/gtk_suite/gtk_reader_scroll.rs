//! #438: a keyboard way to move through a message longer than one screen,
//! without moving the keyboard off the message list.
//!
//! Every key here goes in through [`Window::handle_key`], not by calling the
//! reader directly — the same reason `gtk_parts.rs` does, and the same bead
//! this suite keeps citing (`postio-14b`): a command that only works when
//! called directly proves nothing about whether a keystroke can reach it.
//!
//! What is asserted is [`webkit6::WebView::uri`]'s fragment, not a scroll
//! pixel. `WebKitWebView` implements no `GtkScrollable` and exposes no
//! scroll-position getter at all — confirmed against the installed
//! WebKitGTK's own introspection data while designing the fix, not assumed
//! — so there is no scroll position for a test on this side of the process
//! boundary to read regardless of JavaScript. That a same-document fragment
//! navigation actually moves `window.scrollY` was verified separately, with
//! a throwaway `WebView` built with JavaScript deliberately turned *on* for
//! measurement only; production `enable-javascript` never changes, on the
//! real reader or anywhere near it. What this file owns is the half that
//! measurement could not: the right fragment for the right key, in the
//! right context, clamped, and reset when a new message replaces the old
//! one.
//!
//! Skips without a display. Nothing here touches the network.

use gtk::gdk;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::MessageBody;
use webkit6::prelude::WebViewExt;

fn pump() {
    let context = glib::MainContext::default();
    for _ in 0..40 {
        while context.iteration(false) {}
    }
}

fn press(window: &Window, key: gdk::Key) -> bool {
    window.handle_key(key, gdk::ModifierType::empty()) == glib::Propagation::Stop
}

fn press_shift(window: &Window, key: gdk::Key) -> bool {
    window.handle_key(key, gdk::ModifierType::SHIFT_MASK) == glib::Propagation::Stop
}

fn body() -> MessageBody {
    MessageBody {
        text: Some(
            "A message long enough that scrolling it would mean something, \
                     were this test measuring pixels rather than the mechanism."
                .to_owned(),
        ),
        html: None,
    }
}

/// The fragment `reader.view().uri()` currently carries, or `None` for a
/// bare base URI with nothing after it.
fn fragment(window: &Window) -> Option<String> {
    let uri = window.reader().view().uri()?;
    uri.split_once('#').map(|(_, frag)| frag.to_owned())
}

pub fn page_down_and_page_up_move_a_marker_at_a_time() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    assert_eq!(
        window.context(),
        postio_core::Context::List,
        "the window starts in List, which is where this command has to work \
         -- reading a message never switches context away from it"
    );

    window.show_message(&body(), Some("ada@example.com"));
    pump();
    assert!(
        window.reading(),
        "a message should be open before paging it"
    );
    assert_eq!(
        fragment(&window),
        None,
        "a freshly rendered message starts with no fragment -- the top"
    );

    // -- Page_Down, the default binding -------------------------------------
    assert!(
        press(&window, gdk::Key::Page_Down),
        "Page_Down should be claimed, not passed through"
    );
    pump();
    assert_eq!(
        fragment(&window).as_deref(),
        Some("pos-1"),
        "one Page_Down should land on the first marker"
    );

    assert!(press(&window, gdk::Key::Page_Down));
    pump();
    assert_eq!(fragment(&window).as_deref(), Some("pos-2"));

    // -- Page_Up walks it back ------------------------------------------
    assert!(
        press(&window, gdk::Key::Page_Up),
        "Page_Up should be claimed too"
    );
    pump();
    assert_eq!(fragment(&window).as_deref(), Some("pos-1"));

    // -- the space/shift+space alternates do the same thing -----------------
    assert!(press(&window, gdk::Key::space));
    pump();
    assert_eq!(
        fragment(&window).as_deref(),
        Some("pos-2"),
        "space is the alternate binding for scrolling down"
    );
    assert!(press_shift(&window, gdk::Key::space));
    pump();
    assert_eq!(
        fragment(&window).as_deref(),
        Some("pos-1"),
        "shift+space is the alternate binding for scrolling up"
    );

    // -- Page_Up cannot go past the top --------------------------------
    assert!(press(&window, gdk::Key::Page_Up));
    pump();
    assert_eq!(fragment(&window).as_deref(), Some("pos-0"));
    assert!(
        press(&window, gdk::Key::Page_Up),
        "still claimed at the top -- it is this command's key either way"
    );
    pump();
    assert_eq!(
        fragment(&window).as_deref(),
        Some("pos-0"),
        "Page_Up at the top stays at the top rather than going negative"
    );

    // -- and the keyboard never left the list -------------------------------
    assert_eq!(
        window.context(),
        postio_core::Context::List,
        "paging the reader must not have moved the keyboard context"
    );
}

pub fn a_new_message_resets_the_scroll_position() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    window.show_message(&body(), Some("ada@example.com"));
    pump();
    assert!(press(&window, gdk::Key::Page_Down));
    assert!(press(&window, gdk::Key::Page_Down));
    pump();
    assert_eq!(
        fragment(&window).as_deref(),
        Some("pos-2"),
        "two presses down before the message changes"
    );

    // A second message opening is a `load_html`, which always starts a
    // document at the top -- the counter has to agree, or the next
    // Page_Down would jump to `pos-3` on a page that just reset to zero.
    window.show_message(&body(), Some("grace@example.com"));
    pump();
    assert_eq!(
        fragment(&window),
        None,
        "the new message starts with no fragment, same as any fresh render"
    );

    assert!(press(&window, gdk::Key::Page_Down));
    pump();
    assert_eq!(
        fragment(&window).as_deref(),
        Some("pos-1"),
        "paging the new message starts counting from zero again, not from \
         wherever the last one left off"
    );
}

pub fn paging_with_nothing_open_does_nothing() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    assert!(!window.reading(), "nothing should be open yet");
    // The command is still claimed -- it is bound in this context regardless
    // of whether there happens to be a message open right now, the same way
    // `j`/`k` are claimed with an empty list. What must not happen is a
    // navigation to a marker that means nothing.
    press(&window, gdk::Key::Page_Down);
    pump();
    assert_eq!(
        fragment(&window),
        None,
        "nothing is open, so paging must not have navigated anywhere"
    );
}
