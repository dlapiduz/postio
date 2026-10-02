//! Registry parity (US7, T059; constitution II): every command Focus offers
//! has a key, a row in the command bar, and a control a person can see.
//!
//! What does not yet is named in `NOT_YET`, and each story empties it: a
//! command that gains all three and is still listed fails too, so the list
//! can only shrink.

use postio_core::{CommandId, Frontend, Keymap, registry};

use crate::support;

/// Commands Focus offers that still miss a key, a command-bar row or a
/// control. The command bar lists what the list's context reaches; the rest
/// wait for the surfaces their stories build.
const NOT_YET: &[&str] = &[
    "first_message",
    "last_message",
    "toggle_selection",
    "extend_selection_down",
    "extend_selection_up",
    "select_all",
    "prev_view",
    "toggle_result_order",
    "next_in_conversation",
    "prev_in_conversation",
    "toggle_fold",
    "view_original",
    "toggle_reader_view",
    "darken_message",
    // The open message's own (T213): its control is the render-mode line
    // over an HTML body, which no list-context surface shows.
    "switch_treatment",
    "find_in_message",
    "find_next",
    "find_previous",
    "zoom_in",
    "zoom_out",
    "zoom_reset",
    "expand_all",
    "move",
    "stop_digesting_sender",
    "view_source",
    "open_attachment_or_link",
    // The open message's own (T206): its control is the dialog's More, and
    // the command bar lists what the list reaches.
    "more_actions",
    "save_search",
    "back_to_words",
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
    "edit_config",
    "show_images",
    "always_show_images",
    "unsubscribe",
    "update_credential",
    "next_scope",
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
    "digest_like_this",
    "restore_filtered",
    "filtered_tab_1",
    "filtered_tab_2",
    "filtered_tab_3",
    "filtered_tab_4",
    "filtered_tab_5",
    "filtered_tab_6",
    "filtered_tab_7",
    // The capture sheet's own keys (T158): each has its control in the
    // sheet, and the bar lists what the list reaches, as with the pickers'.
    "capture_change_project",
    "capture_use_subject",
    "capture_write",
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

/// Commands that are answered, but not by running them here: `Quit` closes
/// the window the case is standing in.
const NOT_RUN: &[CommandId] = &[
    CommandId::Quit,
    // Answered by the Settings lane's arm (its own test), which this branch
    // does not carry yet.
    CommandId::Settings,
];

/// A command Focus offers with its key, bar row and control must also reach
/// an arm of `FocusWindow::act`: the three above are all things to look at,
/// and `g s` passed them while pressing it did nothing (T236). Each command
/// outside `NOT_YET` is run, and none may fall through to "no Focus surface
/// answers this command yet".
pub fn every_offered_command_that_is_whole_reaches_a_handler() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        let mut unanswered = Vec::new();
        for spec in registry::all().filter(|spec| spec.requires.offered_by(Frontend::Focus)) {
            let id: CommandId = spec.id;
            if NOT_YET.contains(&id.as_str()) || NOT_RUN.contains(&id) {
                continue;
            }
            window.take_unanswered();
            window.act(id);
            crate::settle();
            unanswered.extend(window.take_unanswered());
            // Whatever it opened -- a picker, the bar, the folders -- is
            // closed before the next.
            window.act(CommandId::Back);
            window.act(CommandId::Back);
            window.take_unanswered();
            crate::settle();
        }
        assert!(
            unanswered.is_empty(),
            "Focus offers these commands and `FocusWindow::act` has no arm for them:\n  {}",
            unanswered
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    });
}
