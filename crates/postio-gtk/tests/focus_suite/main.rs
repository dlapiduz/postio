//! One custom-harness binary: GTK may be initialized from one thread per process (#41) and each
//! extra test binary links the whole GTK stack (#329).
//! A new case is a module here and a row in `CASES`; `list_contract.rs` notices when the `--list`
//! output breaks.

mod a11y;
mod a11y_sweep;
mod across_apps;
mod add_account_running;
mod archive_frames;
mod bar;
mod body_lands;
mod buttons;
mod calendar;
mod calendar_pop_out;
mod capture;
mod chrome;
mod close_buttons;
mod colours;
mod commands;
mod compose;
mod compose_counts;
mod compose_detach;
mod compose_layout;
mod contacts;
mod corrections;
mod cursor;
mod desktop;
mod digest;
mod digest_summary;
mod drafts;
mod drag_out;
mod dropdown;
mod e2e;
mod empty;
mod every_command;
mod filtered;
mod first_run;
mod folder_sync;
mod footers;
mod forward_carried;
mod gmail_keys;
mod harness;
mod hit_composer;
mod hostile_mail;
mod icon_buttons;
mod idle_passes;
mod input_method;
mod invitations;
mod keyboard_home;
mod keycaps;
mod keymap;
mod list_contract;
mod list_reload;
mod marked_rows;
mod marker_card;
mod motion;
mod newsletter_seed;
mod notify_off_the_main_thread;
mod observe;
mod offline_send;
mod one_composer;
mod open_choice;
mod open_keys;
mod open_layout;
mod open_measure;
mod open_message;
mod open_reading;
mod pickers;
mod place_strip;
mod places;
mod pointer_pairs;
mod read_on_dwell;
mod reader_spawns_no_web_process;
mod reader_zoom;
mod reading_pane;
mod registry_parity;
mod reload;
mod remind_on_send;
mod remote_images;
mod results;
mod results_files;
mod row_menu;
mod rows;
mod rule_query;
mod save_attachments;
mod selection;
mod sending_states;
mod settings;
mod settings_wiring;
mod shot;
mod starts_offline;
mod startup_behind_the_window;
mod startup_reads;
mod startup_repair;
mod startup_timeline;
mod state;
mod store_in_use;
mod store_opening;
mod store_unavailable;
mod storyboard_catalogue;
mod storyboard_determinism;
mod storyboards;
mod support;
mod surfaced;
mod threads;
mod treatments;
mod unsubscribe;
mod view_source;
mod visible_window;
mod window_controls;
mod window_state;
mod window_teardown;

/// Cases held out of a default run: this machine may lack what the case needs. A name still runs
/// when asked for and appears in `--list`.
const IGNORED: &[&str] = &["drag_out::a_dragged_message_survives_the_portal"];

const CASES: &[(&str, fn())] = &[
    (
        "gmail_keys::every_list_key",
        gmail_keys::every_list_key as fn(),
    ),
    (
        "gmail_keys::every_reader_key",
        gmail_keys::every_reader_key as fn(),
    ),
    (
        "gmail_keys::every_pane_key",
        gmail_keys::every_pane_key as fn(),
    ),
    (
        "gmail_keys::every_digest_key",
        gmail_keys::every_digest_key as fn(),
    ),
    (
        "gmail_keys::every_filtered_key",
        gmail_keys::every_filtered_key as fn(),
    ),
    (
        "gmail_keys::every_capture_key",
        gmail_keys::every_capture_key as fn(),
    ),
    (
        "gmail_keys::every_menu_key",
        gmail_keys::every_menu_key as fn(),
    ),
    (
        "calendar_pop_out::mod_shift_o_moves_calendar_into_its_own_window_and_back_with_its_ring",
        calendar_pop_out::mod_shift_o_moves_calendar_into_its_own_window_and_back_with_its_ring
            as fn(),
    ),
    (
        "calendar_pop_out::destinations_present_the_window_that_shows_them",
        calendar_pop_out::destinations_present_the_window_that_shows_them as fn(),
    ),
    (
        "calendar_pop_out::closing_the_calendar_window_or_mod_w_pops_calendar_back_in",
        calendar_pop_out::closing_the_calendar_window_or_mod_w_pops_calendar_back_in as fn(),
    ),
    (
        "calendar_pop_out::calendar_popped_out_at_quit_reopens_in_its_own_window",
        calendar_pop_out::calendar_popped_out_at_quit_reopens_in_its_own_window as fn(),
    ),
    (
        "calendar::slash_searches_events_and_escape_brings_the_agenda_back",
        calendar::slash_searches_events_and_escape_brings_the_agenda_back as fn(),
    ),
    (
        "calendar::c_types_a_new_event_and_mod_s_puts_it_in_the_agenda",
        calendar::c_types_a_new_event_and_mod_s_puts_it_in_the_agenda as fn(),
    ),
    (
        "calendar::escape_leaves_the_editor_without_saving",
        calendar::escape_leaves_the_editor_without_saving as fn(),
    ),
    (
        "calendar::v_moves_the_ringed_event_to_next_week",
        calendar::v_moves_the_ringed_event_to_next_week as fn(),
    ),
    (
        "calendar::hash_deletes_the_ringed_event_and_undo_brings_it_back",
        calendar::hash_deletes_the_ringed_event_and_undo_brings_it_back as fn(),
    ),
    (
        "calendar::g_e_opens_agenda_on_the_next_event_and_return_and_escape_walk_back_home",
        calendar::g_e_opens_agenda_on_the_next_event_and_return_and_escape_walk_back_home as fn(),
    ),
    (
        "contacts::g_c_opens_the_people_j_k_walk_them_the_card_follows_and_escape_goes_home",
        contacts::g_c_opens_the_people_j_k_walk_them_the_card_follows_and_escape_goes_home
            as fn(),
    ),
    (
        "contacts::the_strip_says_its_tabs_and_a_click_on_one_is_the_key",
        contacts::the_strip_says_its_tabs_and_a_click_on_one_is_the_key as fn(),
    ),
    (
        "contacts::ctrl_e_edits_escape_arms_before_it_discards_and_ctrl_s_saves",
        contacts::ctrl_e_edits_escape_arms_before_it_discards_and_ctrl_s_saves as fn(),
    ),
    (
        "contacts::i_opens_the_popover_the_arrows_choose_where_mail_arrives_and_escape_closes",
        contacts::i_opens_the_popover_the_arrows_choose_where_mail_arrives_and_escape_closes
            as fn(),
    ),
    (
        "compose_counts::autosave_writes_off_the_main_thread_and_keeps_one_row",
        compose_counts::autosave_writes_off_the_main_thread_and_keeps_one_row as fn(),
    ),
    (
        "compose_counts::opening_a_draft_or_a_reply_reads_nothing_on_the_main_thread",
        compose_counts::opening_a_draft_or_a_reply_reads_nothing_on_the_main_thread as fn(),
    ),
    (
        "compose_counts::typing_back_and_forth_across_the_threshold_stays_connected",
        compose_counts::typing_back_and_forth_across_the_threshold_stays_connected as fn(),
    ),
    (
        "compose_counts::typing_a_recipient_opens_no_connections_and_still_completes",
        compose_counts::typing_a_recipient_opens_no_connections_and_still_completes as fn(),
    ),
    (
        "e2e::a_keystroke_reaches_the_server_and_a_delivery_reaches_the_list",
        e2e::a_keystroke_reaches_the_server_and_a_delivery_reaches_the_list as fn(),
    ),
    (
        "folder_sync::a_folder_the_server_will_not_open_is_marked_until_it_syncs",
        folder_sync::a_folder_the_server_will_not_open_is_marked_until_it_syncs as fn(),
    ),
    (
        "idle_passes::the_idle_passes_wait_for_the_first_frame",
        idle_passes::the_idle_passes_wait_for_the_first_frame as fn(),
    ),
    (
        "notify_off_the_main_thread::a_new_mail_notification_reads_nothing_on_the_main_thread",
        notify_off_the_main_thread::a_new_mail_notification_reads_nothing_on_the_main_thread
            as fn(),
    ),
    (
        "startup_behind_the_window::the_store_opens_behind_a_window_that_is_already_up",
        startup_behind_the_window::the_store_opens_behind_a_window_that_is_already_up as fn(),
    ),
    (
        "startup_behind_the_window::the_store_starts_opening_before_there_is_a_window",
        startup_behind_the_window::the_store_starts_opening_before_there_is_a_window as fn(),
    ),
    (
        "startup_reads::opening_a_window_reads_a_bounded_amount_however_big_the_mailbox_is",
        startup_reads::opening_a_window_reads_a_bounded_amount_however_big_the_mailbox_is as fn(),
    ),
    (
        "startup_repair::an_account_with_no_credential_offers_the_repair",
        startup_repair::an_account_with_no_credential_offers_the_repair as fn(),
    ),
    (
        "a11y_sweep::every_widget_in_focus_s_surfaces_has_a_role_and_a_name",
        a11y_sweep::every_widget_in_focus_s_surfaces_has_a_role_and_a_name as fn(),
    ),
    (
        "a11y_sweep::at_200_percent_text_the_window_stays_usable",
        a11y_sweep::at_200_percent_text_the_window_stays_usable as fn(),
    ),
    (
        "a11y_sweep::at_200_percent_text_rows_grow_with_the_type",
        a11y_sweep::at_200_percent_text_rows_grow_with_the_type as fn(),
    ),
    (
        "window_teardown::a_window_startup_wired_and_used_still_frees_when_destroyed",
        window_teardown::a_window_startup_wired_and_used_still_frees_when_destroyed as fn(),
    ),
    (
        "window_teardown::dropping_a_window_without_destroying_it_is_not_enough",
        window_teardown::dropping_a_window_without_destroying_it_is_not_enough as fn(),
    ),
    (
        "window_teardown::a_destroyed_window_releases_its_composer",
        window_teardown::a_destroyed_window_releases_its_composer as fn(),
    ),
    (
        "desktop::a_second_activate_has_one_window_and_starts_sync_once",
        desktop::a_second_activate_has_one_window_and_starts_sync_once as fn(),
    ),
    (
        "unsubscribe::u_in_a_digest_logs_the_activation_and_the_privacy_section_lists_it",
        unsubscribe::u_in_a_digest_logs_the_activation_and_the_privacy_section_lists_it as fn(),
    ),
    (
        "unsubscribe::leaving_a_list_offers_one_action_on_a_toast_and_ignoring_it_keeps_the_mail",
        unsubscribe::leaving_a_list_offers_one_action_on_a_toast_and_ignoring_it_keeps_the_mail
            as fn(),
    ),
    (
        "unsubscribe::the_setting_makes_the_one_action_delete",
        unsubscribe::the_setting_makes_the_one_action_delete as fn(),
    ),
    (
        "store_opening::a_start_past_its_budget_says_what_it_is_waiting_on",
        store_opening::a_start_past_its_budget_says_what_it_is_waiting_on as fn(),
    ),
    (
        "store_opening::an_ordinary_start_draws_nothing_that_is_then_removed",
        store_opening::an_ordinary_start_draws_nothing_that_is_then_removed as fn(),
    ),
    (
        "visible_window::opening_and_switching_large_folders_asks_a_bounded_number_of_pages",
        visible_window::opening_and_switching_large_folders_asks_a_bounded_number_of_pages
            as fn(),
    ),
    (
        "archive_frames::archiving_the_cursors_row_draws_no_frame_unlike_the_settled_one",
        archive_frames::archiving_the_cursors_row_draws_no_frame_unlike_the_settled_one as fn(),
    ),
    (
        "hit_composer::replying_from_a_hit_leaves_the_composer_clear_of_the_search_bar",
        hit_composer::replying_from_a_hit_leaves_the_composer_clear_of_the_search_bar as fn(),
    ),
    (
        "body_lands::a_body_that_lands_while_its_message_is_open_is_drawn_in_place",
        body_lands::a_body_that_lands_while_its_message_is_open_is_drawn_in_place as fn(),
    ),
    (
        "body_lands::a_body_the_server_refuses_stops_saying_it_is_downloading",
        body_lands::a_body_the_server_refuses_stops_saying_it_is_downloading as fn(),
    ),
    (
        "list_reload::a_batch_arriving_mid_sync_leaves_the_cursor_and_the_selection_alone",
        list_reload::a_batch_arriving_mid_sync_leaves_the_cursor_and_the_selection_alone as fn(),
    ),
    (
        "list_reload::a_reordering_sync_leaves_the_cursor_on_the_same_message",
        list_reload::a_reordering_sync_leaves_the_cursor_on_the_same_message as fn(),
    ),
    (
        "add_account_running::the_add_account_key_opens_a_blank_form_over_the_running_window",
        add_account_running::the_add_account_key_opens_a_blank_form_over_the_running_window
            as fn(),
    ),
    (
        "add_account_running::the_add_account_dialog_is_as_tall_as_its_form",
        add_account_running::the_add_account_dialog_is_as_tall_as_its_form as fn(),
    ),
    (
        "compose_detach::a_key_typed_in_a_detached_composition_lands_in_it",
        compose_detach::a_key_typed_in_a_detached_composition_lands_in_it as fn(),
    ),
    (
        "compose_detach::the_detach_command_moves_the_open_composer_to_a_window_and_back",
        compose_detach::the_detach_command_moves_the_open_composer_to_a_window_and_back as fn(),
    ),
    (
        "compose_detach::a_detached_composer_window_holds_the_keyboard",
        compose_detach::a_detached_composer_window_holds_the_keyboard as fn(),
    ),
    (
        "compose_detach::the_window_warms_its_editing_surface_without_being_asked_to_compose",
        compose_detach::the_window_warms_its_editing_surface_without_being_asked_to_compose
            as fn(),
    ),
    (
        "compose_detach::a_warm_ask_made_before_the_composer_is_mounted_still_warms_it",
        compose_detach::a_warm_ask_made_before_the_composer_is_mounted_still_warms_it as fn(),
    ),
    (
        "hostile_mail::each_hostile_message_opens_and_the_app_keeps_answering",
        hostile_mail::each_hostile_message_opens_and_the_app_keeps_answering as fn(),
    ),
    (
        "reader_spawns_no_web_process::ten_conversations_start_no_web_process_and_hold_what_one_holds",
        reader_spawns_no_web_process::ten_conversations_start_no_web_process_and_hold_what_one_holds
            as fn(),
    ),
    (
        "compose_layout::the_header_is_detach_title_close_and_the_verbs_have_a_row_of_their_own",
        compose_layout::the_header_is_detach_title_close_and_the_verbs_have_a_row_of_their_own
            as fn(),
    ),
    (
        "compose_layout::send_is_the_one_primary_and_nothing_wears_the_accent",
        compose_layout::send_is_the_one_primary_and_nothing_wears_the_accent as fn(),
    ),
    (
        "compose_layout::the_keycaps_are_short",
        compose_layout::the_keycaps_are_short as fn(),
    ),
    (
        "compose_layout::the_field_rows_share_their_edges",
        compose_layout::the_field_rows_share_their_edges as fn(),
    ),
    (
        "compose_layout::the_editor_is_drawn_on_the_dialogs_surface",
        compose_layout::the_editor_is_drawn_on_the_dialogs_surface as fn(),
    ),
    (
        "compose_layout::the_field_holding_the_keyboard_shows_it",
        compose_layout::the_field_holding_the_keyboard_shows_it as fn(),
    ),
    (
        "compose_layout::the_close_is_the_shared_x",
        compose_layout::the_close_is_the_shared_x as fn(),
    ),
    (
        "compose_layout::the_composer_column_is_the_readers_at_900",
        compose_layout::the_composer_column_is_the_readers_at_900 as fn(),
    ),
    (
        "compose_layout::the_composer_column_is_the_readers_at_the_screens_width",
        compose_layout::the_composer_column_is_the_readers_at_the_screens_width as fn(),
    ),
    (
        "compose_layout::the_composer_column_is_the_readers_at_1024",
        compose_layout::the_composer_column_is_the_readers_at_1024 as fn(),
    ),
    (
        "compose_layout::the_dialog_follows_the_message_dialogs_size_rule",
        compose_layout::the_dialog_follows_the_message_dialogs_size_rule as fn(),
    ),
    (
        "dropdown::escape_gives_the_keyboard_back_to_the_cursor_row",
        dropdown::escape_gives_the_keyboard_back_to_the_cursor_row as fn(),
    ),
    (
        "dropdown::a_ghost_drawn_before_the_bar_is_laid_out_shows_once_it_is",
        dropdown::a_ghost_drawn_before_the_bar_is_laid_out_shows_once_it_is as fn(),
    ),
    (
        "dropdown::a_prefix_draws_its_ghost_after_the_caret_and_tab_takes_it",
        dropdown::a_prefix_draws_its_ghost_after_the_caret_and_tab_takes_it as fn(),
    ),
    (
        "dropdown::the_words_state_draws_hits_pills_and_show_all",
        dropdown::the_words_state_draws_hits_pills_and_show_all as fn(),
    ),
    (
        "dropdown::slash_opens_the_dropdown_and_a_word_finds_top_hits",
        dropdown::slash_opens_the_dropdown_and_a_word_finds_top_hits as fn(),
    ),
    (
        "results::a_click_away_from_a_popover_keeps_nothing",
        results::a_click_away_from_a_popover_keeps_nothing as fn(),
    ),
    (
        "results::a_drag_across_the_timeline_narrows_the_dates",
        results::a_drag_across_the_timeline_narrows_the_dates as fn(),
    ),
    (
        "results_files::the_files_tab_saves_a_file_where_the_person_says",
        results_files::the_files_tab_saves_a_file_where_the_person_says as fn(),
    ),
    (
        "results::ctrl_s_opens_the_save_popover_and_return_saves",
        results::ctrl_s_opens_the_save_popover_and_return_saves as fn(),
    ),
    (
        "results::escape_closes_a_dropdown_over_the_results_then_leaves_them",
        results::escape_closes_a_dropdown_over_the_results_then_leaves_them as fn(),
    ),
    (
        "results::checking_and_walking_the_results_keeps_the_keyboard_in_their_list",
        results::checking_and_walking_the_results_keeps_the_keyboard_in_their_list as fn(),
    ),
    (
        "results::the_tabs_hold_still_when_the_sort_changes",
        results::the_tabs_hold_still_when_the_sort_changes as fn(),
    ),
    (
        "results::show_all_opens_the_results_and_back_returns_to_the_inbox",
        results::show_all_opens_the_results_and_back_returns_to_the_inbox as fn(),
    ),
    (
        "results::an_unread_result_has_the_inboxs_accent_dot",
        results::an_unread_result_has_the_inboxs_accent_dot as fn(),
    ),
    (
        "results::the_results_open_on_their_first_heading",
        results::the_results_open_on_their_first_heading as fn(),
    ),
    (
        "results::quick_look_stays_put_while_j_walks",
        results::quick_look_stays_put_while_j_walks as fn(),
    ),
    (
        "bar::a_hit_steps_the_results_and_its_own_thread",
        bar::a_hit_steps_the_results_and_its_own_thread as fn(),
    ),
    (
        "bar::ctrl_k_opens_the_bar_for_commands_and_slash_for_search",
        bar::ctrl_k_opens_the_bar_for_commands_and_slash_for_search as fn(),
    ),
    (
        "bar::the_bar_opens_in_the_top_bars_field",
        bar::the_bar_opens_in_the_top_bars_field as fn(),
    ),
    (
        "first_run::an_inbox_whose_first_pass_is_running_says_syncing_not_empty",
        first_run::an_inbox_whose_first_pass_is_running_says_syncing_not_empty as fn(),
    ),
    (
        "first_run::an_inbox_a_finished_pass_found_empty_says_empty_and_when",
        first_run::an_inbox_a_finished_pass_found_empty_says_empty_and_when as fn(),
    ),
    (
        "open_message::delete_in_the_dialog_deletes_the_message_on_screen",
        open_message::delete_in_the_dialog_deletes_the_message_on_screen as fn(),
    ),
    (
        "selection::the_bulk_bar_has_a_delete_button_that_deletes_the_selection",
        selection::the_bulk_bar_has_a_delete_button_that_deletes_the_selection as fn(),
    ),
    (
        "first_run::the_wizard_opens_large_enough_for_the_server_details",
        first_run::the_wizard_opens_large_enough_for_the_server_details as fn(),
    ),
    (
        "first_run::a_click_inside_the_form_is_the_forms_and_keeps_the_window_open",
        first_run::a_click_inside_the_form_is_the_forms_and_keeps_the_window_open as fn(),
    ),
    (
        "first_run::the_window_close_button_closes_the_app_with_the_form_open",
        first_run::the_window_close_button_closes_the_app_with_the_form_open as fn(),
    ),
    (
        "buttons::the_open_message_toolbar_is_compact_with_its_keycaps_inside",
        buttons::the_open_message_toolbar_is_compact_with_its_keycaps_inside as fn(),
    ),
    (
        "buttons::the_compose_dialog_draws_every_button_one_way",
        buttons::the_compose_dialog_draws_every_button_one_way as fn(),
    ),
    (
        "a11y::a_row_announces_its_marker_and_every_keycap_is_its_control_s_shortcut",
        a11y::a_row_announces_its_marker_and_every_keycap_is_its_control_s_shortcut as fn(),
    ),
    (
        "first_run::first_run_with_no_account_opens_the_add_account_form_and_lists_the_inbox",
        first_run::first_run_with_no_account_opens_the_add_account_form_and_lists_the_inbox
            as fn(),
    ),
    (
        "motion::no_transition_outruns_the_budget_and_reduced_motion_stills_them",
        motion::no_transition_outruns_the_budget_and_reduced_motion_stills_them as fn(),
    ),
    (
        "motion::the_list_rows_cursor_has_no_transition",
        motion::the_list_rows_cursor_has_no_transition as fn(),
    ),
    (
        "compose::reply_all_from_the_open_message_answers_it_and_esc_returns_to_it",
        compose::reply_all_from_the_open_message_answers_it_and_esc_returns_to_it as fn(),
    ),
    (
        "compose::a_send_says_it_was_queued_and_undo_takes_it_back",
        compose::a_send_says_it_was_queued_and_undo_takes_it_back as fn(),
    ),
    (
        "compose::the_heading_key_and_its_markdown_make_a_heading_the_toolbar_shows",
        compose::the_heading_key_and_its_markdown_make_a_heading_the_toolbar_shows as fn(),
    ),
    (
        "invitations::the_open_invitation_s_card_answers_with_its_keys",
        invitations::the_open_invitation_s_card_answers_with_its_keys as fn(),
    ),
    (
        "drafts::g_t_lists_drafts_and_enter_opens_one_to_edit",
        drafts::g_t_lists_drafts_and_enter_opens_one_to_edit as fn(),
    ),
    (
        "bar::the_fields_operator_terms_are_chips",
        bar::the_fields_operator_terms_are_chips as fn(),
    ),
    (
        "bar::offline_search_answers_locally_one_request_a_keystroke",
        bar::offline_search_answers_locally_one_request_a_keystroke as fn(),
    ),
    (
        "pickers::m_moves_three_to_receipts_and_ctrl_z_returns_them",
        pickers::m_moves_three_to_receipts_and_ctrl_z_returns_them as fn(),
    ),
    (
        "surfaced::a_delivered_digest_is_one_row_where_it_came_due",
        surfaced::a_delivered_digest_is_one_row_where_it_came_due as fn(),
    ),
    (
        "surfaced::a_fired_reminder_is_a_no_reply_row_at_its_place_and_listed_once",
        surfaced::a_fired_reminder_is_a_no_reply_row_at_its_place_and_listed_once as fn(),
    ),
    (
        "capture::t_appends_one_tasks_line_with_the_link_before_the_date",
        capture::t_appends_one_tasks_line_with_the_link_before_the_date as fn(),
    ),
    (
        "desktop::a_postio_link_opens_its_message_and_an_unknown_one_is_refused",
        desktop::a_postio_link_opens_its_message_and_an_unknown_one_is_refused as fn(),
    ),
    (
        "filtered::g_f_lists_filtered_mail_and_its_number_keys_narrow_it",
        filtered::g_f_lists_filtered_mail_and_its_number_keys_narrow_it as fn(),
    ),
    (
        "digest::enter_opens_a_digest_and_e_archives_all_of_it",
        digest::enter_opens_a_digest_and_e_archives_all_of_it as fn(),
    ),
    (
        "digest_summary::a_digest_with_a_summary_opens_on_it_and_every_statement_cites_its_mail",
        digest_summary::a_digest_with_a_summary_opens_on_it_and_every_statement_cites_its_mail
            as fn(),
    ),
    (
        "rule_query::match_a_list_or_a_search_instead_previews_and_saves_the_typed_query",
        rule_query::match_a_list_or_a_search_instead_previews_and_saves_the_typed_query as fn(),
    ),
    (
        "desktop::a_mailto_link_opens_a_composer_with_its_fields_filled",
        desktop::a_mailto_link_opens_a_composer_with_its_fields_filled as fn(),
    ),
    (
        "desktop::a_mailto_link_that_arrives_before_the_store_waits_for_it",
        desktop::a_mailto_link_that_arrives_before_the_store_waits_for_it as fn(),
    ),
    (
        "window_state::a_restarted_window_opens_at_the_size_it_was_closed_at",
        window_state::a_restarted_window_opens_at_the_size_it_was_closed_at as fn(),
    ),
    (
        "window_state::a_state_file_that_cannot_be_read_opens_at_the_default",
        window_state::a_state_file_that_cannot_be_read_opens_at_the_default as fn(),
    ),
    (
        "desktop::focus_shows_the_postio_icon",
        desktop::focus_shows_the_postio_icon as fn(),
    ),
    (
        "desktop::focus_says_which_application_it_is",
        desktop::focus_says_which_application_it_is as fn(),
    ),
    (
        "desktop::the_window_is_titled_postio",
        desktop::the_window_is_titled_postio as fn(),
    ),
    (
        "places::the_folders_popover_quiets_the_lists_ring",
        places::the_folders_popover_quiets_the_lists_ring as fn(),
    ),
    (
        "places::flagged_and_snoozed_are_listed_and_open_their_views",
        places::flagged_and_snoozed_are_listed_and_open_their_views as fn(),
    ),
    (
        "places::every_place_shows_its_count",
        places::every_place_shows_its_count as fn(),
    ),
    (
        "keycaps::the_undo_toast_names_the_key_that_undoes",
        keycaps::the_undo_toast_names_the_key_that_undoes as fn(),
    ),
    (
        "keycaps::the_offline_banner_names_the_key_that_retries",
        keycaps::the_offline_banner_names_the_key_that_retries as fn(),
    ),
    (
        "keycaps::try_again_names_the_key_that_runs_it",
        keycaps::try_again_names_the_key_that_runs_it as fn(),
    ),
    (
        "keycaps::settings_add_account_names_its_key",
        keycaps::settings_add_account_names_its_key as fn(),
    ),
    (
        "bar::walking_the_results_keeps_the_ringed_row_in_view",
        bar::walking_the_results_keeps_the_ringed_row_in_view as fn(),
    ),
    (
        "bar::a_result_gives_its_subject_room_before_its_first_line",
        bar::a_result_gives_its_subject_room_before_its_first_line as fn(),
    ),
    (
        "state::a_failing_account_is_named_and_the_others_mail_stays_listed",
        state::a_failing_account_is_named_and_the_others_mail_stays_listed as fn(),
    ),
    (
        "marker_card::the_highlight_starts_and_ends_on_the_sentences_own_words",
        marker_card::the_highlight_starts_and_ends_on_the_sentences_own_words as fn(),
    ),
    (
        "marker_card::a_notice_s_card_draws_its_chip_in_the_secondary_ink",
        marker_card::a_notice_s_card_draws_its_chip_in_the_secondary_ink as fn(),
    ),
    (
        "marker_card::the_card_dismisses_its_marker",
        marker_card::the_card_dismisses_its_marker as fn(),
    ),
    (
        "marker_card::a_marked_message_opens_with_its_card_and_its_sentence_highlighted",
        marker_card::a_marked_message_opens_with_its_card_and_its_sentence_highlighted as fn(),
    ),
    (
        "treatments::a_newsletter_opens_on_paper_and_o_switches_it_to_app_colours",
        treatments::a_newsletter_opens_on_paper_and_o_switches_it_to_app_colours as fn(),
    ),
    (
        "treatments::opening_and_stepping_prepare_no_body_on_the_interface_thread",
        treatments::opening_and_stepping_prepare_no_body_on_the_interface_thread as fn(),
    ),
    (
        "open_layout::the_render_mode_line_sits_24_under_the_card_and_12_over_the_body",
        open_layout::the_render_mode_line_sits_24_under_the_card_and_12_over_the_body as fn(),
    ),
    (
        "open_layout::every_band_starts_at_the_messages_left_edge",
        open_layout::every_band_starts_at_the_messages_left_edge as fn(),
    ),
    (
        "open_layout::the_window_opens_with_its_header_in_view_and_the_sentence_marked",
        open_layout::the_window_opens_with_its_header_in_view_and_the_sentence_marked as fn(),
    ),
    (
        "open_layout::the_action_cards_sentence_wraps_and_is_never_cut",
        open_layout::the_action_cards_sentence_wraps_and_is_never_cut as fn(),
    ),
    (
        "open_layout::the_senders_address_is_never_cut_while_there_is_room",
        open_layout::the_senders_address_is_never_cut_while_there_is_room as fn(),
    ),
    (
        "open_layout::delete_s_cap_reads_its_gmail_key_in_the_action_row",
        open_layout::delete_s_cap_reads_its_gmail_key_in_the_action_row as fn(),
    ),
    (
        "open_layout::a_page_on_paper_is_zoomed_to_its_column_and_scrolls_sideways_below_the_floor",
        open_layout::a_page_on_paper_is_zoomed_to_its_column_and_scrolls_sideways_below_the_floor
            as fn(),
    ),
    (
        "open_layout::the_dialogs_chrome_is_set_in_the_system_faces",
        open_layout::the_dialogs_chrome_is_set_in_the_system_faces as fn(),
    ),
    (
        "open_measure::a_plain_list_draws_as_a_list_with_the_handoffs_rhythm",
        open_measure::a_plain_list_draws_as_a_list_with_the_handoffs_rhythm as fn(),
    ),
    (
        "open_measure::the_attribution_sits_between_the_sign_off_and_the_toggle",
        open_measure::the_attribution_sits_between_the_sign_off_and_the_toggle as fn(),
    ),
    (
        "open_measure::the_body_ends_at_its_last_line",
        open_measure::the_body_ends_at_its_last_line as fn(),
    ),
    (
        "remote_images::remote_images_stay_blocked_scripts_go_and_nothing_is_asked_for",
        remote_images::remote_images_stay_blocked_scripts_go_and_nothing_is_asked_for as fn(),
    ),
    (
        "open_choice::o_offers_the_links_and_parts_and_opens_only_what_is_chosen",
        open_choice::o_offers_the_links_and_parts_and_opens_only_what_is_chosen as fn(),
    ),
    (
        "view_source::v_shows_the_raw_message_from_the_list_and_from_the_open_message",
        view_source::v_shows_the_raw_message_from_the_list_and_from_the_open_message as fn(),
    ),
    (
        "open_message::a_message_opens_at_its_top_every_time",
        open_message::a_message_opens_at_its_top_every_time as fn(),
    ),
    (
        "open_message::arrows_and_paging_keys_scroll_the_message_not_the_list",
        open_message::arrows_and_paging_keys_scroll_the_message_not_the_list as fn(),
    ),
    (
        "open_message::j_and_k_step_with_the_keyboard_on_each_control_of_the_dialog",
        open_message::j_and_k_step_with_the_keyboard_on_each_control_of_the_dialog as fn(),
    ),
    (
        "open_keys::the_arrows_and_paging_keys_scroll_the_open_message_in_steps",
        open_keys::the_arrows_and_paging_keys_scroll_the_open_message_in_steps as fn(),
    ),
    (
        "every_command::every_bound_command_shows_or_is_a_listed_gap",
        every_command::every_bound_command_shows_or_is_a_listed_gap as fn(),
    ),
    (
        "storyboards::a_storyboard_plays_on_focus_and_writes_its_run",
        storyboards::a_storyboard_plays_on_focus_and_writes_its_run as fn(),
    ),
    (
        "storyboard_determinism::two_processes_film_a_storyboard_identically",
        storyboard_determinism::two_processes_film_a_storyboard_identically as fn(),
    ),
    (
        "storyboard_catalogue::the_catalogue_holds_on_focus",
        storyboard_catalogue::the_catalogue_holds_on_focus as fn(),
    ),
    (
        "observe::the_window_says_where_the_keyboard_cursor_and_notices_are",
        observe::the_window_says_where_the_keyboard_cursor_and_notices_are as fn(),
    ),
    (
        "open_keys::the_keyboard_lands_on_the_message_each_time_it_opens",
        open_keys::the_keyboard_lands_on_the_message_each_time_it_opens as fn(),
    ),
    (
        "open_keys::a_toast_raised_under_the_open_message_is_drawn_over_it",
        open_keys::a_toast_raised_under_the_open_message_is_drawn_over_it as fn(),
    ),
    (
        "open_reading::ctrl_c_copies_what_was_dragged_across_the_body",
        open_reading::ctrl_c_copies_what_was_dragged_across_the_body as fn(),
    ),
    (
        "open_reading::the_open_message_is_one_scrolling_column",
        open_reading::the_open_message_is_one_scrolling_column as fn(),
    ),
    (
        "open_reading::opening_the_quoted_fold_grows_the_body_and_shows_the_quote",
        open_reading::opening_the_quoted_fold_grows_the_body_and_shows_the_quote as fn(),
    ),
    (
        "open_reading::the_column_draws_only_the_window_of_a_long_body",
        open_reading::the_column_draws_only_the_window_of_a_long_body as fn(),
    ),
    (
        "open_reading::a_plain_body_has_no_frame_and_a_page_of_its_own_keeps_one",
        open_reading::a_plain_body_has_no_frame_and_a_page_of_its_own_keeps_one as fn(),
    ),
    (
        "open_reading::the_header_card_names_from_to_and_cc_and_dates_today_relatively",
        open_reading::the_header_card_names_from_to_and_cc_and_dates_today_relatively as fn(),
    ),
    (
        "open_reading::the_header_card_has_no_cc_line_without_cc_and_dates_the_past_in_full",
        open_reading::the_header_card_has_no_cc_line_without_cc_and_dates_the_past_in_full
            as fn(),
    ),
    (
        "open_reading::an_attachment_is_a_card_under_the_body",
        open_reading::an_attachment_is_a_card_under_the_body as fn(),
    ),
    (
        "open_reading::a_label_pill_carries_its_colour_dot",
        open_reading::a_label_pill_carries_its_colour_dot as fn(),
    ),
    (
        "open_reading::close_is_the_header_s_square_and_the_verbs_are_30px",
        open_reading::close_is_the_header_s_square_and_the_verbs_are_30px as fn(),
    ),
    (
        "pointer_pairs::a_double_click_on_a_row_opens_its_message_as_enter_does",
        pointer_pairs::a_double_click_on_a_row_opens_its_message_as_enter_does as fn(),
    ),
    (
        "pointer_pairs::ctrl_click_toggles_a_rows_selection",
        pointer_pairs::ctrl_click_toggles_a_rows_selection as fn(),
    ),
    (
        "pointer_pairs::shift_click_extends_the_selection_to_a_range",
        pointer_pairs::shift_click_extends_the_selection_to_a_range as fn(),
    ),
    (
        "pointer_pairs::a_press_in_a_rows_gutter_toggles_its_selection",
        pointer_pairs::a_press_in_a_rows_gutter_toggles_its_selection as fn(),
    ),
    (
        "pointer_pairs::escape_closes_the_message_and_the_key_map",
        pointer_pairs::escape_closes_the_message_and_the_key_map as fn(),
    ),
    (
        "row_menu::a_right_click_on_a_row_offers_its_verbs_with_their_keys",
        row_menu::a_right_click_on_a_row_offers_its_verbs_with_their_keys as fn(),
    ),
    (
        "row_menu::a_right_click_outside_the_selection_is_for_that_row_inside_it_for_the_selection",
        row_menu::a_right_click_outside_the_selection_is_for_that_row_inside_it_for_the_selection
            as fn(),
    ),
    (
        "window_controls::the_top_bars_icon_buttons_hover_in_their_own_shape",
        window_controls::the_top_bars_icon_buttons_hover_in_their_own_shape as fn(),
    ),
    (
        "window_controls::the_inbox_top_bar_is_a_window_handle",
        window_controls::the_inbox_top_bar_is_a_window_handle as fn(),
    ),
    (
        "window_controls::maximize_and_restore_work_before_and_after_mail_opens",
        window_controls::maximize_and_restore_work_before_and_after_mail_opens as fn(),
    ),
    (
        "close_buttons::every_closable_surface_has_the_same_x_at_the_right",
        close_buttons::every_closable_surface_has_the_same_x_at_the_right as fn(),
    ),
    (
        "close_buttons::add_account_opens_with_the_keyboard_in_its_first_field",
        close_buttons::add_account_opens_with_the_keyboard_in_its_first_field as fn(),
    ),
    (
        "close_buttons::add_account_has_the_same_x_at_the_right_on_every_step",
        close_buttons::add_account_has_the_same_x_at_the_right_on_every_step as fn(),
    ),
    (
        "close_buttons::the_digest_has_the_same_x_at_the_right",
        close_buttons::the_digest_has_the_same_x_at_the_right as fn(),
    ),
    (
        "icon_buttons::every_surfaces_icon_buttons_keep_their_own_shape",
        icon_buttons::every_surfaces_icon_buttons_keep_their_own_shape as fn(),
    ),
    (
        "icon_buttons::the_digests_icon_buttons_keep_their_own_shape",
        icon_buttons::the_digests_icon_buttons_keep_their_own_shape as fn(),
    ),
    (
        "open_reading::close_is_an_x_icon_at_the_right_and_the_steps_at_the_left",
        open_reading::close_is_an_x_icon_at_the_right_and_the_steps_at_the_left as fn(),
    ),
    (
        "open_reading::the_steps_carry_their_keys_inside_and_stay_compact",
        open_reading::the_steps_carry_their_keys_inside_and_stay_compact as fn(),
    ),
    (
        "sending_states::an_unconfirmed_send_says_so_and_mark_as_sent_settles_it",
        sending_states::an_unconfirmed_send_says_so_and_mark_as_sent_settles_it as fn(),
    ),
    (
        "sending_states::received_mail_keeps_its_verbs_and_the_send_keys_say_why_not",
        sending_states::received_mail_keeps_its_verbs_and_the_send_keys_say_why_not as fn(),
    ),
    (
        "read_on_dwell::a_message_left_open_in_the_dialog_is_marked_read",
        read_on_dwell::a_message_left_open_in_the_dialog_is_marked_read as fn(),
    ),
    (
        "read_on_dwell::a_message_the_composer_replaced_is_not_marked_read",
        read_on_dwell::a_message_the_composer_replaced_is_not_marked_read as fn(),
    ),
    (
        "read_on_dwell::stepping_the_pane_past_a_message_leaves_it_unread",
        read_on_dwell::stepping_the_pane_past_a_message_leaves_it_unread as fn(),
    ),
    (
        "read_on_dwell::a_message_closed_before_the_dwell_stays_unread",
        read_on_dwell::a_message_closed_before_the_dwell_stays_unread as fn(),
    ),
    (
        "read_on_dwell::r_in_the_open_message_marks_it_unread_again",
        read_on_dwell::r_in_the_open_message_marks_it_unread_again as fn(),
    ),
    (
        "read_on_dwell::a_message_in_a_window_that_lost_focus_stays_unread",
        read_on_dwell::a_message_in_a_window_that_lost_focus_stays_unread as fn(),
    ),
    (
        "read_on_dwell::refocusing_does_not_take_back_r",
        read_on_dwell::refocusing_does_not_take_back_r as fn(),
    ),
    (
        "reading_pane::f8_and_the_setting_switch_between_the_dialog_and_the_pane",
        reading_pane::f8_and_the_setting_switch_between_the_dialog_and_the_pane as fn(),
    ),
    (
        "reading_pane::return_shows_the_message_beside_the_list_and_keeps_its_cursor",
        reading_pane::return_shows_the_message_beside_the_list_and_keeps_its_cursor as fn(),
    ),
    (
        "reading_pane::a_window_under_980_opens_the_dialog_instead",
        reading_pane::a_window_under_980_opens_the_dialog_instead as fn(),
    ),
    (
        "reading_pane::the_panes_column_follows_column_width_for_its_width",
        reading_pane::the_panes_column_follows_column_width_for_its_width as fn(),
    ),
    (
        "reading_pane::a_click_moves_the_cursor_and_the_open_pane_follows_it",
        reading_pane::a_click_moves_the_cursor_and_the_open_pane_follows_it as fn(),
    ),
    (
        "reading_pane::the_composer_takes_over_the_pane",
        reading_pane::the_composer_takes_over_the_pane as fn(),
    ),
    (
        "open_layout::the_dialog_is_sized_by_the_window_and_never_by_the_message",
        open_layout::the_dialog_is_sized_by_the_window_and_never_by_the_message as fn(),
    ),
    (
        "open_layout::recipient_addresses_keep_the_reader_centered",
        open_layout::recipient_addresses_keep_the_reader_centered as fn(),
    ),
    (
        "open_layout::the_list_behind_is_dimmed_by_black_at_20_and_45_percent",
        open_layout::the_list_behind_is_dimmed_by_black_at_20_and_45_percent as fn(),
    ),
    (
        "open_layout::every_block_shares_both_edges_of_one_centred_column",
        open_layout::every_block_shares_both_edges_of_one_centred_column as fn(),
    ),
    (
        "open_layout::the_blocks_keep_the_handoffs_rhythm",
        open_layout::the_blocks_keep_the_handoffs_rhythm as fn(),
    ),
    (
        "open_layout::an_absent_block_takes_its_gap_with_it",
        open_layout::an_absent_block_takes_its_gap_with_it as fn(),
    ),
    (
        "open_layout::the_dialog_wears_its_palette_in_light_and_dark",
        open_layout::the_dialog_wears_its_palette_in_light_and_dark as fn(),
    ),
    (
        "open_layout::a_narrow_dialog_folds_label_move_and_delete_into_more",
        open_layout::a_narrow_dialog_folds_label_move_and_delete_into_more as fn(),
    ),
    (
        "open_measure::a_plain_paragraph_break_is_a_short_gap_not_a_blank_line",
        open_measure::a_plain_paragraph_break_is_a_short_gap_not_a_blank_line as fn(),
    ),
    (
        "open_measure::the_measure_is_the_dialogs_width_less_its_gutters",
        open_measure::the_measure_is_the_dialogs_width_less_its_gutters as fn(),
    ),
    (
        "open_measure::opening_find_keeps_the_reading_position",
        open_measure::opening_find_keeps_the_reading_position as fn(),
    ),
    (
        "open_measure::the_columns_ground_is_the_dialogs_own_in_light_and_dark",
        open_measure::the_columns_ground_is_the_dialogs_own_in_light_and_dark as fn(),
    ),
    (
        "open_message::a_hundredth_open_builds_no_second_message_view",
        open_message::a_hundredth_open_builds_no_second_message_view as fn(),
    ),
    (
        "open_message::a_hundredth_open_of_a_thread_builds_no_second_message_view",
        open_message::a_hundredth_open_of_a_thread_builds_no_second_message_view as fn(),
    ),
    (
        "corrections::three_dismissals_write_a_stop_marker_to_focus_s_config",
        corrections::three_dismissals_write_a_stop_marker_to_focus_s_config as fn(),
    ),
    (
        "invitations::an_invitation_overlapping_a_local_event_says_what_it_clashes_with",
        invitations::an_invitation_overlapping_a_local_event_says_what_it_clashes_with as fn(),
    ),
    (
        "invitations::a_click_on_the_row_s_decline_declines",
        invitations::a_click_on_the_row_s_decline_declines as fn(),
    ),
    (
        "offline_send::offline_a_send_is_in_the_outbox_at_once_and_leaves_once",
        offline_send::offline_a_send_is_in_the_outbox_at_once_and_leaves_once as fn(),
    ),
    (
        "remind_on_send::mod_h_in_the_composer_sets_a_reminder_that_sending_keeps",
        remind_on_send::mod_h_in_the_composer_sets_a_reminder_that_sending_keeps as fn(),
    ),
    (
        "one_composer::the_same_content_from_either_composer_leaves_as_the_same_message",
        one_composer::the_same_content_from_either_composer_leaves_as_the_same_message as fn(),
    ),
    (
        "across_apps::what_focus_archives_the_terminal_sees_archived",
        across_apps::what_focus_archives_the_terminal_sees_archived as fn(),
    ),
    (
        "store_in_use::a_store_another_postio_has_open_is_refused_with_try_again_and_left_alone",
        store_in_use::a_store_another_postio_has_open_is_refused_with_try_again_and_left_alone
            as fn(),
    ),
    (
        "startup_timeline::the_timeline_reaches_the_frame_with_mail_in_it",
        startup_timeline::the_timeline_reaches_the_frame_with_mail_in_it as fn(),
    ),
    (
        "store_unavailable::the_close_button_closes_the_window_when_the_store_will_not_open",
        store_unavailable::the_close_button_closes_the_window_when_the_store_will_not_open
            as fn(),
    ),
    (
        "store_unavailable::ctrl_q_closes_the_window_when_the_store_will_not_open",
        store_unavailable::ctrl_q_closes_the_window_when_the_store_will_not_open as fn(),
    ),
    (
        "store_unavailable::ctrl_w_closes_the_window_when_the_store_will_not_open",
        store_unavailable::ctrl_w_closes_the_window_when_the_store_will_not_open as fn(),
    ),
    (
        "store_unavailable::a_store_no_migration_reaches_offers_a_fresh_store_that_keeps_the_account",
        store_unavailable::a_store_no_migration_reaches_offers_a_fresh_store_that_keeps_the_account
            as fn(),
    ),
    (
        "store_unavailable::a_store_under_another_key_offers_a_fresh_store_and_keeps_the_old_one",
        store_unavailable::a_store_under_another_key_offers_a_fresh_store_and_keeps_the_old_one
            as fn(),
    ),
    (
        "reload::a_saved_rebind_reaches_the_keyboard_every_keycap_and_the_key_map",
        reload::a_saved_rebind_reaches_the_keyboard_every_keycap_and_the_key_map as fn(),
    ),
    (
        "reload::a_saved_focus_section_reaches_the_empty_inbox",
        reload::a_saved_focus_section_reaches_the_empty_inbox as fn(),
    ),
    (
        "commands::star_flags_the_row_and_undo_clears_it",
        commands::star_flags_the_row_and_undo_clears_it as fn(),
    ),
    (
        "registry_parity::every_offered_command_that_is_whole_reaches_a_handler",
        registry_parity::every_offered_command_that_is_whole_reaches_a_handler as fn(),
    ),
    (
        "save_attachments::save_in_the_chooser_writes_the_part_where_the_person_chose",
        save_attachments::save_in_the_chooser_writes_the_part_where_the_person_chose as fn(),
    ),
    (
        "save_attachments::save_all_in_the_chooser_writes_every_part_into_the_chosen_folder",
        save_attachments::save_all_in_the_chooser_writes_every_part_into_the_chosen_folder
            as fn(),
    ),
    (
        "save_attachments::a_cancelled_portal_writes_nothing",
        save_attachments::a_cancelled_portal_writes_nothing as fn(),
    ),
    (
        "save_attachments::a_click_on_a_chip_shows_the_chooser_at_that_part",
        save_attachments::a_click_on_a_chip_shows_the_chooser_at_that_part as fn(),
    ),
    (
        "reader_zoom::the_zoom_keys_work_in_the_dialog_and_the_pane_and_the_level_is_kept",
        reader_zoom::the_zoom_keys_work_in_the_dialog_and_the_pane_and_the_level_is_kept as fn(),
    ),
    (
        "open_layout::paper_fit_multiplies_under_the_zoom",
        open_layout::paper_fit_multiplies_under_the_zoom as fn(),
    ),
    (
        "remote_images::show_fetches_once_and_asks_for_nothing_before",
        remote_images::show_fetches_once_and_asks_for_nothing_before as fn(),
    ),
    (
        "remote_images::always_holds_for_the_senders_next_message",
        remote_images::always_holds_for_the_senders_next_message as fn(),
    ),
    (
        "reload::a_saved_compose_section_reaches_the_next_reply",
        reload::a_saved_compose_section_reaches_the_next_reply as fn(),
    ),
    (
        "reload::a_saved_reader_zoom_reaches_the_open_message",
        reload::a_saved_reader_zoom_reaches_the_open_message as fn(),
    ),
    (
        "reload::editing_the_ceiling_live_evicts_a_running_stores_oldest_blobs",
        reload::editing_the_ceiling_live_evicts_a_running_stores_oldest_blobs as fn(),
    ),
    (
        "registry_parity::every_focus_command_has_a_key_a_bar_row_and_a_control",
        registry_parity::every_focus_command_has_a_key_a_bar_row_and_a_control as fn(),
    ),
    (
        "keymap::every_key_the_key_map_shows_runs_its_command",
        keymap::every_key_the_key_map_shows_runs_its_command as fn(),
    ),
    (
        "empty::with_filtering_the_empty_inbox_counts_what_was_filtered_today",
        empty::with_filtering_the_empty_inbox_counts_what_was_filtered_today as fn(),
    ),
    (
        "state::each_sync_state_shows_its_banner_and_label",
        state::each_sync_state_shows_its_banner_and_label as fn(),
    ),
    (
        "state::update_password_on_the_sign_in_banner_opens_the_credential_dialog",
        state::update_password_on_the_sign_in_banner_opens_the_credential_dialog as fn(),
    ),
    (
        "rows::the_inbox_opens_with_its_first_heading_on_screen",
        rows::the_inbox_opens_with_its_first_heading_on_screen as fn(),
    ),
    (
        "shot::screen_01_is_written_as_a_png",
        shot::screen_01_is_written_as_a_png as fn(),
    ),
    (
        "shot::an_unknown_screen_writes_nothing_and_says_so",
        shot::an_unknown_screen_writes_nothing_and_says_so as fn(),
    ),
    (
        "shot::the_demo_inbox_opens_with_its_first_heading_on_screen",
        shot::the_demo_inbox_opens_with_its_first_heading_on_screen as fn(),
    ),
    (
        "list_contract::the_list_output_stays_libtest_shaped",
        list_contract::the_list_output_stays_libtest_shaped as fn(),
    ),
    ("harness::an_empty_case", harness::an_empty_case as fn()),
    (
        "harness::settle_outlasts_the_runtimes_budget",
        harness::settle_outlasts_the_runtimes_budget as fn(),
    ),
    (
        "harness::an_empty_case_is_listed_and_runs",
        harness::an_empty_case_is_listed_and_runs as fn(),
    ),
    (
        "chrome::the_top_bar_and_the_header_strip_carry_each_control_and_its_key",
        chrome::the_top_bar_and_the_header_strip_carry_each_control_and_its_key as fn(),
    ),
    (
        "keyboard_home::the_window_opens_with_the_keyboard_in_the_list",
        keyboard_home::the_window_opens_with_the_keyboard_in_the_list as fn(),
    ),
    (
        "keyboard_home::escape_from_the_bar_returns_the_keyboard_to_the_list",
        keyboard_home::escape_from_the_bar_returns_the_keyboard_to_the_list as fn(),
    ),
    (
        "keyboard_home::going_to_a_place_leaves_the_keyboard_in_the_list",
        keyboard_home::going_to_a_place_leaves_the_keyboard_in_the_list as fn(),
    ),
    (
        "keyboard_home::going_to_an_empty_place_rests_the_keyboard_on_its_page",
        keyboard_home::going_to_an_empty_place_rests_the_keyboard_on_its_page as fn(),
    ),
    (
        "keyboard_home::an_empty_inbox_at_launch_rests_the_keyboard_on_its_page",
        keyboard_home::an_empty_inbox_at_launch_rests_the_keyboard_on_its_page as fn(),
    ),
    (
        "keyboard_home::the_keyboard_follows_the_cursor_row",
        keyboard_home::the_keyboard_follows_the_cursor_row as fn(),
    ),
    (
        "keyboard_home::a_list_that_opens_takes_the_keyboard_from_a_row_to_the_first",
        keyboard_home::a_list_that_opens_takes_the_keyboard_from_a_row_to_the_first as fn(),
    ),
    (
        "place_strip::an_empty_snoozed_place_says_why_and_the_way_back",
        place_strip::an_empty_snoozed_place_says_why_and_the_way_back as fn(),
    ),
    (
        "rows::a_selected_digest_row_is_drawn_selected",
        rows::a_selected_digest_row_is_drawn_selected as fn(),
    ),
    (
        "threads::right_opens_a_thread_in_place",
        threads::right_opens_a_thread_in_place as fn(),
    ),
    (
        "threads::j_walks_the_children_and_on_to_the_next_thread",
        threads::j_walks_the_children_and_on_to_the_next_thread as fn(),
    ),
    (
        "threads::clicking_the_chevron_opens_and_folds_a_thread",
        threads::clicking_the_chevron_opens_and_folds_a_thread as fn(),
    ),
    (
        "threads::the_window_walks_its_thread",
        threads::the_window_walks_its_thread as fn(),
    ),
    (
        "threads::clicking_a_thread_bar_chip_shows_that_message",
        threads::clicking_a_thread_bar_chip_shows_that_message as fn(),
    ),
    (
        "threads::clicking_the_more_chip_opens_the_outline",
        threads::clicking_the_more_chip_opens_the_outline as fn(),
    ),
    (
        "threads::a_search_childs_row_says_who_wrote_it_and_when",
        threads::a_search_childs_row_says_who_wrote_it_and_when as fn(),
    ),
    (
        "threads::the_forward_chooser_answers_its_number_keys_only_while_it_has_the_keyboard",
        threads::the_forward_chooser_answers_its_number_keys_only_while_it_has_the_keyboard
            as fn(),
    ),
    (
        "cursor::the_list_view_moving_the_cursor_is_where_j_walks_on_from",
        cursor::the_list_view_moving_the_cursor_is_where_j_walks_on_from as fn(),
    ),
    (
        "reading_pane::a_sweep_reads_the_row_it_lands_on_not_every_row_it_passes",
        reading_pane::a_sweep_reads_the_row_it_lands_on_not_every_row_it_passes as fn(),
    ),
    (
        "reading_pane::opening_a_conversation_costs_the_same_however_long_it_is",
        reading_pane::opening_a_conversation_costs_the_same_however_long_it_is as fn(),
    ),
    (
        "colours::the_roles_resolve_and_follow_the_system_into_dark",
        colours::the_roles_resolve_and_follow_the_system_into_dark as fn(),
    ),
    (
        "colours::the_rows_repaint_in_dark_at_once_and_the_marker_keeps_the_accent",
        colours::the_rows_repaint_in_dark_at_once_and_the_marker_keeps_the_accent as fn(),
    ),
    (
        "colours::a_notice_s_to_do_is_drawn_in_the_secondary_ink_and_says_whose_it_is",
        colours::a_notice_s_to_do_is_drawn_in_the_secondary_ink_and_says_whose_it_is as fn(),
    ),
    (
        "marked_rows::a_marked_row_is_two_lines_whatever_its_state",
        marked_rows::a_marked_row_is_two_lines_whatever_its_state as fn(),
    ),
    (
        "marked_rows::a_selected_marked_row_shows_its_check_not_its_dot",
        marked_rows::a_selected_marked_row_shows_its_check_not_its_dot as fn(),
    ),
    (
        "marked_rows::the_dot_is_unread_mail_not_a_marker",
        marked_rows::the_dot_is_unread_mail_not_a_marker as fn(),
    ),
    (
        "rows::a_row_shows_the_subject_and_first_line_exactly_as_they_arrived",
        rows::a_row_shows_the_subject_and_first_line_exactly_as_they_arrived as fn(),
    ),
    (
        "rows::a_third_label_draws_no_third_pill_and_none_is_the_accent",
        rows::a_third_label_draws_no_third_pill_and_none_is_the_accent as fn(),
    ),
    (
        "rows::rows_sit_under_their_day_s_heading",
        rows::rows_sit_under_their_day_s_heading as fn(),
    ),
    (
        "starts_offline::the_inbox_is_listed_from_the_store_with_no_network",
        starts_offline::the_inbox_is_listed_from_the_store_with_no_network as fn(),
    ),
    (
        "newsletter_seed::the_newsletters_row_says_nothing_of_the_message_it_was_made_from",
        newsletter_seed::the_newsletters_row_says_nothing_of_the_message_it_was_made_from as fn(),
    ),
    (
        "visible_window::a_row_whose_page_has_not_landed_draws_a_skeleton",
        visible_window::a_row_whose_page_has_not_landed_draws_a_skeleton as fn(),
    ),
    (
        "visible_window::a_jump_to_the_bottom_reads_the_ends_and_nothing_between",
        visible_window::a_jump_to_the_bottom_reads_the_ends_and_nothing_between as fn(),
    ),
    (
        "settings::mod_comma_and_the_menu_open_settings_in_focuss_frame",
        settings::mod_comma_and_the_menu_open_settings_in_focuss_frame as fn(),
    ),
    (
        "settings::every_section_focus_shows_is_reachable_and_appearance_is_not",
        settings::every_section_focus_shows_is_reachable_and_appearance_is_not as fn(),
    ),
    (
        "settings::the_signature_editor_has_the_keyboard_and_says_what_it_is",
        settings::the_signature_editor_has_the_keyboard_and_says_what_it_is as fn(),
    ),
    (
        "settings::a_signature_made_in_settings_signs_the_next_message",
        settings::a_signature_made_in_settings_signs_the_next_message as fn(),
    ),
    (
        "settings::agents_in_settings_shows_the_log_and_revokes_from_the_file",
        settings::agents_in_settings_shows_the_log_and_revokes_from_the_file as fn(),
    ),
    (
        "drag_out::a_dragged_row_hands_a_file_manager_its_message_as_an_eml",
        drag_out::a_dragged_row_hands_a_file_manager_its_message_as_an_eml as fn(),
    ),
    (
        "drag_out::a_dragged_selection_is_every_selected_message_and_nothing_is_written_early",
        drag_out::a_dragged_selection_is_every_selected_message_and_nothing_is_written_early
            as fn(),
    ),
    (
        "drag_out::a_select_all_drag_offers_no_files",
        drag_out::a_select_all_drag_offers_no_files as fn(),
    ),
    (
        "drag_out::a_dragged_row_offers_the_portal_spelling_too",
        drag_out::a_dragged_row_offers_the_portal_spelling_too as fn(),
    ),
    (
        "drag_out::a_dragged_message_survives_the_portal",
        drag_out::a_dragged_message_survives_the_portal as fn(),
    ),
    (
        "settings_wiring::an_account_verb_leaves_the_keyboard_on_its_row",
        settings_wiring::an_account_verb_leaves_the_keyboard_on_its_row as fn(),
    ),
    (
        "settings_wiring::account_rows_persist_enable_default_and_removal",
        settings_wiring::account_rows_persist_enable_default_and_removal as fn(),
    ),
    (
        "settings_wiring::update_credential_opens_a_prefilled_form_over_the_window",
        settings_wiring::update_credential_opens_a_prefilled_form_over_the_window as fn(),
    ),
    (
        "settings_wiring::opening_makes_no_connection_and_privacy_lists_the_log",
        settings_wiring::opening_makes_no_connection_and_privacy_lists_the_log as fn(),
    ),
    (
        "close_buttons::settings_has_the_same_x_at_the_right",
        close_buttons::settings_has_the_same_x_at_the_right as fn(),
    ),
    (
        "icon_buttons::the_settings_icon_buttons_keep_their_own_shape",
        icon_buttons::the_settings_icon_buttons_keep_their_own_shape as fn(),
    ),
    (
        "settings::filtering_turned_off_in_settings_is_written_and_the_strip_follows",
        settings::filtering_turned_off_in_settings_is_written_and_the_strip_follows as fn(),
    ),
    (
        "settings::a_click_into_settings_as_it_opens_waits_for_it_to_open",
        settings::a_click_into_settings_as_it_opens_waits_for_it_to_open as fn(),
    ),
    (
        "gmail_keys::the_digest_opens_from_a_cursor_below_it",
        gmail_keys::the_digest_opens_from_a_cursor_below_it as fn(),
    ),
    (
        "gmail_keys::every_gmail_command_is_probed_or_excused",
        gmail_keys::every_gmail_command_is_probed_or_excused as fn(),
    ),
    (
        "gmail_keys::the_controls_teach_the_gmail_keys_and_no_old_ones",
        gmail_keys::the_controls_teach_the_gmail_keys_and_no_old_ones as fn(),
    ),
    (
        "gmail_keys::the_attachment_and_unsubscribe_controls_teach_their_keys",
        gmail_keys::the_attachment_and_unsubscribe_controls_teach_their_keys as fn(),
    ),
    (
        "gmail_keys::the_key_map_shows_the_gmail_keys_and_no_old_ones",
        gmail_keys::the_key_map_shows_the_gmail_keys_and_no_old_ones as fn(),
    ),
    (
        "gmail_keys::the_palette_shows_the_gmail_keys_and_no_old_ones",
        gmail_keys::the_palette_shows_the_gmail_keys_and_no_old_ones as fn(),
    ),
    (
        "gmail_keys::settings_keyboard_lists_the_gmail_keys",
        gmail_keys::settings_keyboard_lists_the_gmail_keys as fn(),
    ),
    (
        "gmail_keys::gmail_letters_type_in_the_composer_and_in_the_search_bar",
        gmail_keys::gmail_letters_type_in_the_composer_and_in_the_search_bar as fn(),
    ),
    (
        "gmail_keys::the_reader_names_the_key_that_checks_for_new_mail",
        gmail_keys::the_reader_names_the_key_that_checks_for_new_mail as fn(),
    ),
    (
        "gmail_keys::z_takes_back_a_removed_account_in_settings",
        gmail_keys::z_takes_back_a_removed_account_in_settings as fn(),
    ),
    (
        "forward_carried::a_forwarded_part_reads_downloading_until_its_bytes_land",
        forward_carried::a_forwarded_part_reads_downloading_until_its_bytes_land as fn(),
    ),
    (
        "input_method::a_focused_field_uses_gtks_simple_input_method",
        input_method::a_focused_field_uses_gtks_simple_input_method as fn(),
    ),
    (
        "open_layout::a_folded_thread_bar_keeps_its_height_at_a_narrow_window",
        open_layout::a_folded_thread_bar_keeps_its_height_at_a_narrow_window as fn(),
    ),
    (
        "footers::a_message_alone_has_its_own_footer",
        footers::a_message_alone_has_its_own_footer as fn(),
    ),
    (
        "footers::a_waiting_send_offers_what_settles_it",
        footers::a_waiting_send_offers_what_settles_it as fn(),
    ),
    (
        "footers::the_composer_has_no_footer",
        footers::the_composer_has_no_footer as fn(),
    ),
    (
        "footers::settings_has_no_footer",
        footers::settings_has_no_footer as fn(),
    ),
    (
        "footers::the_capture_sheet_has_no_footer",
        footers::the_capture_sheet_has_no_footer as fn(),
    ),
    (
        "footers::a_draft_row_offers_to_be_finished",
        footers::a_draft_row_offers_to_be_finished as fn(),
    ),
    (
        "footers::the_places_popover_has_no_footer",
        footers::the_places_popover_has_no_footer as fn(),
    ),
    (
        "footers::a_removal_in_the_window_has_the_next_messages_footer",
        footers::a_removal_in_the_window_has_the_next_messages_footer as fn(),
    ),
];

use gtk::glib;

/// The loop is turned inside one poll of the case's future, so tokio-awaiting futures spend that
/// poll's 128-operation budget;
/// once spent each wakes itself at once and `settle` never goes idle. The turn runs in
/// `block_in_place`, where tokio does not count.
pub fn settle() {
    let patience = postio_test_support::scaled(std::time::Duration::from_secs(10));
    tokio::task::block_in_place(|| {
        let started = std::time::Instant::now();
        while glib::MainContext::default().iteration(false) {
            assert!(
                started.elapsed() < patience,
                "the main loop never went idle: a future it polls is woken the moment \
                 it returns Pending"
            );
        }
    });
}

pub async fn settle_for(least: std::time::Duration) {
    let started = std::time::Instant::now();
    settle_until(async || started.elapsed() >= least).await;
}

/// Gives up after ten seconds (scaled by `POSTIO_TEST_PATIENCE`); returns whether it happened,
/// since call sites assert.
pub async fn settle_until<F, Fut>(done: F) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    settle_until_within(std::time::Duration::from_secs(10), done).await
}

/// Creating the encrypted file, migrating and building the search index takes about six seconds on
/// an idle workstation.
pub const STORE_OPEN: std::time::Duration = std::time::Duration::from_secs(60);

pub async fn settle_until_within<F, Fut>(base: std::time::Duration, done: F) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + postio_test_support::scaled(base);
    while std::time::Instant::now() < deadline {
        settle();
        if done().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    done().await
}

pub async fn settle_while<F, Fut>(held: F) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now()
        + postio_test_support::scaled(std::time::Duration::from_millis(500));
    while std::time::Instant::now() < deadline {
        settle();
        if !held().await {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    held().await
}

/// On this thread because everything touches GTK, on a multi-threaded runtime because a synchronous
/// store read (`block_in_place`) panics on a current-thread one.
pub fn gtk_case<F: std::future::Future<Output = ()>>(body: F) {
    thread_local! {
        static RUNTIME: tokio::runtime::Runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime for the Focus suite");
    }
    RUNTIME.with(|runtime| runtime.block_on(body));
}

/// Dropped outside every runtime, the one place a host's own runtime may be dropped.
fn tidy_up(name: &str) {
    use gtk::prelude::*;
    running(&format!("{name}, destroying its windows"));
    if gtk::is_initialized() {
        let toplevels = gtk::Window::toplevels();
        let windows: Vec<gtk::Window> = (0..toplevels.n_items())
            .filter_map(|item| toplevels.item(item))
            .filter_map(|object| object.downcast::<gtk::Window>().ok())
            .collect();
        for window in windows {
            window.destroy();
        }
        settle();
    }
    running(&format!("{name}, dropping what it kept"));
    support::drop_kept();
    running(&format!("after {name}"));
}

/// What a GLib critical is stamped with, so a process failing after its case printed "ok" says
/// where (#2192).
static RUNNING: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

fn running(now: &str) {
    now.clone_into(
        &mut RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
}

/// Prints each GTK, GDK and GObject critical with its case, phase and a backtrace: #2192 failed
/// after the body passed, with criticals that did not say what was running.
fn stamp_criticals() {
    for domain in ["Gtk", "Gdk", "GLib-GObject", "GLib", "Adwaita"] {
        glib::log_set_handler(
            Some(domain),
            glib::LogLevels::LEVEL_CRITICAL,
            false,
            false,
            |domain, _, message| {
                let running = RUNNING
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                eprintln!(
                    "({}): {}-CRITICAL **: {message}\n  while running {running}, on thread {:?}\n{}",
                    std::process::id(),
                    domain.unwrap_or("?"),
                    std::thread::current().name().unwrap_or("unnamed"),
                    std::backtrace::Backtrace::force_capture()
                );
            },
        );
    }
}

const HERMETIC: &str = "POSTIO_FOCUS_SUITE_HERMETIC";

/// Runs this binary again with its own config directory: GTK loads the person's `gtk-4.0/gtk.css`,
/// where a desktop can put its palette above any application's.
/// Setting the variable in this process needs `unsafe`, so a child is given it.
fn hermetic(arguments: &[String]) {
    if std::env::var_os(HERMETIC).is_some() {
        return;
    }
    let config = tempfile::tempdir().expect("an empty configuration directory");
    let status = std::process::Command::new(std::env::current_exe().expect("this suite"))
        .args(arguments)
        .env(HERMETIC, "1")
        .env("XDG_CONFIG_HOME", config.path())
        // State too: the remote-image allow list lives there.
        .env("XDG_STATE_HOME", config.path().join("state"))
        // Cache too: a dragged message is written there.
        .env("XDG_CACHE_HOME", config.path().join("cache"))
        // Accessible properties are recorded only with the test backend, so a case can ask what a
        // screen reader would be told.
        .env("GTK_A11Y", "test")
        // GTK's own input method: the Wayland one can be left pointing at a freed context
        // (`input_method`, #2155, #2192).
        .env("GTK_IM_MODULE", "gtk-im-context-simple")
        .status()
        .expect("the suite runs again in its own configuration");
    drop(config);
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        eprintln!(
            "the suite's run died by signal {signal}{}",
            if std::os::unix::process::ExitStatusExt::core_dumped(&status) {
                " (core dumped)"
            } else {
                ""
            }
        );
    }
    std::process::exit(status.code().unwrap_or(101));
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if !arguments.iter().any(|a| a == "--list") {
        hermetic(&arguments);
    }
    if arguments.iter().any(|a| a == "--list") {
        // A runner asks for every test, then `--ignored`; answering the second with the full list
        // makes it run nothing and report success.
        let only_ignored = arguments.iter().any(|a| a == "--ignored");
        for (name, _) in CASES {
            if !only_ignored || IGNORED.contains(name) {
                println!("{name}: test");
            }
        }
        if !arguments.iter().any(|a| a == "terse") {
            println!();
            println!("{} tests, 0 benchmarks", CASES.len());
        }
        return;
    }
    let exact = arguments.iter().any(|a| a == "--exact");
    let run_ignored_only = arguments.iter().any(|a| a == "--ignored");
    let filters: Vec<&str> = arguments
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    let _ = adw::init();
    stamp_criticals();
    let display = support::display();

    let mut failed = Vec::new();
    let mut ran = 0usize;
    for (name, case) in CASES {
        let matched = filters
            .iter()
            .any(|f| if exact { *name == *f } else { name.contains(f) });
        if !filters.is_empty() && !matched {
            continue;
        }
        if IGNORED.contains(name) && filters.is_empty() && !run_ignored_only {
            continue;
        }
        ran += 1;
        if !display {
            println!("test {name} ... ok (skipped: no display)");
            continue;
        }
        println!("test {name} ...");
        running(name);
        if std::panic::catch_unwind(case).is_err() {
            println!("test {name} ... FAILED");
            failed.push(*name);
        } else {
            println!("test {name} ... ok");
        }
        tidy_up(name);
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
