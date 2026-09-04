//! The shared controls on a real display (#1002).
//!
//! What is proven here is the part that needs a widget tree: that a cap
//! actually appears and disappears, that a notice stays one line under a
//! sentence far too long for it, and that an overflow entry runs the handler
//! it was built with. The rules these draw are proven without a display, in
//! `postio_ui::conversation`.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_gtk::widgets::{Action, ActionBar, KeycapButton, NoticeBar, NoticeMenuItem};

/// Whether there is a display to build widgets on. The suite's own
/// convention: each case guards, so a headless box skips rather than fails.
fn ready() -> bool {
    adw::init().is_ok() && gtk::gdk::Display::default().is_some()
}

/// A button says which key runs it, and says nothing when no key does.
pub fn a_keycap_shows_the_key_or_nothing_at_all() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }

    let button = Rc::new(KeycapButton::new(None, "Try again", "probe-retry", true));
    KeycapButton::arm(&button);
    assert_eq!(
        button.key(),
        "",
        "a button given no key shows no cap, rather than an empty one"
    );

    button.set_key(Some("Ret"));
    assert_eq!(button.key(), "Ret");

    // The word changes; the key that runs it does not. `set_busy` on the
    // unavailable screen is exactly this call, and the hand-rolled version it
    // replaced did it by swapping the button's whole child — which took the
    // cap with it.
    button.set_label("Trying…");
    assert_eq!(button.label(), "Trying…");
    assert_eq!(button.key(), "Ret", "renaming the verb kept its key");

    button.set_key(None);
    assert_eq!(
        button.key(),
        "",
        "a cleared binding hides the cap rather than showing a blank one"
    );

    let pressed = Rc::new(Cell::new(0));
    let counter = pressed.clone();
    button.connect_clicked(move || counter.set(counter.get() + 1));
    button.press();
    assert_eq!(pressed.get(), 1);
}

/// The four verbs a bar carries, and the keys it advertises for them.
const PROBE: [Action; 2] = [
    Action::new(CommandId::Reply, "Reply", "probe-reply").primary(),
    Action::new(CommandId::Archive, "Archive", "probe-archive"),
];

/// A bar runs the command its cap advertises, and re-caps on a rebind.
pub fn an_action_bar_dispatches_the_command_its_cap_advertises() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }

    let bar = ActionBar::new(&PROBE, "probe-bar");
    assert_eq!(
        bar.button(CommandId::Reply)
            .expect("Reply is in the bar")
            .key(),
        "e",
        "a bar caps itself from the registry defaults the moment it exists"
    );

    let ran: Rc<Cell<Option<CommandId>>> = Rc::new(Cell::new(None));
    let seen = ran.clone();
    bar.connect_command(move |command| seen.set(Some(command.id())));
    bar.press(CommandId::Archive);
    assert_eq!(
        ran.get(),
        Some(CommandId::Archive),
        "the button runs the same command the keyboard's binding would"
    );

    let mut overrides = postio_config::KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("reply".to_string(), "r".to_string());
    bar.set_keymap(&Keymap::resolve(&overrides));
    assert_eq!(
        bar.button(CommandId::Reply).expect("still there").key(),
        "r",
        "a rebind reaches the cap, not only the keyboard"
    );
}

/// A notice is one line at any width, however much it is asked to say.
///
/// The measurement is a comparison rather than a pixel count: two notices at
/// the same width, one saying four words and one saying a paragraph. If the
/// long one is taller, it wrapped — which is the bug the canvas's turn-7
/// note is about, where an Apple relay address spelled out inline grew the
/// remote-image banner to three lines and pushed the mail down the pane.
pub fn a_notice_never_wraps_however_long_the_sentence() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }

    // Narrower than the canvas's narrowest reading pane.
    const WIDTH: i32 = 320;

    let short = NoticeBar::new("image-missing-symbolic", "probe-notice");
    short.set_visible(true);
    short.set_text("Images blocked");
    short.set_action(Some("Show images"));
    short.set_action_key(Some("H"));

    let long = NoticeBar::new("image-missing-symbolic", "probe-notice");
    long.set_visible(true);
    long.set_text(
        "14 remote images and 1 tracker blocked, from a sender whose address \
         is long enough that the old banner wrapped to three lines and pushed \
         the mail down the pane",
    );
    long.set_action(Some("Show images"));
    long.set_action_key(Some("H"));

    let window = gtk::Window::new();
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.append(&short.widget());
    holder.append(&long.widget());
    window.set_child(Some(&holder));
    window.set_default_size(WIDTH, 200);
    window.present();
    crate::pump();

    let (_, tall_short, _, _) = short.widget().measure(gtk::Orientation::Vertical, WIDTH);
    let (_, tall_long, _, _) = long.widget().measure(gtk::Orientation::Vertical, WIDTH);
    assert!(tall_short > 0, "the notice has to be on screen to measure");
    assert_eq!(
        tall_long, tall_short,
        "a notice is one line whatever it says: {tall_long}px against \
         {tall_short}px for four words"
    );
    assert_eq!(long.action().key(), "H");

    window.close();
}

/// The overflow runs what it names, and replaces rather than appends.
pub fn a_notice_overflow_replaces_rather_than_appends() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }

    let notice = NoticeBar::new("image-missing-symbolic", "probe-notice");

    let allowed = Rc::new(Cell::new(0));
    let count = allowed.clone();
    notice.set_menu(vec![NoticeMenuItem::new(
        "Always allow transaction_at_example@relay.example.com",
        move || count.set(count.get() + 1),
    )]);
    assert_eq!(notice.menu_labels().len(), 1);
    notice.press_menu_item(0);
    assert_eq!(allowed.get(), 1);

    // A notice describes the message it is reporting on. Leaving the last
    // message's entries behind would offer to always-allow the wrong sender.
    notice.set_menu(vec![NoticeMenuItem::new("Always allow this domain", || {})]);
    assert_eq!(
        notice.menu_labels(),
        vec!["Always allow this domain".to_string()],
        "the overflow was replaced, not appended to"
    );

    notice.set_menu(Vec::new());
    assert!(
        notice.menu_labels().is_empty(),
        "a notice with nothing to offer has no overflow at all"
    );
}

/// Acting on a notice may be what makes the notice wrong, and that must not
/// panic.
///
/// The reader's "always allow" is this shape: choosing it allow-lists the
/// sender, which clears the sender, which rebuilds this very menu — all
/// while the handler that started it is still on the stack. The first
/// version held the handler list borrowed across the call and `gtk_reader`
/// died on the way back in.
pub fn a_notice_survives_an_overflow_entry_that_rebuilds_the_menu() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }

    let notice = NoticeBar::new("image-missing-symbolic", "probe-notice");
    let inner = Rc::new(std::cell::RefCell::new(None::<Rc<NoticeBar>>));
    *inner.borrow_mut() = Some(notice.clone());

    let reentered = Rc::new(Cell::new(false));
    let flag = reentered.clone();
    let target = inner.clone();
    notice.set_menu(vec![NoticeMenuItem::new(
        "Always allow this sender",
        move || {
            flag.set(true);
            // What the reader does next: the sender is gone, so the menu that
            // named it is rebuilt out from under the handler running now.
            if let Some(notice) = target.borrow().as_ref() {
                notice.set_menu(Vec::new());
            }
        },
    )]);

    notice.press_menu_item(0);
    assert!(reentered.get(), "the entry ran");
    assert!(
        notice.menu_labels().is_empty(),
        "and its own rebuild took effect rather than being lost"
    );
}

/// The remote-image notice says what it blocked and keeps its long action in
/// the overflow (#1008).
pub fn the_blocked_images_notice_counts_and_elides() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }

    let banner = postio_gtk::reader::banner::RemoteImageBanner::new();
    banner.set_held_back(postio_ui::reader::document::HeldBack {
        remote_images: 14,
        trackers: 1,
    });
    assert_eq!(
        banner.text(),
        "14 remote images and 1 tracker blocked",
        "the canvas's own wording: a picture and a beacon are different claims"
    );

    // A relay address is the case that made the old banner three lines tall.
    banner.set_sender(Some(
        "transaction_at_shop_12345@privaterelay.appleid.example",
    ));
    let menu = banner.menu_labels();
    assert_eq!(
        menu.len(),
        2,
        "the address and its domain are different promises: {menu:?}"
    );
    assert!(menu[0].starts_with("Always allow transaction"), "{menu:?}");
    assert!(
        menu[0].contains('…'),
        "the address is elided rather than spelled out: {menu:?}"
    );
    assert!(
        menu[0].len() < 60,
        "and the entry stays a menu entry: {menu:?}"
    );
    assert_eq!(menu[1], "Always allow privaterelay.appleid.example");

    // Nothing to name, nothing to offer.
    banner.set_sender(None);
    assert!(banner.menu_labels().is_empty());
}
