//! The header bar, per canvas 1b.
//!
//! Left to right: the sidebar toggle, a search field carrying its own `/`
//! hint, then Keys `?`, Compose `c` and the main menu — inside real Adwaita
//! chrome, with the compositor's own window controls where GNOME puts them.
//!
//! Every control here shows the key that reaches it. That is the whole
//! argument of the design: the mouse is a discoverability affordance for the
//! keyboard, so a button that hides its shortcut is a button that teaches
//! nothing. The bindings shown are the canvas' — `c` compose, `?` keys, `/`
//! search — and become overridable when the keymap lands (E6).
//!
//! Compose is wired to the `win.compose` action the composer installs; the
//! rest of the actions arrive with their own beads.

use adw::prelude::*;

use crate::finder;

/// How wide the search field is allowed to get, from the canvas.
pub const SEARCH_MAX_WIDTH: i32 = 600;

/// The `max-width-chars` that gets the field to [`SEARCH_MAX_WIDTH`].
///
/// GTK has no `max-width`. What it has is the natural size: a widget that
/// does not expand is allocated its natural width when there is room and the
/// available width when there is not, which is exactly max-width semantics.
/// `GtkText` takes its natural width from this, in characters — so the number
/// is a font metric, and `gtk_shell.rs` asserts the field it produces is still
/// the canvas' 600px. Retuning the type will fail that test rather than
/// quietly widening the header.
const SEARCH_WIDTH_CHARS: i32 = 76;

/// The header bar and the controls other code needs to reach.
pub struct Header {
    /// The bar itself, ready to be a toolbar view's top bar.
    pub bar: adw::HeaderBar,
    /// Shows and hides the sidebar.
    pub sidebar_toggle: gtk::ToggleButton,
    /// The one box: search mail, run a command, jump to a folder.
    ///
    /// Canvas 1b draws it at rest and 2b draws it active; they are the same
    /// field. [`crate::finder::Finder`] drives it.
    pub search: finder::Field,
    /// `Compose c`, wired to the `win.compose` action.
    pub compose: gtk::Button,
}

/// Build the header bar.
pub fn build() -> Header {
    let bar = adw::HeaderBar::new();
    bar.add_css_class("postio-header");

    let sidebar_toggle = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Toggle sidebar")
        .active(true)
        .build();
    sidebar_toggle.add_css_class("flat");
    sidebar_toggle.update_property(&[gtk::accessible::Property::Label("Toggle sidebar")]);
    bar.pack_start(&sidebar_toggle);

    // Packed at the start, flush against the toggle, exactly as the canvas
    // draws it. The start box hands a child its natural width, which is what
    // makes the cap below work.
    let (field, search) = search_field();
    bar.pack_start(&field);

    // No window title: a centred "Postio" would be a third thing saying where
    // you are, after the sidebar's account line and the folder heading over
    // the list. Emptying the slot also stops the header reserving space to
    // centre something in.
    bar.set_title_widget(Some(&gtk::Label::new(None)));

    // Packed in reverse: pack_end works outwards from the window controls.
    let compose = compose_button();
    bar.pack_end(&menu_button());
    bar.pack_end(&compose);
    bar.pack_end(&keys_button());

    Header {
        bar,
        sidebar_toggle,
        search,
        compose,
    }
}

/// The one box: accent magnifier, mode marker, chips, text, and the `/` cap.
///
/// Built out of a `GtkText` in a styled box rather than a `GtkSearchEntry`,
/// because the canvas puts a key hint and the query's chips inside the field
/// and an entry has room for neither.
fn search_field() -> (gtk::Widget, finder::Field) {
    let icon = gtk::Image::from_icon_name("system-search-symbolic");
    icon.add_css_class("postio-search-icon");

    // Shown in place of the magnifier once a prefix has chosen a mode, so
    // which question the box is asking is visible without reading the text.
    let marker = gtk::Label::new(None);
    marker.add_css_class("postio-search-marker");
    marker.set_visible(false);
    marker.set_accessible_role(gtk::AccessibleRole::Presentation);

    let text = gtk::Text::builder()
        .placeholder_text("Search all mail")
        .width_chars(8)
        .max_width_chars(SEARCH_WIDTH_CHARS)
        .hexpand(true)
        .build();
    text.update_property(&[gtk::accessible::Property::Label("Search all mail")]);

    let hint = gtk::Label::new(Some("/"));
    hint.add_css_class("postio-key");
    // The hint is decoration for the field's own label; announcing it would
    // read as a stray slash.
    hint.set_accessible_role(gtk::AccessibleRole::Presentation);

    // `space-2` from the design system, rounded to the pixel GTK works in.
    let frame = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    frame.add_css_class("postio-search");
    frame.append(&icon);
    frame.append(&marker);
    frame.append(&text);
    frame.append(&hint);

    // Not `AdwClamp`: it centres its child, and the canvas has the field
    // flush against the sidebar toggle. Start-aligned and non-expanding gives
    // the same cap without the centring.
    frame.set_halign(gtk::Align::Start);
    frame.set_valign(gtk::Align::Center);
    frame.set_hexpand(false);
    // The canvas' 16px gap between the sidebar toggle and the field.
    frame.set_margin_start(16);

    let field = finder::Field {
        frame: frame.clone(),
        icon,
        marker,
        text,
        hint,
    };
    (frame.upcast(), field)
}

/// `Keys ?` — the cheat sheet, which arrives in E6.
fn keys_button() -> gtk::Button {
    let button = gtk::Button::builder()
        .tooltip_text("Keyboard shortcuts")
        .build();
    button.add_css_class("flat");
    button.add_css_class("postio-ghost");
    button.set_child(Some(&labelled("Keys", "?")));
    button.update_property(&[gtk::accessible::Property::Label("Keyboard shortcuts")]);
    button
}

/// `Compose c` — the one suggested action in the whole bar.
fn compose_button() -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.append(&gtk::Image::from_icon_name("document-edit-symbolic"));
    content.append(&labelled("Compose", "c"));

    let button = gtk::Button::builder()
        .tooltip_text("Compose a message")
        .build();
    button.add_css_class("suggested-action");
    button.add_css_class("postio-compose");
    button.set_child(Some(&content));
    button.update_property(&[gtk::accessible::Property::Label("Compose a message")]);
    // The composer installs `win.compose` when it is mounted; naming the
    // action here rather than taking a callback keeps the button working
    // whether or not a composer is in the window, and keeps the mouse path
    // and the `c` key on the one command.
    button.set_action_name(Some("win.compose"));
    button
}

fn menu_button() -> gtk::MenuButton {
    let menu = gio::Menu::new();
    menu.append(Some("Preferences"), Some("app.preferences"));
    menu.append(Some("Keyboard Shortcuts"), Some("win.show-help-overlay"));
    menu.append(Some("About Postio"), Some("app.about"));

    let button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Main menu")
        .menu_model(&menu)
        .build();
    button.add_css_class("flat");
    button.update_property(&[gtk::accessible::Property::Label("Main menu")]);
    button
}

/// A label and the key that reaches it, set in the mono face.
pub(crate) fn labelled(text: &str, key: &str) -> gtk::Widget {
    let label = gtk::Label::new(Some(text));

    // The shortcut is already in the button's accessible label; a screen
    // reader announcing a bare "c" after "Compose" is noise.
    let hint = gtk::Label::new(Some(key));
    hint.add_css_class("postio-keyhint");
    hint.set_accessible_role(gtk::AccessibleRole::Presentation);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&label);
    row.append(&hint);
    row.upcast()
}
