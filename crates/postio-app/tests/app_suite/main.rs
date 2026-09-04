//! One binary for postio-app's single-test suites — the same custom harness
//! as postio-gtk's `gtk_suite`, for the same two reasons: GTK initializes on
//! exactly one thread per process (#41) while libtest runs tests on a pool,
//! and every test binary here links the entire application. See
//! crates/postio-gtk/tests/gtk_suite/main.rs for the full rationale (#329).
//!
//! The `e2e*` binaries stay out on purpose: they run under the headless
//! runner's watchdog by *name* (#272), in isolation.

mod account_connection_wiring;
mod add_account_wiring;
mod aiming;
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
mod correlation;
mod cursor_preview;
mod decode_notice;
mod degraded_unified;
mod drag_out_portal;
mod drag_out_wiring;
mod dwell_wiring;
mod egress_wiring;
mod event_fanout;
mod keystroke;
mod label_wiring;
mod list_contract;
mod manual_sync;
mod onboarding_probe;
mod orientation;
mod parts_open_wiring;
mod read_receipt_wiring;
mod reader_loads;
mod reading;
mod reading_offline;
mod reclaim_pages;
mod reclaim_wiring;
mod reply_identity;
mod reply_source;
mod resume_draft;
mod resume_queued_draft;
mod search_close_without_escape;
mod search_index;
mod search_live;
mod search_open;
mod search_results;
mod search_return_and_tab;
mod search_unreachable_retraction;
mod search_wiring;
mod second_activate_wiring;
mod send_later_wiring;
mod send_wiring;
mod settings_account_detail_wiring;
mod settings_accounts_token_wiring;
mod settings_accounts_wiring;
mod settings_credential_wiring;
mod settings_reindex_wiring;
mod sidebar_backfill_wiring;
mod signature_default_wiring;
mod startup_repair;
mod storage_ceiling_wiring;
mod sync_window;
mod thread_bulk_keystroke;
mod thread_dwell;
mod thread_keystroke;
mod unconfirmed_send;
mod unified_list;
mod unified_search;
mod unified_search_reach;
mod unified_select_all;
mod unsubscribe_wiring;
mod window_drain;
mod wiring;

/// Cases held out of a default run, by name.
///
/// libtest spells this `#[ignore]`; a table-driven harness needs a table. A
/// name here still runs when asked for explicitly, and still appears in
/// `--list`, exactly as an ignored libtest case does.
const IGNORED: &[&str] = &["parts_open_wiring::opening_and_open_with_ing_a_part_reach_the_desktop"];

const CASES: &[(&str, fn())] = &[
    (
        "list_contract::the_list_output_stays_libtest_shaped",
        list_contract::the_list_output_stays_libtest_shaped as fn(),
    ),
    (
        "account_connection_wiring::a_connection_event_a_scope_cycle_and_the_trackers_all_agree_with_appstate",
        account_connection_wiring::a_connection_event_a_scope_cycle_and_the_trackers_all_agree_with_appstate
            as fn(),
    ),
    (
        "add_account_wiring::the_add_account_key_opens_a_blank_form_over_the_running_window",
        add_account_wiring::the_add_account_key_opens_a_blank_form_over_the_running_window as fn(),
    ),
    (
        "add_account_wiring::closing_the_dialog_stops_the_probe_it_started",
        add_account_wiring::closing_the_dialog_stops_the_probe_it_started as fn(),
    ),
    (
        "aiming::the_gtk_adapter_aims_every_gesture_the_way_the_shared_table_says",
        aiming::the_gtk_adapter_aims_every_gesture_the_way_the_shared_table_says as fn(),
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
        "decode_notice::a_body_that_did_not_decode_cleanly_says_so_in_the_pane",
        decode_notice::a_body_that_did_not_decode_cleanly_says_so_in_the_pane as fn(),
    ),
    (
        "degraded_unified::the_unified_list_names_an_account_it_could_not_reach_and_then_forgets_it",
        degraded_unified::the_unified_list_names_an_account_it_could_not_reach_and_then_forgets_it
            as fn(),
    ),
    (
        "unified_select_all::select_all_in_a_degraded_unified_view_archives_only_what_it_could_see",
        unified_select_all::select_all_in_a_degraded_unified_view_archives_only_what_it_could_see
            as fn(),
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
        "manual_sync::the_status_lines_sync_button_asks_for_a_refresh",
        manual_sync::the_status_lines_sync_button_asks_for_a_refresh as fn(),
    ),
    (
        "orientation::the_first_sync_shows_it_and_got_it_ends_it_for_every_later_run",
        orientation::the_first_sync_shows_it_and_got_it_ends_it_for_every_later_run as fn(),
    ),
    (
        "orientation::a_command_retires_it_even_when_it_was_never_on_screen",
        orientation::a_command_retires_it_even_when_it_was_never_on_screen as fn(),
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
        "reclaim_pages::a_store_written_before_the_setting_is_converted_by_the_application",
        reclaim_pages::a_store_written_before_the_setting_is_converted_by_the_application as fn(),
    ),
    (
        "reclaim_wiring::opening_a_store_reclaims_what_nothing_references",
        reclaim_wiring::opening_a_store_reclaims_what_nothing_references as fn(),
    ),
    (
        "reclaim_wiring::opening_a_store_with_a_ceiling_evicts_down_to_it",
        reclaim_wiring::opening_a_store_with_a_ceiling_evicts_down_to_it as fn(),
    ),
    (
        "storage_ceiling_wiring::editing_the_ceiling_live_evicts_a_running_stores_oldest_blobs",
        storage_ceiling_wiring::editing_the_ceiling_live_evicts_a_running_stores_oldest_blobs
            as fn(),
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
        "search_index::opening_the_window_indexes_local_headers_without_being_asked",
        search_index::opening_the_window_indexes_local_headers_without_being_asked as fn(),
    ),
    (
        "unified_search::a_unified_search_reaches_every_account",
        unified_search::a_unified_search_reaches_every_account as fn(),
    ),
    (
        "unified_search_reach::a_unified_search_names_the_account_it_could_not_reach",
        unified_search_reach::a_unified_search_names_the_account_it_could_not_reach as fn(),
    ),
    (
        "search_unreachable_retraction::an_account_going_away_and_coming_back_updates_the_caveat_without_asking_again",
        search_unreachable_retraction::an_account_going_away_and_coming_back_updates_the_caveat_without_asking_again
            as fn(),
    ),
    (
        "egress_wiring::opening_the_app_costs_zero_connections_and_the_log_is_auditable",
        egress_wiring::opening_the_app_costs_zero_connections_and_the_log_is_auditable as fn(),
    ),
    (
        "unsubscribe_wiring::clicking_unsubscribe_logs_the_activation_and_the_privacy_pane_lists_it",
        unsubscribe_wiring::clicking_unsubscribe_logs_the_activation_and_the_privacy_pane_lists_it
            as fn(),
    ),
    (
        "read_receipt_wiring::opening_settings_shows_how_many_messages_asked_for_a_receipt",
        read_receipt_wiring::opening_settings_shows_how_many_messages_asked_for_a_receipt as fn(),
    ),
    (
        "search_open::opening_a_previewed_result_shows_it_in_the_reading_pane",
        search_open::opening_a_previewed_result_shows_it_in_the_reading_pane as fn(),
    ),
    (
        "search_close_without_escape::closing_the_finder_without_pressing_escape_still_restores_the_folder",
        search_close_without_escape::closing_the_finder_without_pressing_escape_still_restores_the_folder
            as fn(),
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
        "settings_account_detail_wiring::editing_the_detail_view_writes_straight_to_the_accounts_table",
        settings_account_detail_wiring::editing_the_detail_view_writes_straight_to_the_accounts_table
            as fn(),
    ),
    (
        "settings_accounts_wiring::account_rows_persist_enable_and_mark_removal",
        settings_accounts_wiring::account_rows_persist_enable_and_mark_removal as fn(),
    ),
    (
        "settings_accounts_token_wiring::an_oauth_accounts_row_shows_its_real_persisted_expiry",
        settings_accounts_token_wiring::an_oauth_accounts_row_shows_its_real_persisted_expiry
            as fn(),
    ),
    (
        "settings_credential_wiring::update_credential_opens_a_prefilled_dialog_without_disturbing_the_window",
        settings_credential_wiring::update_credential_opens_a_prefilled_dialog_without_disturbing_the_window
            as fn(),
    ),
    (
        "settings_reindex_wiring::the_rows_own_action_clears_and_refills_its_accounts_local_index",
        settings_reindex_wiring::the_rows_own_action_clears_and_refills_its_accounts_local_index
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
        "thread_bulk_keystroke::marking_two_thread_rows_archives_both_conversations",
        thread_bulk_keystroke::marking_two_thread_rows_archives_both_conversations as fn(),
    ),
    (
        "thread_keystroke::pressing_a_on_a_thread_row_archives_the_whole_conversation",
        thread_keystroke::pressing_a_on_a_thread_row_archives_the_whole_conversation as fn(),
    ),
    (
        "unconfirmed_send::an_unconfirmed_send_is_listed_and_can_be_marked_as_sent",
        unconfirmed_send::an_unconfirmed_send_is_listed_and_can_be_marked_as_sent as fn(),
    ),
    (
        "window_drain::an_event_from_a_producer_that_is_not_the_bus_reaches_the_panes",
        window_drain::an_event_from_a_producer_that_is_not_the_bus_reaches_the_panes as fn(),
    ),
    (
        "unified_list::picking_unified_lists_mail_from_every_account",
        unified_list::picking_unified_lists_mail_from_every_account as fn(),
    ),
    (
        "wiring::a_window_over_a_populated_store_lists_its_mail",
        wiring::a_window_over_a_populated_store_lists_its_mail as fn(),
    ),
    (
        "correlation::a_programmatic_caller_gets_the_answer_to_its_own_archive",
        correlation::a_programmatic_caller_gets_the_answer_to_its_own_archive as fn(),
    ),
    (
        "correlation::a_caller_is_told_when_the_application_refuses",
        correlation::a_caller_is_told_when_the_application_refuses as fn(),
    ),
    (
        "correlation::the_frontends_own_sends_are_unaffected",
        correlation::the_frontends_own_sends_are_unaffected as fn(),
    ),
    (
        "event_fanout::a_second_frontend_sees_everything_the_window_sees",
        event_fanout::a_second_frontend_sees_everything_the_window_sees as fn(),
    ),
    (
        "event_fanout::one_subscription_carries_both_of_the_applications_producers",
        event_fanout::one_subscription_carries_both_of_the_applications_producers as fn(),
    ),
    (
        "onboarding_probe::the_probe_call_site_drives_the_screen_from_a_transport_it_was_given",
        onboarding_probe::the_probe_call_site_drives_the_screen_from_a_transport_it_was_given as fn(),
    ),
    (
        "search_live::a_real_account_answers_a_real_query",
        search_live::a_real_account_answers_a_real_query as fn(),
    ),
    (
        "sync_window::picking_a_sync_window_and_pressing_start_sync_writes_it_to_config_toml",
        sync_window::picking_a_sync_window_and_pressing_start_sync_writes_it_to_config_toml as fn(),
    ),
    (
        "label_wiring::a_label_command_puts_a_label_on_the_message_it_names",
        label_wiring::a_label_command_puts_a_label_on_the_message_it_names as fn(),
    ),
    (
        "label_wiring::a_label_command_with_no_label_opens_the_picker",
        label_wiring::a_label_command_with_no_label_opens_the_picker as fn(),
    ),
];

use gtk::glib;

/// Turn the GTK main loop until there is nothing left to do.
///
/// One definition for the suite. There were 18 identical copies of this and
/// 38 of `settle_until` below, which is the duplication #842 is about: a
/// deadline written into every file cannot be adjusted anywhere.
pub fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

/// Turn the loop until `done`, or give up after ten seconds.
///
/// Ten seconds because that is the number these tests were written with --
/// how long the engine may take to settle -- and it is a different question
/// from `postio_test_support::patience`'s "how long before a wait is a
/// failure". `scaled` keeps the base and applies `POSTIO_TEST_PATIENCE`, so a
/// loaded runner can be given more without editing this.
///
/// Returns whether it happened, because every call site is already inside an
/// `assert!` that says what was expected.
/// Turn the loop while `held` stays true, for a bounded time.
///
/// The inverse of `settle_until`, and it needs a duration rather than a
/// condition: proving something does *not* stop happening cannot be polled
/// for. Three copies, now one.
pub fn settle_while(held: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now()
        + postio_test_support::scaled(std::time::Duration::from_millis(500));
    while std::time::Instant::now() < deadline {
        settle();
        if !held() {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    held()
}

pub fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while std::time::Instant::now() < deadline {
        settle();
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.iter().any(|a| a == "--list") {
        // Two questions, and a libtest-compatible runner asks both: every
        // test, then `--ignored` for the ignored subset. Answering the second
        // with the full list tells a process-per-test runner that everything
        // is ignored -- it then runs nothing and reports success, which looks
        // exactly like a fast green run.
        // Plain `--list` names every case, ignored ones included -- that is
        // what libtest does, and a runner takes the ignored set as a subset
        // of it. `--ignored` narrows to just those.
        let only_ignored = arguments.iter().any(|a| a == "--ignored");
        for (name, _) in CASES {
            if !only_ignored || IGNORED.contains(name) {
                println!("{name}: test");
            }
        }
        // `--format terse` is a machine-readable contract: real libtest emits
        // the names and nothing else, and a runner rejects any line not
        // ending in ": test". The count below is what `cargo test` and the
        // tooling's test counting read, so it stays for the non-terse form.
        if !arguments.iter().any(|a| a == "terse") {
            println!();
            println!("{} tests, 0 benchmarks", CASES.len());
        }
        return;
    }
    // `--exact` means the argument is a whole test name, not a substring: a
    // process-per-test runner passes it for every case, and without it a name
    // that is a prefix of another would run both.
    let exact = arguments.iter().any(|a| a == "--exact");
    let run_ignored_only = arguments.iter().any(|a| a == "--ignored");
    let filters: Vec<&str> = arguments
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    let mut failed = Vec::new();
    let mut ran = 0usize;
    for (name, case) in CASES {
        let matched = filters
            .iter()
            .any(|f| if exact { *name == *f } else { name.contains(f) });
        if !filters.is_empty() && !matched {
            continue;
        }
        // An ignored case runs only when it is asked for by name, or when
        // `--ignored` asks for exactly those -- same rule libtest uses.
        if IGNORED.contains(name) && filters.is_empty() && !run_ignored_only {
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
    // Once, after every case -- not after each one.
    //
    // Per-case sweeping here segfaulted at exit: this suite's cases each
    // stand up a full window with a live engine, and repeatedly tearing
    // WebKit down mid-run left the process crashing in `Error releasing
    // name …WebProcess…` on the way out. `gtk_suite` tolerates the per-case
    // form and these do not, which is worth knowing before someone
    // "unifies" the two harnesses.
    //
    // Once is enough for what #794 is about: nothing should still be
    // attached when `exit()` runs.
    postio_gtk::window::close_all_windows();

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
