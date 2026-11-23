//! Every command Focus offers has a key, a command-bar row and a control a person can see. A
//! command that gains all three and is still in `NOT_YET` fails, so the list only shrinks.
//!
//! POSTIO-MEASUREMENT: a sweep over every command's key, bar row and control
//! rather than a case about one of them; it runs on the nightly timer with
//! `every_command` and `a11y_sweep`.

use postio_config::paths::Platform;
use postio_core::{
    ActionId, CommandId, CommandSpec, Context, ContextSet, Frontend, Keymap, registry,
};

use crate::support;

const NOT_YET: &[&str] = &[
    "go_to_calendar",
    "calendar_agenda",
    "calendar_previous",
    "calendar_next",
    "calendar_today",
    "pop_out",
    "pop_in",
    "new_event",
    "edit_event",
    "move_event",
    "duplicate_event",
    "tentative_invite",
    "save_event",
    "save_and_send_event",
    "email_attendees",
    // A window's own close button is its control, drawn by its header bar, not listed here.
    "close_window",
    "first_message",
    "last_message",
    "toggle_selection",
    "extend_selection_down",
    "extend_selection_up",
    "select_all",
    "prev_view",
    "toggle_result_order",
    "next_in_thread",
    "prev_in_thread",
    // The thread's own commands act in the open message, outline or composer, which the command bar
    // (what the list reaches) does not list.
    // First, latest and first unread have no button: the chips step to any message.
    "first_in_thread",
    "latest_in_thread",
    "first_unread_in_thread",
    "outline_first_unread",
    "thread_menu",
    "split_thread",
    "forward_what_1",
    "forward_what_2",
    "forward_what_3",
    "show_newer",
    "expand_quoted",
    "toggle_fold",
    "view_original",
    "toggle_reader_view",
    "darken_message",
    // The open message's own: its control is the render-mode line over an HTML body.
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
    "more_actions",
    "save_search",
    "back_to_words",
    "send",
    "schedule_send",
    "save_draft",
    "discard_draft",
    "attach_file",
    "detach_composer",
    "copy_fields",
    "insert_image",
    "bold",
    "italic",
    "heading_1",
    "bullet_list",
    "numbered_list",
    "insert_link",
    "quote_block",
    // Spec 014's composer commands: the composer's, like the six above, so
    // the bar (which lists the list's) has no row for them. The source
    // view's toggle and Download and attach have their controls; the
    // formatting commands get theirs with ADR 0039's toolbar (Part B).
    "heading_2",
    "heading_3",
    "heading_4",
    "heading_5",
    "heading_6",
    "normal_text",
    "strikethrough",
    "inline_code",
    "code_block",
    "task_list",
    "toggle_task",
    "indent",
    "outdent",
    "divider",
    "table",
    "clear_formatting",
    "toggle_markdown_source",
    "attach_remote_image",
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
    // The capture sheet's own keys each have a control in the sheet; the command bar lists what the
    // list reaches.
    "capture_change_project",
    "capture_use_subject",
    "capture_write",
    // Spec 010's dropdown keys (#1825) act on the dropdown's row, so the dropdown is their control
    // and the command bar has no row for them.
    "show_all_results",
    "forget_recent",
    "exclude_suggestion",
    // History moves by its keys alone (and the Mac's swipe), with no button.
    "history_back",
    "history_forward",
    // The Contacts view's own keys are reached from the view; the command bar lists what the list
    // reaches.
    "contact_popover",
    "contact_open_card",
    "contact_add",
    "contact_ignore",
    "contact_arrival_prev",
    "contact_arrival_next",
    "contact_set_arrival",
    "contact_show_mail",
    "contact_compose",
    "contacts_filter",
    "contacts_tab_previous",
    "contacts_tab_next",
    "contacts_toggle_everyone",
    "contact_join",
    "contact_add_address",
    "contact_detach_address",
    "contact_set_preferred",
    "contact_new",
    "contact_edit",
    "contact_save",
    "contact_delete",
    "contact_restore",
    "contacts_toggle_deleted",
    "suggestion_dismiss",
    "contact_group_new",
    "contact_group_rename",
    "contact_group_add",
    "contact_group_remove",
    "contacts_import",
    "contacts_export",
];

fn focus_commands_on_linux() -> impl Iterator<Item = &'static CommandSpec> {
    registry::all().filter(|spec| {
        spec.requires.offered_by(Frontend::Focus)
            && registry::offered_on(ActionId::Builtin(spec.id), Platform::Freedesktop)
    })
}

pub fn every_focus_command_has_a_key_a_bar_row_and_a_control() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;
        let keymap = Keymap::defaults();
        let bar = window.command_bar_rows();
        let controls = window.controls();
        let mut missing = Vec::new();
        let mut listed_but_whole = Vec::new();
        for spec in focus_commands_on_linux() {
            let id: CommandId = spec.id;
            let mut lacks = Vec::new();
            if keymap.binding(id).is_none() {
                lacks.push("key");
            }
            // The results' own keys (#1825) are reached from the results, whose controls draw them;
            // the command bar lists what the list reaches.
            let results_only = spec.contexts == ContextSet::of(Context::Results);
            if !results_only && !bar.contains(&id) {
                lacks.push("command-bar row");
            }
            if !results_only && !controls.contains(&id) {
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

const NOT_RUN: &[CommandId] = &[CommandId::Quit, CommandId::CloseWindow];

/// A command with a key, bar row and control must also reach an arm of `FocusWindow::act`: `g s`
/// once passed those checks while pressing it did nothing.
pub fn every_offered_command_that_is_whole_reaches_a_handler() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;
        let mut unanswered = Vec::new();
        for spec in focus_commands_on_linux() {
            let id: CommandId = spec.id;
            if NOT_YET.contains(&id.as_str()) || NOT_RUN.contains(&id) {
                continue;
            }
            window.take_unanswered();
            window.act(id);
            crate::settle();
            unanswered.extend(window.take_unanswered());
            // Whatever it opened is closed before the next; Settings' own keys close it, so `Back`
            // acted on the window does not.
            if let Some(settings) = window.settings() {
                settings.close();
            }
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
