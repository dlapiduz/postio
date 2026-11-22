//! The suite types through GTK's own input method, never the compositor's
//! (#2155, #2192).
//!
//! Under `mutter --headless`, which offers text-input-v3, a focused field's
//! input method is GTK's Wayland one. GTK 4.22 keeps the context that has
//! the keyboard in a per-display global as a bare pointer, set on focus-in
//! and cleared on focus-out only when the global's text input is bound. A
//! field focused before that binding leaves the pointer behind when its
//! context is freed, and the compositor's next `enter` or `leave` reads the
//! freed context: three criticals (`gtk_widget_get_display`,
//! `g_object_get_data`, `gdk_wayland_display_get_wl_display`) and a
//! SIGSEGV. Each case is a fresh process that focuses a field early, so
//! the race came round on CI now and then, in whichever case it hit
//! (contacts' form, the add-account form, Settings' find field). No case
//! needs an input method, so the suite's run uses the simple one.

use gtk::prelude::*;

use crate::support;

pub fn a_focused_field_uses_gtks_simple_input_method() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let window = gtk::Window::new();
        let entry = gtk::Entry::new();
        window.set_child(Some(&entry));
        window.present();
        entry.grab_focus();
        crate::settle();
        let context = gtk::IMMulticontext::new();
        context.set_client_widget(Some(&entry));
        context.focus_in();
        let id = context.context_id();
        context.focus_out();
        context.set_client_widget(None::<&gtk::Widget>);
        window.destroy();
        assert_eq!(
            id.as_str(),
            "gtk-im-context-simple",
            "a field in this suite reaches the compositor's input method"
        );
    });
}
