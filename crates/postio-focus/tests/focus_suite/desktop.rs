//! What the desktop knows Focus by (T159): the name its windows give the
//! compositor, and the `postio://` links it opens.

/// GNOME matches a window to its desktop entry by the Wayland `app_id`,
/// which GDK takes from `g_get_prgname()` -- the *binary* name,
/// `postio-focus`, unless the application says otherwise. Focus's entry is
/// `dev.postio.Postio.Focus.desktop`, with `StartupWMClass` to match, so a
/// Focus that never set its name would be drawn with a generic icon under a
/// generic name. The classic app's `desktop_entry.rs` holds the same line.
pub fn focus_says_which_application_it_is() {
    let _application = postio_focus::app::application();
    let reported = gtk::glib::prgname();
    assert_eq!(
        reported.as_ref().map(|name| name.as_str()),
        Some(postio_focus::app::APP_ID),
        "Focus's windows would tell the compositor they are {reported:?}"
    );
}
