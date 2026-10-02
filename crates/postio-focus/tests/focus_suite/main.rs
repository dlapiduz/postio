//! One binary for Postio Focus's GTK cases -- the custom harness of
//! postio-app's `app_suite` and postio-widgets' `widgets_suite`, for the same
//! two reasons:
//!
//!   * GTK may be initialized from exactly one thread per process (#41), and
//!     libtest runs `#[test]` functions on a thread pool;
//!   * every extra test *binary* links the whole GTK stack, and linking was
//!     once the dominant cost of a GTK crate's tests (#329).
//!
//! So: `harness = false`, one `adw::init`, every case a plain `pub fn` in
//! `focus_suite/`, run in sequence under `catch_unwind` so one failure does
//! not hide the rest. The cases run on the headless compositor the cargo
//! runner puts test binaries on, and assert on the widget tree -- what a
//! person would see -- never on what a layer was handed.
//!
//! **A new case is a module here and a row in `CASES`.** `--list` and name
//! filtering behave enough like libtest for `cargo test`, nextest and the
//! tooling's test counting to work, and that output is a contract:
//! `list_contract.rs` is what notices when it breaks, because a runner that
//! misreads it runs nothing and reports success.
//!
//! A panicking case can leave toolkit state behind that fails a later case:
//! when several cases fail at once, trust the first.

mod a11y;
mod across_apps;
mod bar;
mod buttons;
mod capture;
mod chrome;
mod close_buttons;
mod colours;
mod compose;
mod compose_layout;
mod corrections;
mod cursor;
mod desktop;
mod digest;
mod digest_summary;
mod drafts;
mod empty;
mod filtered;
mod first_run;
mod harness;
mod has_action;
mod icon_buttons;
mod invitations;
mod keymap;
mod list_contract;
mod marked_rows;
mod marker_card;
mod motion;
mod offline_send;
mod one_composer;
mod one_keymap;
mod open_choice;
mod open_keys;
mod open_layout;
mod open_measure;
mod open_message;
mod open_reading;
mod pickers;
mod places;
mod pointer_pairs;
mod registry_parity;
mod reload;
mod remind_on_send;
mod remote_images;
mod row_menu;
mod rows;
mod rule_query;
mod selection;
mod shot;
mod starts_offline;
mod state;
mod store_in_use;
mod store_unavailable;
mod support;
mod surfaced;
mod treatments;
mod undo;
mod view_source;
mod visible_window;
mod window_controls;

/// Cases held out of a default run, by name -- the table-driven spelling of
/// `#[ignore]`, which means one thing here: this machine may not have what
/// the case needs. A name here still runs when asked for explicitly, and still
/// appears in `--list`, exactly as an ignored libtest case does. Say in a
/// comment beside the name which issue or task takes it back.
const IGNORED: &[&str] = &[]; // nothing held out

const CASES: &[(&str, fn())] = &[
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
        "compose_layout::the_close_is_the_shared_x",
        compose_layout::the_close_is_the_shared_x as fn(),
    ),
    (
        "compose_layout::the_dialog_follows_the_message_dialogs_size_rule",
        compose_layout::the_dialog_follows_the_message_dialogs_size_rule as fn(),
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
        "first_run::a_first_sync_that_fails_says_so_rather_than_empty",
        first_run::a_first_sync_that_fails_says_so_rather_than_empty as fn(),
    ),
    (
        "first_run::the_list_keys_work_while_the_first_sync_fills_the_inbox",
        first_run::the_list_keys_work_while_the_first_sync_fills_the_inbox as fn(),
    ),
    (
        "open_message::delete_in_the_dialog_deletes_the_message_on_screen",
        open_message::delete_in_the_dialog_deletes_the_message_on_screen as fn(),
    ),
    (
        "open_message::deleting_from_the_dialog_moves_to_the_next_message",
        open_message::deleting_from_the_dialog_moves_to_the_next_message as fn(),
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
        "first_run::compose_with_no_account_says_so_and_offers_to_add_one",
        first_run::compose_with_no_account_says_so_and_offers_to_add_one as fn(),
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
        "invitations::the_open_invitation_s_card_answers_with_its_keys",
        invitations::the_open_invitation_s_card_answers_with_its_keys as fn(),
    ),
    (
        "drafts::g_t_lists_drafts_and_enter_opens_one_to_edit",
        drafts::g_t_lists_drafts_and_enter_opens_one_to_edit as fn(),
    ),
    (
        "bar::a_plain_word_offers_commands_and_searches_only_when_asked",
        bar::a_plain_word_offers_commands_and_searches_only_when_asked as fn(),
    ),
    (
        "bar::offline_search_answers_locally_one_request_a_keystroke",
        bar::offline_search_answers_locally_one_request_a_keystroke as fn(),
    ),
    (
        "pickers::escape_closes_a_picker_and_changes_nothing",
        pickers::escape_closes_a_picker_and_changes_nothing as fn(),
    ),
    (
        "pickers::h_then_3_reminds_at_the_end_of_the_week",
        pickers::h_then_3_reminds_at_the_end_of_the_week as fn(),
    ),
    (
        "pickers::l_offers_the_labels_of_the_row_s_own_account",
        pickers::l_offers_the_labels_of_the_row_s_own_account as fn(),
    ),
    (
        "pickers::l_toggles_a_label_and_creates_a_new_one",
        pickers::l_toggles_a_label_and_creates_a_new_one as fn(),
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
        "capture::n_appends_a_note_and_alt_s_uses_the_subject",
        capture::n_appends_a_note_and_alt_s_uses_the_subject as fn(),
    ),
    (
        "capture::without_a_vault_there_is_no_task_and_t_says_why",
        capture::without_a_vault_there_is_no_task_and_t_says_why as fn(),
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
        "filtered::the_strip_counts_what_was_filtered_today",
        filtered::the_strip_counts_what_was_filtered_today as fn(),
    ),
    (
        "filtered::focus_never_notifies_for_mail_it_filtered_or_held",
        filtered::focus_never_notifies_for_mail_it_filtered_or_held as fn(),
    ),
    (
        "filtered::f_says_what_a_sweep_would_move_then_moves_it_as_one_undo",
        filtered::f_says_what_a_sweep_would_move_then_moves_it_as_one_undo as fn(),
    ),
    (
        "filtered::r_restores_the_focused_row_and_ctrl_z_takes_it_back",
        filtered::r_restores_the_focused_row_and_ctrl_z_takes_it_back as fn(),
    ),
    (
        "digest::enter_opens_a_digest_and_shift_a_archives_all_of_it",
        digest::enter_opens_a_digest_and_shift_a_archives_all_of_it as fn(),
    ),
    (
        "digest::d_on_a_message_previews_the_rule_and_create_writes_it",
        digest::d_on_a_message_previews_the_rule_and_create_writes_it as fn(),
    ),
    (
        "digest::g_d_lists_the_rules_and_delete_releases_what_one_held",
        digest::g_d_lists_the_rules_and_delete_releases_what_one_held as fn(),
    ),
    (
        "digest::held_mail_is_found_by_search_and_says_where_it_waits",
        digest::held_mail_is_found_by_search_and_says_where_it_waits as fn(),
    ),
    (
        "digest::d_stops_digesting_the_sender_once_confirmed",
        digest::d_stops_digesting_the_sender_once_confirmed as fn(),
    ),
    (
        "digest_summary::a_digest_with_a_summary_opens_on_it_and_every_statement_cites_its_mail",
        digest_summary::a_digest_with_a_summary_opens_on_it_and_every_statement_cites_its_mail
            as fn(),
    ),
    (
        "digest_summary::a_reference_opens_its_email_in_place_with_the_passage_highlighted",
        digest_summary::a_reference_opens_its_email_in_place_with_the_passage_highlighted as fn(),
    ),
    (
        "rule_query::match_a_list_or_a_search_instead_previews_and_saves_the_typed_query",
        rule_query::match_a_list_or_a_search_instead_previews_and_saves_the_typed_query as fn(),
    ),
    (
        "rule_query::digest_mail_like_this_is_absent_with_no_model_configured",
        rule_query::digest_mail_like_this_is_absent_with_no_model_configured as fn(),
    ),
    (
        "rule_query::digest_mail_like_this_present_with_a_message_says_so_with_no_model",
        rule_query::digest_mail_like_this_present_with_a_message_says_so_with_no_model as fn(),
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
        "pickers::s_then_2_snoozes_the_row_until_tomorrow_morning",
        pickers::s_then_2_snoozes_the_row_until_tomorrow_morning as fn(),
    ),
    (
        "places::g_o_then_trav_and_enter_shows_travel",
        places::g_o_then_trav_and_enter_shows_travel as fn(),
    ),
    (
        "bar::alt_2_runs_the_second_saved_search",
        bar::alt_2_runs_the_second_saved_search as fn(),
    ),
    (
        "bar::half_typed_operators_show_no_error_and_results_keep_updating",
        bar::half_typed_operators_show_no_error_and_results_keep_updating as fn(),
    ),
    (
        "bar::a_sentence_names_its_sender_from_the_address_book",
        bar::a_sentence_names_its_sender_from_the_address_book as fn(),
    ),
    (
        "bar::tab_steps_into_the_chips_and_ctrl_s_saves_the_query",
        bar::tab_steps_into_the_chips_and_ctrl_s_saves_the_query as fn(),
    ),
    (
        "bar::in_rec_lists_receipts_newest_first",
        bar::in_rec_lists_receipts_newest_first as fn(),
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
        "treatments::switching_the_treatment_leaves_the_column_where_it_was",
        treatments::switching_the_treatment_leaves_the_column_where_it_was as fn(),
    ),
    (
        "treatments::switching_a_marked_message_keeps_the_reading_position",
        treatments::switching_a_marked_message_keeps_the_reading_position as fn(),
    ),
    (
        "open_layout::the_render_mode_line_sits_24_under_the_card_and_12_over_the_body",
        open_layout::the_render_mode_line_sits_24_under_the_card_and_12_over_the_body as fn(),
    ),
    (
        "open_layout::the_action_cards_sentence_wraps_and_is_never_cut",
        open_layout::the_action_cards_sentence_wraps_and_is_never_cut as fn(),
    ),
    (
        "open_layout::delete_s_cap_reads_del_in_the_action_row",
        open_layout::delete_s_cap_reads_del_in_the_action_row as fn(),
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
        "open_message::escape_closes_and_keeps_the_cursor_and_the_selection",
        open_message::escape_closes_and_keeps_the_cursor_and_the_selection as fn(),
    ),
    (
        "open_message::j_and_k_step_the_list_behind_the_dialog",
        open_message::j_and_k_step_the_list_behind_the_dialog as fn(),
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
        "open_message::brackets_step_through_the_thread",
        open_message::brackets_step_through_the_thread as fn(),
    ),
    (
        "open_message::enter_opens_the_conversation_over_the_list_at_once",
        open_message::enter_opens_the_conversation_over_the_list_at_once as fn(),
    ),
    (
        "open_keys::the_arrows_and_paging_keys_scroll_the_open_message_in_steps",
        open_keys::the_arrows_and_paging_keys_scroll_the_open_message_in_steps as fn(),
    ),
    (
        "open_keys::j_and_k_step_the_open_message_from_where_the_keyboard_is",
        open_keys::j_and_k_step_the_open_message_from_where_the_keyboard_is as fn(),
    ),
    (
        "open_reading::the_open_message_is_one_scrolling_column",
        open_reading::the_open_message_is_one_scrolling_column as fn(),
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
        "row_menu::a_menu_verb_runs_its_command_on_the_row",
        row_menu::a_menu_verb_runs_its_command_on_the_row as fn(),
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
        "close_buttons::every_closable_surface_has_the_same_x_at_the_right",
        close_buttons::every_closable_surface_has_the_same_x_at_the_right as fn(),
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
        "open_layout::the_dialog_is_sized_by_the_window_and_never_by_the_message",
        open_layout::the_dialog_is_sized_by_the_window_and_never_by_the_message as fn(),
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
        "open_measure::the_measure_is_near_seventy_characters_in_a_centred_column",
        open_measure::the_measure_is_near_seventy_characters_in_a_centred_column as fn(),
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
        "corrections::three_dismissals_write_a_stop_marker_to_focus_s_config",
        corrections::three_dismissals_write_a_stop_marker_to_focus_s_config as fn(),
    ),
    (
        "invitations::y_accepts_from_the_row_and_the_toast_lasts_the_window",
        invitations::y_accepts_from_the_row_and_the_toast_lasts_the_window as fn(),
    ),
    (
        "invitations::a_click_on_the_row_s_decline_declines",
        invitations::a_click_on_the_row_s_decline_declines as fn(),
    ),
    (
        "invitations::a_cancelled_or_past_invitation_offers_no_answer",
        invitations::a_cancelled_or_past_invitation_offers_no_answer as fn(),
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
        "one_composer::the_same_content_from_either_app_leaves_as_the_same_message",
        one_composer::the_same_content_from_either_app_leaves_as_the_same_message as fn(),
    ),
    (
        "drafts::escape_keeps_the_draft_and_the_classic_app_opens_it",
        drafts::escape_keeps_the_draft_and_the_classic_app_opens_it as fn(),
    ),
    (
        "drafts::a_draft_the_classic_app_kept_opens_in_focus",
        drafts::a_draft_the_classic_app_kept_opens_in_focus as fn(),
    ),
    (
        "compose::reply_all_starts_with_every_recipient_re_the_labels_and_a_folded_quote",
        compose::reply_all_starts_with_every_recipient_re_the_labels_and_a_folded_quote as fn(),
    ),
    (
        "across_apps::what_focus_archives_the_classic_app_sees_archived",
        across_apps::what_focus_archives_the_classic_app_sees_archived as fn(),
    ),
    (
        "store_in_use::a_store_another_postio_has_open_is_refused_with_try_again_and_left_alone",
        store_in_use::a_store_another_postio_has_open_is_refused_with_try_again_and_left_alone
            as fn(),
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
        "one_keymap::a_registered_command_reaches_the_key_map_with_its_key",
        one_keymap::a_registered_command_reaches_the_key_map_with_its_key as fn(),
    ),
    (
        "one_keymap::no_default_key_means_two_things_and_each_app_runs_the_same_key",
        one_keymap::no_default_key_means_two_things_and_each_app_runs_the_same_key as fn(),
    ),
    (
        "one_keymap::the_classic_app_s_defaults_are_the_one_keymap_s",
        one_keymap::the_classic_app_s_defaults_are_the_one_keymap_s as fn(),
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
        "empty::without_filtering_the_empty_inbox_names_the_next_digest_and_no_count",
        empty::without_filtering_the_empty_inbox_names_the_next_digest_and_no_count as fn(),
    ),
    (
        "state::during_a_first_sync_what_has_arrived_is_listed",
        state::during_a_first_sync_what_has_arrived_is_listed as fn(),
    ),
    (
        "state::offline_an_archive_takes_effect_at_once_and_queues",
        state::offline_an_archive_takes_effect_at_once_and_queues as fn(),
    ),
    (
        "state::offline_a_label_shows_at_once_and_queues_and_search_answers",
        state::offline_a_label_shows_at_once_and_queues_and_search_answers as fn(),
    ),
    (
        "state::during_a_first_sync_what_has_arrived_opens_and_is_found",
        state::during_a_first_sync_what_has_arrived_opens_and_is_found as fn(),
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
        "has_action::has_action_narrows_to_the_marked_rows_and_back",
        has_action::has_action_narrows_to_the_marked_rows_and_back as fn(),
    ),
    (
        "list_contract::the_list_output_stays_libtest_shaped",
        list_contract::the_list_output_stays_libtest_shaped as fn(),
    ),
    ("harness::an_empty_case", harness::an_empty_case as fn()),
    (
        "harness::an_empty_case_is_listed_and_runs",
        harness::an_empty_case_is_listed_and_runs as fn(),
    ),
    (
        "chrome::the_top_bar_and_the_header_strip_carry_each_control_and_its_key",
        chrome::the_top_bar_and_the_header_strip_carry_each_control_and_its_key as fn(),
    ),
    (
        "cursor::j_and_k_move_only_the_cursor",
        cursor::j_and_k_move_only_the_cursor as fn(),
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
        "marked_rows::a_marked_row_is_two_lines_whatever_its_state",
        marked_rows::a_marked_row_is_two_lines_whatever_its_state as fn(),
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
        "selection::three_selected_and_the_cursor_on_a_fourth_archives_exactly_the_three",
        selection::three_selected_and_the_cursor_on_a_fourth_archives_exactly_the_three as fn(),
    ),
    (
        "selection::escape_clears_the_selection_and_the_cursor_stays",
        selection::escape_clears_the_selection_and_the_cursor_stays as fn(),
    ),
    (
        "selection::a_select_all_archives_what_focus_lists_and_never_held_mail",
        selection::a_select_all_archives_what_focus_lists_and_never_held_mail as fn(),
    ),
    (
        "starts_offline::the_inbox_is_listed_from_the_store_with_no_network",
        starts_offline::the_inbox_is_listed_from_the_store_with_no_network as fn(),
    ),
    (
        "undo::one_ctrl_z_returns_all_three_after_the_toast_has_gone",
        undo::one_ctrl_z_returns_all_three_after_the_toast_has_gone as fn(),
    ),
    (
        "visible_window::a_row_whose_page_has_not_landed_draws_a_skeleton",
        visible_window::a_row_whose_page_has_not_landed_draws_a_skeleton as fn(),
    ),
    (
        "visible_window::a_jump_to_the_bottom_reads_the_ends_and_nothing_between",
        visible_window::a_jump_to_the_bottom_reads_the_ends_and_nothing_between as fn(),
    ),
];

use gtk::glib;

/// Turn the GTK main loop until there is nothing left to do.
pub fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

/// Let the main loop run for at least `least`: a dwell a case must outlast,
/// not a deadline it waits against.
pub async fn settle_for(least: std::time::Duration) {
    let started = std::time::Instant::now();
    settle_until(async || started.elapsed() >= least).await;
}

/// Turn the loop until `done`, or give up after ten seconds (scaled by
/// `POSTIO_TEST_PATIENCE`). Returns whether it happened, because every call
/// site is already inside an `assert!` that says what was expected.
pub async fn settle_until<F, Fut>(done: F) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while std::time::Instant::now() < deadline {
        settle();
        if done().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    done().await
}

/// Turn the loop while `held` stays true, for half a second (scaled): the
/// inverse of `settle_until`, for proving something does *not* happen.
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

/// Run a case's body, which is async because the store is.
///
/// On **this** thread, because everything in it touches GTK; on a
/// multi-threaded runtime, because a synchronous store read reached from
/// inside it (`block_in_place`) panics on a current-thread one. One runtime
/// per thread, not per case. See app_suite's `gtk_case` for the whole story.
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

/// Destroy every window a case left open, let the teardown run, then drop
/// what the case kept (`support::keep`): outside every runtime, which is the
/// one place a host's own runtime may be dropped.
fn tidy_up() {
    use gtk::prelude::*;
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
    support::drop_kept();
}

/// Set once the suite is running in a configuration of its own.
const HERMETIC: &str = "POSTIO_FOCUS_SUITE_HERMETIC";

/// Run this binary again with a configuration directory of its own, and exit
/// as it does -- unless this is that run.
///
/// GTK loads the person's own `gtk-4.0/gtk.css` from `XDG_CONFIG_HOME`, and a
/// desktop can put its whole palette there: COSMIC writes its dark colours as
/// `@define-color`s, at a priority above any application's, whatever the
/// colour scheme. A case asserting what Focus's colours resolve to would then
/// be asserting the developer's theme. An empty directory is the machine CI
/// is. It also keeps every case away from the person's `config.toml`.
/// Setting the variable in this process would need `unsafe`, which the
/// workspace forbids; a child is given it instead.
fn hermetic(arguments: &[String]) {
    if std::env::var_os(HERMETIC).is_some() {
        return;
    }
    let config = tempfile::tempdir().expect("an empty configuration directory");
    let status = std::process::Command::new(std::env::current_exe().expect("this suite"))
        .args(arguments)
        .env(HERMETIC, "1")
        .env("XDG_CONFIG_HOME", config.path())
        // State too: the remote-image allow list lives there, and a case
        // about blocked images must not read the developer's own.
        .env("XDG_STATE_HOME", config.path().join("state"))
        // Accessible properties are recorded only with a backend: the test
        // one, so a case can ask what a screen reader would be told (T142).
        .env("GTK_A11Y", "test")
        .status()
        .expect("the suite runs again in its own configuration");
    drop(config);
    std::process::exit(status.code().unwrap_or(101));
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if !arguments.iter().any(|a| a == "--list") {
        hermetic(&arguments);
    }
    if arguments.iter().any(|a| a == "--list") {
        // Two questions, and a libtest-compatible runner asks both: every
        // test, then `--ignored` for the ignored subset. Answering the second
        // with the full list tells a process-per-test runner that everything
        // is ignored -- it then runs nothing and reports success.
        let only_ignored = arguments.iter().any(|a| a == "--ignored");
        for (name, _) in CASES {
            if !only_ignored || IGNORED.contains(name) {
                println!("{name}: test");
            }
        }
        // `--format terse` is a machine-readable contract: real libtest emits
        // the names and nothing else. The count is for the non-terse form.
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

    // One initialisation for the process, before any case. No display is not
    // an error here; a case that needs one says so itself.
    let _ = adw::init();

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
        tidy_up();
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
