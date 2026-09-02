//! One binary for postio-app's single-test suites — the same custom harness
//! as postio-gtk's `gtk_suite`, for the same two reasons: GTK initializes on
//! exactly one thread per process (#41) while libtest runs tests on a pool,
//! and every test binary here links the entire application. See
//! crates/postio-gtk/tests/gtk_suite/main.rs for the full rationale (#329).
//!
//! The `e2e*` binaries stay out on purpose: they run under the headless
//! runner's watchdog by *name* (#272), in isolation.

mod add_account_wiring;
mod attach_account;
mod body_arrives;
mod bulk_keystroke;
mod click_preview;
mod command_wiring;
mod compose_detach;
mod compose_typing;
mod conversation_body_arrives;
mod conversation_by_default;
mod conversation_recipients;
mod cursor_preview;
mod drag_out_portal;
mod drag_out_wiring;
mod dwell_wiring;
mod egress_wiring;
mod keystroke;
mod parts_open_wiring;
mod reader_loads;
mod reading;
mod reading_offline;
mod reclaim_wiring;
mod reply_identity;
mod reply_source;
mod resume_draft;
mod resume_queued_draft;
mod search_index;
mod search_results;
mod search_return_and_tab;
mod search_wiring;
mod second_activate_wiring;
mod send_later_wiring;
mod send_wiring;
mod settings_accounts_wiring;
mod settings_credential_wiring;
mod sidebar_backfill_wiring;
mod signature_default_wiring;
mod startup_repair;
mod thread_bulk_keystroke;
mod thread_cursor_preview;
mod thread_dwell;
mod thread_keystroke;
mod window_drain;
mod wiring;

const CASES: &[(&str, fn())] = &[
    (
        "add_account_wiring::the_add_account_key_opens_a_blank_form_over_the_running_window",
        add_account_wiring::the_add_account_key_opens_a_blank_form_over_the_running_window as fn(),
    ),
    (
        "add_account_wiring::closing_the_dialog_stops_the_probe_it_started",
        add_account_wiring::closing_the_dialog_stops_the_probe_it_started as fn(),
    ),
    (
        "attach_account::an_account_added_to_a_running_application_syncs_without_a_restart",
        attach_account::an_account_added_to_a_running_application_syncs_without_a_restart as fn(),
    ),
    (
        "body_arrives::a_body_that_lands_repaints_the_pane_waiting_for_it_and_no_other",
        body_arrives::a_body_that_lands_repaints_the_pane_waiting_for_it_and_no_other as fn(),
    ),
    (
        "bulk_keystroke::ctrl_a_then_shift_u_marks_the_whole_folder_read",
        bulk_keystroke::ctrl_a_then_shift_u_marks_the_whole_folder_read as fn(),
    ),
    (
        "click_preview::clicking_a_message_fills_the_reading_pane",
        click_preview::clicking_a_message_fills_the_reading_pane as fn(),
    ),
    (
        "command_wiring::every_command_id_is_handled_locally_or_wired_to_the_bus",
        command_wiring::every_command_id_is_handled_locally_or_wired_to_the_bus as fn(),
    ),
    (
        "compose_detach::the_detach_key_reaches_the_composer_in_a_wired_application",
        compose_detach::the_detach_key_reaches_the_composer_in_a_wired_application as fn(),
    ),
    (
        "compose_typing::every_letter_can_be_typed_into_the_composer_body",
        compose_typing::every_letter_can_be_typed_into_the_composer_body as fn(),
    ),
    (
        "conversation_body_arrives::a_body_that_lands_repaints_the_conversation_entry_waiting_for_it_and_no_other",
        conversation_body_arrives::a_body_that_lands_repaints_the_conversation_entry_waiting_for_it_and_no_other
            as fn(),
    ),
    (
        "conversation_by_default::landing_on_a_thread_row_opens_the_conversation",
        conversation_by_default::landing_on_a_thread_row_opens_the_conversation as fn(),
    ),
    (
        "conversation_recipients::an_expanded_entry_shows_who_it_went_to_without_repeating_its_header",
        conversation_recipients::an_expanded_entry_shows_who_it_went_to_without_repeating_its_header
            as fn(),
    ),
    (
        "cursor_preview::the_pane_follows_the_cursor_and_says_why_a_body_is_missing",
        cursor_preview::the_pane_follows_the_cursor_and_says_why_a_body_is_missing as fn(),
    ),
    (
        "drag_out_portal::a_dragged_message_survives_the_portal",
        drag_out_portal::a_dragged_message_survives_the_portal as fn(),
    ),
    (
        "drag_out_wiring::a_message_in_the_list_can_be_dragged_out_as_a_file",
        drag_out_wiring::a_message_in_the_list_can_be_dragged_out_as_a_file as fn(),
    ),
    (
        "dwell_wiring::resting_on_a_message_marks_it_read_and_sweeping_past_does_not",
        dwell_wiring::resting_on_a_message_marks_it_read_and_sweeping_past_does_not as fn(),
    ),
    (
        "keystroke::pressing_a_archives_the_row_in_the_database",
        keystroke::pressing_a_archives_the_row_in_the_database as fn(),
    ),
    (
        "parts_open_wiring::opening_and_open_with_ing_a_part_reach_the_desktop",
        parts_open_wiring::opening_and_open_with_ing_a_part_reach_the_desktop as fn(),
    ),
    (
        "reader_loads::moving_and_reopening_a_message_costs_one_document_load_each",
        reader_loads::moving_and_reopening_a_message_costs_one_document_load_each as fn(),
    ),
    (
        "reading::opening_a_message_fills_the_pane_and_its_chips_open_the_parts_tree",
        reading::opening_a_message_fills_the_pane_and_its_chips_open_the_parts_tree as fn(),
    ),
    (
        "reading_offline::the_pane_says_offline_and_updates_the_moment_the_connection_does",
        reading_offline::the_pane_says_offline_and_updates_the_moment_the_connection_does as fn(),
    ),
    (
        "reclaim_wiring::opening_a_store_reclaims_what_nothing_references",
        reclaim_wiring::opening_a_store_reclaims_what_nothing_references as fn(),
    ),
    (
        "reply_identity::a_reply_to_a_message_in_a_second_account_uses_that_accounts_identity",
        reply_identity::a_reply_to_a_message_in_a_second_account_uses_that_accounts_identity
            as fn(),
    ),
    (
        "reply_source::reply_forward_and_reply_all_act_on_the_message_under_the_cursor",
        reply_source::reply_forward_and_reply_all_act_on_the_message_under_the_cursor as fn(),
    ),
    (
        "resume_draft::return_on_a_draft_row_opens_the_composer_on_that_draft",
        resume_draft::return_on_a_draft_row_opens_the_composer_on_that_draft as fn(),
    ),
    (
        "resume_queued_draft::return_on_a_queued_draft_row_cancels_the_send_and_reopens_it_for_editing",
        resume_queued_draft::return_on_a_queued_draft_row_cancels_the_send_and_reopens_it_for_editing
            as fn(),
    ),
    (
        "search_index::a_store_the_application_opened_can_be_searched",
        search_index::a_store_the_application_opened_can_be_searched as fn(),
    ),
    (
        "search_index::a_store_that_predates_body_indexing_catches_up",
        search_index::a_store_that_predates_body_indexing_catches_up as fn(),
    ),
    (
        "search_index::opening_the_window_indexes_local_bodies_without_being_asked",
        search_index::opening_the_window_indexes_local_bodies_without_being_asked as fn(),
    ),
    (
        "egress_wiring::opening_the_app_costs_zero_connections_and_the_log_is_auditable",
        egress_wiring::opening_the_app_costs_zero_connections_and_the_log_is_auditable as fn(),
    ),
    (
        "search_results::a_query_puts_the_matching_messages_in_the_list",
        search_results::a_query_puts_the_matching_messages_in_the_list as fn(),
    ),
    (
        "search_return_and_tab::return_and_tab_move_the_keyboard_to_the_message_list",
        search_return_and_tab::return_and_tab_move_the_keyboard_to_the_message_list as fn(),
    ),
    (
        "search_wiring::typing_in_the_box_searches_the_store_and_fills_every_search_surface",
        search_wiring::typing_in_the_box_searches_the_store_and_fills_every_search_surface
            as fn(),
    ),
    (
        "second_activate_wiring::a_second_activate_does_not_double_wire_the_window",
        second_activate_wiring::a_second_activate_does_not_double_wire_the_window as fn(),
    ),
    (
        "send_later_wiring::choosing_a_time_schedules_the_draft_for_sending",
        send_later_wiring::choosing_a_time_schedules_the_draft_for_sending as fn(),
    ),
    (
        "send_wiring::ctrl_return_queues_the_draft_for_sending",
        send_wiring::ctrl_return_queues_the_draft_for_sending as fn(),
    ),
    (
        "settings_accounts_wiring::account_rows_persist_enable_and_mark_removal",
        settings_accounts_wiring::account_rows_persist_enable_and_mark_removal as fn(),
    ),
    (
        "settings_credential_wiring::update_credential_opens_a_prefilled_dialog_without_disturbing_the_window",
        settings_credential_wiring::update_credential_opens_a_prefilled_dialog_without_disturbing_the_window
            as fn(),
    ),
    (
        "sidebar_backfill_wiring::the_menu_persists_and_the_sidebar_reflects_it_without_a_sync",
        sidebar_backfill_wiring::the_menu_persists_and_the_sidebar_reflects_it_without_a_sync
            as fn(),
    ),
    (
        "signature_default_wiring::compose_signs_with_the_selected_mailbox_or_account_default",
        signature_default_wiring::compose_signs_with_the_selected_mailbox_or_account_default
            as fn(),
    ),
    (
        "startup_repair::an_account_with_no_credential_lands_on_the_repair_screen",
        startup_repair::an_account_with_no_credential_lands_on_the_repair_screen as fn(),
    ),
    (
        "thread_dwell::resting_inside_a_conversation_reads_each_message_as_focus_reaches_it",
        thread_dwell::resting_inside_a_conversation_reads_each_message_as_focus_reaches_it as fn(),
    ),
    (
        "thread_cursor_preview::moving_the_thread_cursor_fills_the_reading_pane",
        thread_cursor_preview::moving_the_thread_cursor_fills_the_reading_pane as fn(),
    ),
    (
        "thread_bulk_keystroke::marking_two_thread_rows_archives_both_conversations",
        thread_bulk_keystroke::marking_two_thread_rows_archives_both_conversations as fn(),
    ),
    (
        "thread_keystroke::pressing_a_on_a_thread_row_archives_the_whole_conversation",
        thread_keystroke::pressing_a_on_a_thread_row_archives_the_whole_conversation as fn(),
    ),
    (
        "window_drain::an_event_from_a_producer_that_is_not_the_bus_reaches_the_panes",
        window_drain::an_event_from_a_producer_that_is_not_the_bus_reaches_the_panes as fn(),
    ),
    (
        "wiring::a_window_over_a_populated_store_lists_its_mail",
        wiring::a_window_over_a_populated_store_lists_its_mail as fn(),
    ),
];

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.iter().any(|a| a == "--list") {
        for (name, _) in CASES {
            println!("{name}: test");
        }
        println!();
        println!("{} tests, 0 benchmarks", CASES.len());
        return;
    }
    let filters: Vec<&str> = arguments
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    let mut failed = Vec::new();
    let mut ran = 0usize;
    for (name, case) in CASES {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f)) {
            continue;
        }
        ran += 1;
        println!("test {name} ...");
        if std::panic::catch_unwind(case).is_err() {
            println!("test {name} ... FAILED");
            failed.push(*name);
        } else {
            println!("test {name} ... ok");
        }
    }
    if failed.is_empty() {
        println!("\ntest result: ok. {ran} passed; 0 failed");
    } else {
        println!("\nfailures:");
        for name in &failed {
            println!("    {name}");
        }
        println!(
            "\ntest result: FAILED. {} passed; {} failed",
            ran - failed.len(),
            failed.len()
        );
        std::process::exit(101);
    }
}
