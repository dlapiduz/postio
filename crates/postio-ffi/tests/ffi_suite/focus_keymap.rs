//! `[keys]` reloaded and the key map, at the boundary (specs/009-focus-macos
//! T103, for the Mac's T104-T106).
//!
//! The controller decides (`postio-focus`'s `tests/keymap.rs`); these assert
//! the Mac gets it through the session: `?` opens the key map as
//! `FocusOpenKeyMap`, carrying the sheet `postio_ui::keymap_sheet` groups for
//! this platform, and closes it again; a key rebound in `config.toml` is in
//! the sheet, the menu's bindings and the keyboard at once.

use postio_config::paths::Platform;
use postio_core::{CommandId, Frontend, Keymap};
use postio_ffi::{KeyMapSheetFfi, Session, SessionOptions, SurfaceKindFfi, UiEvent};
use postio_ui::keymap_sheet;

use crate::focus::heard;

/// The keys a sheet shows for `command`.
fn keys_of(sheet: &KeyMapSheetFfi, command: CommandId) -> Vec<String> {
    sheet
        .groups
        .iter()
        .flat_map(|group| &group.rows)
        .find(|row| row.command == command.to_string())
        .map(|row| row.keys.clone())
        .unwrap_or_else(|| panic!("{command:?} is in the key map"))
}

#[tokio::test(flavor = "multi_thread")]
async fn question_mark_opens_the_key_map_and_closes_it() {
    let session = Session::open(SessionOptions::in_memory()).expect("a session");
    session.invoke(&CommandId::CheatSheet.to_string());
    let mut sheet = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusOpenKeyMap { sheet: said } => {
                sheet = Some(said.clone());
                true
            }
            _ => false,
        })
        .await,
        "`?` opens the key map"
    );
    let sheet = sheet.expect("seen");
    let keymap = Keymap::resolve(&Default::default());
    let expected: Vec<&str> = keymap_sheet::key_map_on(&keymap, Frontend::Focus, Platform::host())
        .iter()
        .map(|(group, _)| group.title())
        .collect();
    assert_eq!(
        sheet
            .groups
            .iter()
            .map(|group| group.title.as_str())
            .collect::<Vec<_>>(),
        expected,
        "grouped as on Linux, from the one table"
    );
    assert_eq!(sheet.title, keymap_sheet::TITLE);
    assert_eq!(
        sheet.rebind_footer,
        keymap_sheet::rebind_footer(Platform::host())
    );
    assert!(
        sheet
            .close_keys
            .contains(&postio_ui::hints::key(&keymap, CommandId::CheatSheet).expect("`?`")),
        "`?` is among the keys that close it: {:?}",
        sheet.close_keys
    );
    assert_eq!(
        sheet.columns.concat(),
        (0..u32::try_from(sheet.groups.len()).expect("a few groups")).collect::<Vec<_>>(),
        "every group is in a column, in order"
    );

    session.invoke(&CommandId::CheatSheet.to_string());
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusCloseSurface {
                kind: SurfaceKindFfi::KeyMap
            }
        ))
        .await,
        "`?` again closes it"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rebound_key_is_in_the_key_map_and_the_menu_at_once() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("config.toml");
    std::fs::write(&path, "[keys]\narchive = \"w\"\n").expect("written");
    let session = Session::open(SessionOptions::in_memory().with_config_file_for_test(&path))
        .expect("a session");
    assert_eq!(
        keys_of(&session.focus_key_map(), CommandId::Archive)
            .first()
            .map(String::as_str),
        Some("w")
    );

    std::fs::write(&path, "[keys]\narchive = \"F9\"\n").expect("rewritten");
    assert!(
        heard(&session, 5, |event| matches!(event, UiEvent::KeymapChanged)).await,
        "the Mac is told the keys changed"
    );
    assert_eq!(
        keys_of(&session.focus_key_map(), CommandId::Archive)
            .first()
            .map(String::as_str),
        Some("F9"),
        "the key map shows the new key without a restart"
    );
    assert_eq!(
        session
            .bindings_for("archive".to_owned())
            .first()
            .map(String::as_str),
        Some("F9"),
        "and so does the menu item"
    );
    session.shutdown();
}

#[test]
fn command_w_does_not_quit_on_the_mac() {
    // `quit`'s alternate `mod+w` is GTK's (spec 007 T216: one window, so
    // closing it is quitting). On the Mac ⌘W closes the window in front --
    // the message window over the list most of all -- through Window ›
    // Close, and the key monitor runs before the menu: resolved to Quit,
    // ⌘W in the message window ended the app.
    let session = Session::open(SessionOptions::in_memory()).expect("a session");
    let command = postio_ffi::ModifiersFfi {
        control: false,
        option: false,
        shift: false,
        command: true,
    };
    for context in [postio_ffi::UiContext::List, postio_ffi::UiContext::Reader] {
        let outcome = session.key(Some("w"), None, command, context, false);
        assert!(
            !matches!(&outcome, postio_ffi::KeyOutcomeFfi::Command { id } if id == "quit"),
            "⌘W quits in {context:?}"
        );
    }
    let outcome = session.key(Some("q"), None, command, postio_ffi::UiContext::List, false);
    assert!(
        matches!(&outcome, postio_ffi::KeyOutcomeFfi::Command { id } if id == "quit"),
        "⌘Q still quits: {outcome:?}"
    );
    session.shutdown();
}
