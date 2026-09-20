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

use crate::pump;
use gtk::gdk;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::MessageBody;

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

/// Which marker the reader believes it is on.
///
/// **Was `view().uri()`'s fragment, and could not stay that way.** Scrolling
/// used to be `load_uri("postio-reader:///#pos-N")`, so the URI *was* the
/// bookkeeping; #1433 made it a scripted `scrollIntoView`, because a
/// fragment `load_uri` is a same-document scroll only while the URI still
/// matches the base -- and after `Reader::warm` empties it, the same call
/// became a real navigation to a scheme with no handler and put "The URL
/// can't be shown" in front of a reader.
///
/// So the URI no longer moves, deliberately, and a test that watched it hung
/// for two minutes waiting. What this file owns is unchanged and is asserted
/// here directly: the right marker for the right key, clamped, and reset when
/// a new message replaces the old one.
fn marker(window: &Window) -> u32 {
    window.reader().page_for_test()
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
        marker(&window),
        0,
        "a freshly rendered message starts with no fragment -- the top"
    );

    // -- Page_Down, the default binding -------------------------------------
    assert!(
        press(&window, gdk::Key::Page_Down),
        "Page_Down should be claimed, not passed through"
    );
    pump();
    // The fragment navigation is asynchronous; wait for it rather
    // than trusting the turn count above (#851, and this file again
    // on #187).
    crate::settle_until("the reader to reach pos-1", || marker(&window) == 1);
    assert_eq!(
        marker(&window),
        1,
        "one Page_Down should land on the first marker"
    );

    assert!(press(&window, gdk::Key::Page_Down));
    pump();
    // The fragment navigation is asynchronous; wait for it rather
    // than trusting the turn count above (#851, and this file again
    // on #187).
    crate::settle_until("the reader to reach pos-2", || marker(&window) == 2);
    assert_eq!(marker(&window), 2);

    // -- Page_Up walks it back ------------------------------------------
    assert!(
        press(&window, gdk::Key::Page_Up),
        "Page_Up should be claimed too"
    );
    pump();
    // The fragment navigation is asynchronous; wait for it rather
    // than trusting the turn count above (#851, and this file again
    // on #187).
    crate::settle_until("the reader to reach pos-1", || marker(&window) == 1);
    assert_eq!(marker(&window), 1);

    // -- the space/shift+space alternates do the same thing -----------------
    assert!(press(&window, gdk::Key::space));
    pump();
    // The fragment navigation is asynchronous; wait for it rather
    // than trusting the turn count above (#851, and this file again
    // on #187).
    crate::settle_until("the reader to reach pos-2", || marker(&window) == 2);
    assert_eq!(
        marker(&window),
        2,
        "space is the alternate binding for scrolling down"
    );
    assert!(press_shift(&window, gdk::Key::space));
    pump();
    // The fragment navigation is asynchronous; wait for it rather
    // than trusting the turn count above (#851, and this file again
    // on #187).
    crate::settle_until("the reader to reach pos-1", || marker(&window) == 1);
    assert_eq!(
        marker(&window),
        1,
        "shift+space is the alternate binding for scrolling up"
    );

    // -- Page_Up cannot go past the top --------------------------------
    assert!(press(&window, gdk::Key::Page_Up));
    pump();
    // The fragment navigation is asynchronous; wait for it rather
    // than trusting the turn count above (#851, and this file again
    // on #187).
    crate::settle_until("the reader to reach pos-0", || marker(&window) == 0);
    assert_eq!(marker(&window), 0);
    assert!(
        press(&window, gdk::Key::Page_Up),
        "still claimed at the top -- it is this command's key either way"
    );
    pump();
    // The fragment navigation is asynchronous; wait for it rather
    // than trusting the turn count above (#851, and this file again
    // on #187).
    crate::settle_until("the reader to reach pos-0", || marker(&window) == 0);
    assert_eq!(
        marker(&window),
        0,
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
    // The fragment navigation is asynchronous; wait for it rather
    // than trusting the turn count above (#851, and this file again
    // on #187).
    crate::settle_until("the reader to reach pos-2", || marker(&window) == 2);
    assert_eq!(
        marker(&window),
        2,
        "two presses down before the message changes"
    );

    // A second message opening is a `load_html`, which always starts a
    // document at the top -- the counter has to agree, or the next
    // Page_Down would jump to `pos-3` on a page that just reset to zero.
    window.show_message(&body(), Some("grace@example.com"));
    pump();
    assert_eq!(
        marker(&window),
        0,
        "the new message starts with no fragment, same as any fresh render"
    );

    assert!(press(&window, gdk::Key::Page_Down));
    // A condition, not a count. `show_message` is a `load_html`, which is
    // asynchronous: the forty pump rounds above are enough on an idle
    // workstation and were not enough on a loaded runner, where this read
    // `None` because the fragment navigation had not landed yet. Waiting for
    // the thing being asserted removes the guess -- and a timeout now says
    // what it was waiting for instead of failing an equality (#851).
    crate::settle_until("the new message's first page marker", || {
        marker(&window) == 1
    });
    assert_eq!(
        marker(&window),
        1,
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
        marker(&window),
        0,
        "nothing is open, so paging must not have navigated anywhere"
    );
}
