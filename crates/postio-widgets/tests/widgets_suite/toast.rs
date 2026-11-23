use gtk::glib::object::ObjectExt;
use postio_widgets::widgets::toast::Toast;

fn ready() -> bool {
    adw::init().is_ok() && gtk::gdk::Display::default().is_some()
}

pub fn the_undo_toast_coalesces_and_offers_undo_only_when_there_is_something_to_undo() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }

    let toast = Toast::new();
    toast.show_action_completed("Archived 3 messages", true);
    let first = toast.showing();
    assert!(first.is_some());

    toast.show_action_completed("Archived 5 messages", true);
    let second = toast.showing();
    assert!(second.is_some());
    assert_ne!(
        first.unwrap(),
        second.unwrap(),
        "coalescing swaps in a new toast rather than editing the old one in place"
    );

    let toast = Toast::new();
    toast.show_action_completed("Archived 12 messages", true);
    let current = toast.showing().unwrap();
    assert_eq!(current.button_label().as_deref(), Some("Undo"));
    assert_eq!(current.action_name().as_deref(), Some("win.undo"));

    let toast = Toast::new();
    toast.show_action_completed("Marked 1 message as read", false);
    let current = toast.showing().unwrap();
    assert_eq!(current.button_label(), None);
    assert_eq!(current.action_name(), None);

    let toast = Toast::new();
    let called = std::rc::Rc::new(std::cell::Cell::new(false));
    toast.show_removable("Account removed", {
        let called = called.clone();
        move || called.set(true)
    });
    let current = toast.showing().unwrap();
    assert_eq!(current.button_label().as_deref(), Some("Undo"));
    assert_eq!(
        current.action_name(),
        None,
        "a removable toast's button must not also carry win.undo"
    );
    current.emit_by_name::<()>("button-clicked", &[]);
    assert!(called.get(), "the toast's own button did not call back");
}
