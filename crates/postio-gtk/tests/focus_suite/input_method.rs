//! The suite types through GTK's own input method, never the compositor's: GTK 4.22 can leave its
//! per-display context pointer
//! dangling when a field is focused before text-input-v3 binds, and the compositor's next enter or
//! leave crashes (#2155, #2192).

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
