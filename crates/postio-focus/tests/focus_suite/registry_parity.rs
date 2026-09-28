//! Registry parity (US7, T059; constitution II): every command Focus offers
//! has a key, a row in the command bar, and a control a person can see.
//!
//! What does not yet is named in `NOT_YET`, and each story empties it: a
//! command that gains all three and is still listed fails too, so the list
//! can only shrink.

use postio_core::{CommandId, Frontend, Keymap, registry};

use crate::support;

/// Commands Focus offers that still miss a key, a command-bar row or a
/// control. Every one misses its command-bar row until the command bar
/// exists (US4); the rest wait for the surfaces their stories build.
const NOT_YET: &[&str] = &[
    "next_message",
    "prev_message",
    "first_message",
    "last_message",
    "open_message",
    "toggle_selection",
    "extend_selection_down",
    "extend_selection_up",
    "select_all",
    "prev_view",
    "back",
    "toggle_result_order",
    "next_in_conversation",
    "prev_in_conversation",
    "toggle_fold",
    "view_original",
    "toggle_reader_view",
    "darken_message",
    "find_in_message",
    "find_next",
    "find_previous",
    "zoom_in",
    "zoom_out",
    "zoom_reset",
    "expand_all",
    "reply",
    "reply_all",
    "forward",
    "archive",
    "archive_thread",
    "delete",
    "move",
    "toggle_read",
    "snooze",
    "unsnooze",
    "remind_if_no_reply",
    "add_label",
    "accept_invite",
    "decline_invite",
    "digest_rule",
    "stop_digesting_sender",
    // The commands lane's verbs (T118, T128): the engine answers them, and
    // nothing draws them until the bar (US4) and the marker card do.
    "dismiss_marker",
    "sweep_inbox",
    "view_source",
    "open_attachment_or_link",
    "search",
    "save_search",
    "back_to_words",
    "compose",
    "send",
    "schedule_send",
    "save_draft",
    "discard_draft",
    "mark_sent",
    "retry_send",
    "cancel_send",
    "attach_file",
    "detach_composer",
    "copy_fields",
    "insert_image",
    "bold",
    "italic",
    "bullet_list",
    "numbered_list",
    "insert_link",
    "quote_block",
    "undo",
    "command_palette",
    "cheat_sheet",
    "settings",
    "add_account",
    "edit_config",
    "quit",
    "show_images",
    "always_show_images",
    "unsubscribe",
    "go_to_folders",
    "go_to_inbox",
    "go_to_drafts",
    "go_to_sent",
    "go_to_flagged",
    "go_to_archive",
    "go_to_snoozed",
    "go_to_filtered",
    "go_to_digest_rules",
    "saved_search_1",
    "saved_search_2",
    "saved_search_3",
    "saved_search_4",
    "toggle_has_action",
    "update_credential",
    "next_scope",
    "refresh",
    "scroll_reader_down",
    "scroll_reader_up",
    "picker_choose_1",
    "picker_choose_2",
    "picker_choose_3",
    "picker_choose_4",
    "picker_type_date",
    "picker_toggle",
    "picker_confirm",
    "next_reference",
    "prev_reference",
    "toggle_digest_summary",
    "restore_filtered",
    "filtered_tab_1",
    "filtered_tab_2",
    "filtered_tab_3",
    "filtered_tab_4",
    "filtered_tab_5",
    "filtered_tab_6",
    "filtered_tab_7",
];

pub fn every_focus_command_has_a_key_a_bar_row_and_a_control() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        let keymap = Keymap::defaults();
        let bar = window.command_bar_rows();
        let controls = window.controls();
        let mut missing = Vec::new();
        let mut listed_but_whole = Vec::new();
        for spec in registry::all().filter(|spec| spec.requires.offered_by(Frontend::Focus)) {
            let id: CommandId = spec.id;
            let mut lacks = Vec::new();
            if keymap.binding(id).is_none() {
                lacks.push("key");
            }
            if !bar.contains(&id) {
                lacks.push("command-bar row");
            }
            if !controls.contains(&id) {
                lacks.push("control");
            }
            let listed = NOT_YET.contains(&id.as_str());
            match (lacks.is_empty(), listed) {
                (false, false) => missing.push(format!("{} ({})", id.as_str(), lacks.join(", "))),
                (true, true) => listed_but_whole.push(id.as_str()),
                _ => {}
            }
        }
        assert!(
            missing.is_empty(),
            "Focus commands missing a key, a command-bar row or a control, and not in \
             NOT_YET:\n  {}",
            missing.join("\n  ")
        );
        assert!(
            listed_but_whole.is_empty(),
            "in NOT_YET but whole now; take them out: {listed_but_whole:?}"
        );
        assert!(!NOT_YET.is_empty(), "NOT_YET empties story by story");
    });
}
