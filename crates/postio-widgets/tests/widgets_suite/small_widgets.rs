use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_widgets::widgets::{Action, ActionBar, KeycapButton, NoticeBar, NoticeMenuItem};

fn ready() -> bool {
    adw::init().is_ok() && gtk::gdk::Display::default().is_some()
}

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

const PROBE: [Action; 2] = [
    Action::new(CommandId::Reply, "Reply", "probe-reply").primary(),
    Action::new(CommandId::Archive, "Archive", "probe-archive"),
];

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
        "r",
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
        .insert("reply".to_string(), "w".to_string());
    bar.set_keymap(&Keymap::resolve(&overrides));
    assert_eq!(
        bar.button(CommandId::Reply).expect("still there").key(),
        "w",
        "a rebind reaches the cap, not only the keyboard"
    );
}

/// Compared by height: two notices at one width, four words against a paragraph; the long one being
/// taller means it wrapped.
pub fn a_notice_never_wraps_however_long_the_sentence() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }

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
    crate::support_reader::pump();

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

/// "Always allow" allow-lists the sender, which rebuilds this very menu while the handler is still
/// on the stack; holding the handler list borrowed across the call panicked.
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

pub fn the_blocked_images_notice_counts_and_elides() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }

    let banner = postio_widgets::reader::banner::RemoteImageBanner::new();
    banner.set_held_back(postio_ui::reader::document::HeldBack {
        remote_images: 14,
        trackers: 1,
    });
    assert_eq!(
        banner.text(),
        "14 remote images and 1 tracker blocked",
        "the canvas's own wording: a picture and a beacon are different claims"
    );

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

    banner.set_sender(None);
    assert!(banner.menu_labels().is_empty());
}
