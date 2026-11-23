use gtk::glib;
use gtk::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_ui::hints;

use crate::support::{self, only, texts};

fn key(command: CommandId) -> String {
    hints::key(Keymap::defaults(), command).expect("the one keymap binds it")
}

pub fn the_top_bar_and_the_header_strip_carry_each_control_and_its_key() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;

        assert_eq!(key(CommandId::Search), "/");
        assert_eq!(key(CommandId::CommandPalette), "ctrl+k");
        assert_eq!(key(CommandId::GoToFolders), "g l");
        assert_eq!(key(CommandId::ToggleHasAction), "!");

        let compose = only(&window, "focus-compose");
        let told = compose.tooltip_text().unwrap_or_default();
        assert!(
            told.contains("Compose") && told.contains(&key(CommandId::Compose)),
            "compose names itself and its key: {told:?}"
        );
        let field = texts(&only(&window, "focus-command-field"));
        for said in [
            "Search mail, go to a folder, or run a command".to_owned(),
            key(CommandId::Search),
            key(CommandId::CommandPalette),
        ] {
            assert!(
                field.contains(&said),
                "the command-bar field shows {said:?}: {field:?}"
            );
        }
        assert!(
            only(&window, "focus-sync").is_mapped(),
            "the sync label is on screen"
        );
        let menu = only(&window, "focus-menu")
            .downcast::<gtk::MenuButton>()
            .expect("the main menu is a menu button");
        let model = menu.menu_model().expect("the menu has items");
        let items: Vec<String> = (0..model.n_items())
            .filter_map(|item| {
                model
                    .item_attribute_value(item, "label", Some(glib::VariantTy::STRING))
                    .and_then(|label| label.get::<String>())
            })
            .collect();
        assert_eq!(
            items,
            [
                "Contacts",
                "Settings",
                "Read beside the list",
                "Keyboard shortcuts",
                "About",
                "Quit"
            ],
            "the main menu holds what the contract names"
        );
        let close = only(&window, "focus-close");
        assert_eq!(close.tooltip_text().as_deref(), Some("Close"));

        let place = texts(&only(&window, "focus-place"));
        assert!(
            place.contains(&"Inbox".to_owned()) && place.contains(&key(CommandId::GoToFolders)),
            "Inbox names its key: {place:?}"
        );
        assert!(
            crate::settle_until(async || {
                texts(&only(&window, "focus-counts"))
                    .first()
                    .is_some_and(|said| said.starts_with('3'))
            })
            .await,
            "the strip counts the three conversations: {:?}",
            texts(&only(&window, "focus-counts"))
        );
        let toggle = texts(&only(&window, "focus-has-action"));
        assert!(
            toggle.iter().any(|said| said.starts_with("Has action"))
                && toggle.contains(&key(CommandId::ToggleHasAction)),
            "the has-action toggle names its key: {toggle:?}"
        );

        for class in ["focus-filtered-today", "focus-digest-rules"] {
            assert!(
                !only(&window, class).is_mapped(),
                "{class} is shown with nothing to count"
            );
        }
    });
}
