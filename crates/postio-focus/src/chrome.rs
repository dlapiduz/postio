//! The window's chrome: the top bar and the header strip
//! (contracts/focus-surface.md, "The window").
//!
//! Every key a control shows is read from the keymap in force, never typed
//! in (constitution II; `check-key-hints-are-derived.py`), so a `[keys]`
//! rebind reaches the keycaps the moment it reaches the keyboard. Every
//! control does what its key does: a click and a key press both go through
//! the window's one `act`, with the command's id.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_ui::hints;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};
use postio_widgets::widgets::{Kind, Size, icon_button};

/// What a control asks the window to do.
type Handler = Rc<dyn Fn(CommandId)>;

/// The top bar and the header strip, and the keycaps on them.
pub struct Chrome {
    top: gtk::CenterBox,
    strip: gtk::Box,
    compose: gtk::Button,
    field_keys: gtk::Box,
    sync: gtk::Label,
    sync_icon: gtk::Image,
    place_key: gtk::Box,
    counts: gtk::Label,
    has_action: gtk::ToggleButton,
    has_action_label: gtk::Label,
    has_action_key: gtk::Box,
    showing: gtk::Label,
    filtered_today: gtk::Box,
    digest_rules: gtk::Box,
    handler: RefCell<Option<Handler>>,
}

/// A cap for `command`'s key under `keymap`, or nothing when it has none.
fn caps_for(holder: &gtk::Box, keymap: &Keymap, command: CommandId) {
    while let Some(child) = holder.first_child() {
        holder.remove(&child);
    }
    if let Some(key) = hints::key(keymap, command) {
        holder.append(&keyhint::cap(&key));
    }
}

impl Chrome {
    /// The chrome, its keys read from `keymap`.
    pub fn new(keymap: &Keymap) -> Rc<Self> {
        // The top bar: compose, the command-bar field, the sync label, the
        // main menu and close.
        let compose = icon_button("document-edit-symbolic", "Compose");
        compose.add_css_class("focus-compose");

        let field = gtk::Button::new();
        postio_widgets::widgets::button::style(&field, Kind::Secondary, Size::Regular);
        field.add_css_class("focus-command-field");
        field.set_width_request(480);
        // Its own width, not the bar's: the prompt inside expands, and an
        // expanding child would otherwise widen the field to the bar.
        field.set_hexpand(false);
        field.set_halign(gtk::Align::Center);
        field.set_valign(gtk::Align::Center);
        let field_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        let search_icon = gtk::Image::from_icon_name("system-search-symbolic");
        search_icon.set_accessible_role(gtk::AccessibleRole::Presentation);
        field_row.append(&search_icon);
        let prompt = gtk::Label::new(Some("Search mail, go to a folder, or run a command"));
        prompt.add_css_class("dim-label");
        prompt.set_xalign(0.0);
        prompt.set_hexpand(true);
        prompt.set_ellipsize(pango::EllipsizeMode::End);
        field_row.append(&prompt);
        let field_keys = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        field_keys.set_valign(gtk::Align::Center);
        field_row.append(&field_keys);
        field.set_child(Some(&field_row));
        field.update_property(&[gtk::accessible::Property::Label(
            "Search mail, go to a folder, or run a command",
        )]);

        let sync_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        sync_row.add_css_class("focus-sync");
        let sync_icon = gtk::Image::from_icon_name("emblem-synchronizing-symbolic");
        sync_icon.set_accessible_role(gtk::AccessibleRole::Presentation);
        let sync = gtk::Label::new(None);
        sync.add_css_class("dim-label");
        sync_row.append(&sync_icon);
        sync_row.append(&sync);

        let menu = gio::Menu::new();
        menu.append(Some("Settings"), Some("win.run::settings"));
        menu.append(Some("Keyboard shortcuts"), Some("win.run::cheat_sheet"));
        menu.append(Some("About"), Some("win.about"));
        menu.append(Some("Quit"), Some("win.run::quit"));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text("Main menu")
            .build();
        menu_button.add_css_class("focus-menu");
        menu_button.add_css_class("flat");

        let close = icon_button("window-close-symbolic", "Close");
        close.add_css_class("focus-close");
        close.add_css_class("circular");

        let end = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        end.append(&sync_row);
        end.append(&menu_button);
        end.append(&close);

        let top = gtk::CenterBox::new();
        top.add_css_class("focus-top-bar");
        top.set_start_widget(Some(&compose));
        top.set_center_widget(Some(&field));
        top.set_end_widget(Some(&end));

        // The header strip: where the person is, the counts, the has-action
        // toggle, and the counts that wait for their features.
        let place = gtk::Button::new();
        postio_widgets::widgets::button::style(&place, Kind::Ghost, Size::Regular);
        place.add_css_class("focus-place");
        place.set_valign(gtk::Align::Center);
        let place_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        let place_name = gtk::Label::new(Some("Inbox"));
        place_name.add_css_class("focus-place-name");
        let place_arrow = gtk::Image::from_icon_name("pan-down-symbolic");
        place_arrow.set_accessible_role(gtk::AccessibleRole::Presentation);
        let place_key = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        place_key.set_valign(gtk::Align::Center);
        place_row.append(&place_name);
        place_row.append(&place_arrow);
        place_row.append(&place_key);
        place.set_child(Some(&place_row));

        let counts_holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        counts_holder.add_css_class("focus-counts");
        let counts = gtk::Label::new(None);
        counts.add_css_class("dim-label");
        counts_holder.append(&counts);

        let divider = gtk::Separator::new(gtk::Orientation::Vertical);
        divider.add_css_class("focus-strip-divider");

        let has_action = gtk::ToggleButton::new();
        postio_widgets::widgets::button::style(&has_action, Kind::Ghost, Size::Regular);
        has_action.add_css_class("focus-has-action");
        has_action.set_valign(gtk::Align::Center);
        let has_action_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        let flag = gtk::Image::from_icon_name("emoji-flags-symbolic");
        flag.set_accessible_role(gtk::AccessibleRole::Presentation);
        let has_action_label = gtk::Label::new(Some("Has action"));
        has_action_label.add_css_class("focus-has-action-label");
        let has_action_key = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        has_action_key.set_valign(gtk::Align::Center);
        has_action_row.append(&flag);
        has_action_row.append(&has_action_label);
        has_action_row.append(&has_action_key);
        has_action.set_child(Some(&has_action_row));

        let showing = gtk::Label::new(None);
        showing.add_css_class("dim-label");
        showing.add_css_class("focus-showing");
        showing.set_visible(false);

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);

        // Shown once filtering and digests have their surfaces (FR-018).
        let filtered_today = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        filtered_today.add_css_class("focus-filtered-today");
        filtered_today.set_visible(false);
        let digest_rules = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        digest_rules.add_css_class("focus-digest-rules");
        digest_rules.set_visible(false);

        let strip = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        strip.add_css_class("focus-header-strip");
        strip.append(&place);
        strip.append(&counts_holder);
        strip.append(&divider);
        strip.append(&has_action);
        strip.append(&showing);
        strip.append(&spacer);
        strip.append(&filtered_today);
        strip.append(&digest_rules);

        let chrome = Rc::new(Chrome {
            top,
            strip,
            compose: compose.clone(),
            field_keys,
            sync,
            sync_icon,
            place_key,
            counts,
            has_action: has_action.clone(),
            has_action_label,
            has_action_key,
            showing,
            filtered_today,
            digest_rules,
            handler: RefCell::default(),
        });
        chrome.set_keymap(keymap);

        for (button, command) in [
            (compose, CommandId::Compose),
            (field, CommandId::Search),
            (place, CommandId::GoToFolders),
            (close, CommandId::Quit),
        ] {
            let weak = Rc::downgrade(&chrome);
            button.connect_clicked(move |_| {
                if let Some(chrome) = weak.upgrade() {
                    chrome.run(command);
                }
            });
        }
        let weak = Rc::downgrade(&chrome);
        has_action.connect_clicked(move |_| {
            if let Some(chrome) = weak.upgrade() {
                chrome.run(CommandId::ToggleHasAction);
            }
        });
        chrome
    }

    /// The top bar, to place in a layout.
    pub fn top_bar(&self) -> &gtk::CenterBox {
        &self.top
    }

    /// The header strip, to place in a layout.
    pub fn strip(&self) -> &gtk::Box {
        &self.strip
    }

    /// Run `handler` with the command a control stands for when it is
    /// pressed.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    fn run(&self, command: CommandId) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(command);
        }
    }

    /// Read every key the chrome shows from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        let compose = match hints::key(keymap, CommandId::Compose) {
            Some(key) => format!("Compose \u{b7} {key}"),
            None => "Compose".to_owned(),
        };
        self.compose.set_tooltip_text(Some(&compose));
        while let Some(child) = self.field_keys.first_child() {
            self.field_keys.remove(&child);
        }
        for command in [CommandId::Search, CommandId::CommandPalette] {
            if let Some(key) = hints::key(keymap, command) {
                self.field_keys.append(&keyhint::framed_cap(&key));
            }
        }
        caps_for(&self.place_key, keymap, CommandId::GoToFolders);
        caps_for(&self.has_action_key, keymap, CommandId::ToggleHasAction);
    }

    /// The counts the strip shows: the conversations and the unread.
    pub fn set_counts(&self, conversations: u32, unread: u32) {
        self.counts
            .set_text(&postio_ui::focus_row::strip_counts(conversations, unread));
    }

    /// What the sync label says, and its icon.
    pub fn set_sync(&self, said: &str, icon: &str) {
        self.sync.set_text(said);
        self.sync_icon.set_icon_name(Some(icon));
    }

    /// What the sync label says.
    pub fn sync_said(&self) -> String {
        self.sync.text().to_string()
    }

    /// Whether the has-action filter is on, and what the toggle and the
    /// strip say about it.
    pub fn set_has_action(&self, on: bool, label: &str, showing: Option<&str>) {
        self.has_action.set_active(on);
        self.has_action_label.set_text(label);
        match showing {
            Some(showing) => {
                self.showing.set_text(showing);
                self.showing.set_visible(true);
            }
            None => self.showing.set_visible(false),
        }
    }

    /// The counts that wait for their features (FR-018), for the tasks that
    /// give them one.
    pub fn waiting_counts(&self) -> (&gtk::Box, &gtk::Box) {
        (&self.filtered_today, &self.digest_rules)
    }
}
