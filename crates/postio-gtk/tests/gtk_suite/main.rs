//! One binary for the single-test GTK suites, run sequentially on the main
//! thread — a custom harness, because the two constraints cannot both be met
//! any other way:
//!
//!   * GTK may be initialized from exactly one thread per process (#41), and
//!     libtest runs `#[test]` functions on a thread pool;
//!   * every extra test *binary* links the whole GTK + WebKit stack, and at
//!     65 binaries linking was the dominant cost of `cargo test -p
//!     postio-gtk` on this machine (#329).
//!
//! So: `harness = false`, one `adw::init` (each case's own guard becomes a
//! harmless re-init), every case a plain `pub fn` in `gtk_suite/`, run in
//! sequence under `catch_unwind` so one failure does not hide the rest.
//! `--list` and name filtering behave enough like libtest for `cargo test
//! gtk_suite <name>` and the tooling's test counting to keep working.
//!
//! What deliberately stays out: `gtk_reader` (WebKit; runs under the
//! headless runner's watchdog, in isolation — #272), `gtk_accessibility`
//! (its own display races, #45/#114), and `gtk_composer` — which asserts
//! the 16ms interaction budget and therefore measures *the process it runs
//! in*. It passes alone and failed at 16.37ms as the 122nd case in a shared
//! binary, against an allocator 121 cases had already warmed and fragmented.
//! A wall-clock budget cannot share a process with an arbitrary amount of
//! prior work and still mean what it says (#841).
//!
//! Multi-test files used to stay out too, on the grounds that they "already
//! amortize their binary". They did not amortize anything: a file with two
//! display-needing `#[test]`s hands them to libtest's thread pool, GTK
//! tolerates one thread, and the loser returns through its own `no display`
//! guard and is reported as a pass. Three such files had been quietly
//! running half their cases — see #355, and
//! `scripts/checks/check-one-gtk-test-per-binary.py`, which now refuses a
//! new one. This is where such cases belong; a second case here costs one
//! `pub fn`, not a binary.
//!
//! A panicking case can leave toolkit state behind that fails a later case:
//! when several cases fail at once, trust the first.

mod feed;
mod feed_results;
mod gtk_accelerators;
mod gtk_cheatsheet;
mod gtk_composer_action_row;
mod gtk_composer_attachments;
mod gtk_composer_autosave;
mod gtk_composer_detach;
mod gtk_composer_document;
mod gtk_composer_focus;
mod gtk_composer_header;
mod gtk_composer_inline_image;
mod gtk_composer_keymap;
mod gtk_composer_recipient_select;
mod gtk_composer_recipients;
mod gtk_composer_reply;
mod gtk_composer_resume;
mod gtk_composer_schedule_send;
mod gtk_composer_signature_default;
mod gtk_composer_toolbar;
mod gtk_composer_tracking_notice;
mod gtk_conversation;
mod gtk_conversation_index;
mod gtk_cursor_preview;
mod gtk_dispatch;
mod gtk_display_required;
mod gtk_dwell;
mod gtk_editable_dialect;
mod gtk_editor_bridge;
mod gtk_editor_format;
mod gtk_editor_images;
mod gtk_editor_profile;
mod gtk_feeds;
mod gtk_finder;
mod gtk_finder_focus;
mod gtk_flagged;
mod gtk_focus_visible;
mod gtk_folder_sections;
mod gtk_identity;
mod gtk_keymap_lazy;
mod gtk_layout_intent;
mod gtk_list_focus_return;
mod gtk_list_recycling;
mod gtk_list_reload;
mod gtk_list_select_message;
mod gtk_list_state;
mod gtk_live_config;
mod gtk_move_picker;
mod gtk_new_mail_scroll;
mod gtk_next_scope;
mod gtk_onboarding;
mod gtk_onboarding_enter;
mod gtk_onboarding_guess;
mod gtk_onboarding_name;
mod gtk_pane_cycle;
mod gtk_parts;
mod gtk_prev_view;
mod gtk_reader_account;
mod gtk_reader_actions;
mod gtk_reader_fonts;
mod gtk_reader_pane_owner;
mod gtk_reader_scroll;
mod gtk_reading_pane;
mod gtk_result_order;
mod gtk_row;
mod gtk_saved_searches_live;
mod gtk_search_live;
mod gtk_search_panel;
mod gtk_search_preview;
mod gtk_selection;
mod gtk_settings;
mod gtk_settings_accounts;
mod gtk_shell;
mod gtk_sidebar;
mod gtk_sidebar_accounts;
mod gtk_sidebar_backfill_exclusion;
mod gtk_sidebar_height;
mod gtk_sidebar_keys;
mod gtk_sidebar_saved_searches;
mod gtk_sidebar_sections;
mod gtk_sidebar_tree;
mod gtk_signature_placement;
mod gtk_style;
mod gtk_thread;
mod gtk_thread_dwell_cancel;
mod gtk_thread_scope;
mod gtk_toast;
mod gtk_toggle_sidebar;
mod gtk_unavailable;
mod gtk_undo_toast;
mod gtk_window;
mod gtk_window_open_message;
mod gtk_window_run_search;
mod no_stray_prints;

const CASES: &[(&str, fn())] = &[
    (
        "feed::the_message_list_is_fed_from_the_runtime",
        feed::the_message_list_is_fed_from_the_runtime as fn(),
    ),
    (
        "feed_results::search_hits_reach_the_message_list",
        feed_results::search_hits_reach_the_message_list as fn(),
    ),
    (
        "gtk_keymap_lazy::applying_a_keymap_does_not_build_a_composer_nobody_asked_for",
        gtk_keymap_lazy::applying_a_keymap_does_not_build_a_composer_nobody_asked_for as fn(),
    ),
    (
        "gtk_layout_intent::a_narrow_window_hides_the_sidebar_and_widening_brings_it_back",
        gtk_layout_intent::a_narrow_window_hides_the_sidebar_and_widening_brings_it_back as fn(),
    ),
    (
        "gtk_layout_intent::a_sidebar_turned_off_stays_off_across_a_resize",
        gtk_layout_intent::a_sidebar_turned_off_stays_off_across_a_resize as fn(),
    ),
    (
        "gtk_layout_intent::reaching_for_the_sidebar_on_a_narrow_window_is_not_a_preference",
        gtk_layout_intent::reaching_for_the_sidebar_on_a_narrow_window_is_not_a_preference as fn(),
    ),
    (
        "gtk_layout_intent::opening_a_message_gives_the_reader_the_screen_when_there_is_room_for_one",
        gtk_layout_intent::opening_a_message_gives_the_reader_the_screen_when_there_is_room_for_one as fn(),
    ),
    (
        "gtk_composer_autosave::typing_debounces_into_one_autosave_and_closing_flushes_what_is_pending",
        gtk_composer_autosave::typing_debounces_into_one_autosave_and_closing_flushes_what_is_pending
            as fn(),
    ),
    (
        "gtk_composer_autosave::saving_twice_carries_the_assigned_id_forward_into_the_second_save",
        gtk_composer_autosave::saving_twice_carries_the_assigned_id_forward_into_the_second_save
            as fn(),
    ),
    (
        "gtk_composer_document::the_body_round_trips_through_the_neutral_document",
        gtk_composer_document::the_body_round_trips_through_the_neutral_document as fn(),
    ),
    (
        "gtk_composer_focus::focus_lands_when_the_composer_opens_before_the_window_is_ever_mapped",
        gtk_composer_focus::focus_lands_when_the_composer_opens_before_the_window_is_ever_mapped
            as fn(),
    ),
    (
        "gtk_composer_recipient_select::clicking_a_suggestion_puts_that_one_in_the_field",
        gtk_composer_recipient_select::clicking_a_suggestion_puts_that_one_in_the_field as fn(),
    ),
    (
        "gtk_composer_recipient_select::return_commits_the_suggestion_the_popover_has_selected",
        gtk_composer_recipient_select::return_commits_the_suggestion_the_popover_has_selected
            as fn(),
    ),
    (
        "gtk_composer_recipient_select::nothing_is_offered_until_four_characters_are_typed",
        gtk_composer_recipient_select::nothing_is_offered_until_four_characters_are_typed as fn(),
    ),
    (
        "gtk_composer_recipient_select::accepting_a_group_inserts_every_member",
        gtk_composer_recipient_select::accepting_a_group_inserts_every_member as fn(),
    ),
    (
        "gtk_composer_signature_default::a_resolved_signature_wins_over_the_identity_s_own",
        gtk_composer_signature_default::a_resolved_signature_wins_over_the_identity_s_own as fn(),
    ),
    (
        "gtk_composer_signature_default::a_resolved_signature_the_account_does_not_have_falls_back_to_the_identity",
        gtk_composer_signature_default::a_resolved_signature_the_account_does_not_have_falls_back_to_the_identity
            as fn(),
    ),
    (
        "gtk_composer_signature_default::no_resolution_resets_a_picker_a_previous_compose_left_pointed_elsewhere",
        gtk_composer_signature_default::no_resolution_resets_a_picker_a_previous_compose_left_pointed_elsewhere
            as fn(),
    ),
    (
        "gtk_conversation::the_conversation_pane_stacks_a_thread_and_acts_per_message",
        gtk_conversation::the_conversation_pane_stacks_a_thread_and_acts_per_message as fn(),
    ),
    (
        "gtk_conversation::reader_for_finds_only_an_expanded_entrys_own_reader",
        gtk_conversation::reader_for_finds_only_an_expanded_entrys_own_reader as fn(),
    ),
    (
        "gtk_conversation::an_expanded_entrys_reader_does_not_draw_its_own_action_bar",
        gtk_conversation::an_expanded_entrys_reader_does_not_draw_its_own_action_bar as fn(),
    ),
    (
        "gtk_conversation_index::the_column_and_the_conversation_share_one_current_message",
        gtk_conversation_index::the_column_and_the_conversation_share_one_current_message as fn(),
    ),
    (
        "gtk_display_required::ci_has_a_display_to_run_the_gtk_suites_on",
        gtk_display_required::ci_has_a_display_to_run_the_gtk_suites_on as fn(),
    ),
    (
        "gtk_feeds::the_panes_follow_the_account_the_sync_and_the_folder_you_pick",
        gtk_feeds::the_panes_follow_the_account_the_sync_and_the_folder_you_pick as fn(),
    ),
    (
        "gtk_flagged::the_sidebar_offers_flagged_and_opening_it_lists_the_flagged_mail",
        gtk_flagged::the_sidebar_offers_flagged_and_opening_it_lists_the_flagged_mail as fn(),
    ),
    (
        "gtk_flagged::flagged_and_snoozed_update_live_on_a_membership_change",
        gtk_flagged::flagged_and_snoozed_update_live_on_a_membership_change as fn(),
    ),
    (
        "gtk_finder::one_box_searches_mail_runs_commands_and_jumps_to_folders",
        gtk_finder::one_box_searches_mail_runs_commands_and_jumps_to_folders as fn(),
    ),
    (
        "gtk_finder::at_finds_a_correspondent_and_searches_their_mail",
        gtk_finder::at_finds_a_correspondent_and_searches_their_mail as fn(),
    ),
    (
        "gtk_finder::typing_a_mode_prefix_does_not_warn_gtk",
        gtk_finder::typing_a_mode_prefix_does_not_warn_gtk as fn(),
    ),
    (
        "gtk_focus_visible::taking_focus_changes_what_is_drawn",
        gtk_focus_visible::taking_focus_changes_what_is_drawn as fn(),
    ),
    (
        "gtk_list_focus_return::shift_tab_from_after_the_list_returns_to_the_cursor_row",
        gtk_list_focus_return::shift_tab_from_after_the_list_returns_to_the_cursor_row as fn(),
    ),
    (
        "gtk_list_focus_return::tab_from_before_the_list_also_returns_to_the_cursor_row",
        gtk_list_focus_return::tab_from_before_the_list_also_returns_to_the_cursor_row as fn(),
    ),
    (
        "gtk_list_recycling::a_list_view_builds_a_bounded_window_however_big_the_model_is",
        gtk_list_recycling::a_list_view_builds_a_bounded_window_however_big_the_model_is as fn(),
    ),
    (
        "gtk_list_reload::a_batch_arriving_mid_sync_leaves_the_cursor_and_the_selection_alone",
        gtk_list_reload::a_batch_arriving_mid_sync_leaves_the_cursor_and_the_selection_alone
            as fn(),
    ),
    (
        "gtk_new_mail_scroll::new_mail_reveals_itself_at_the_top_and_nowhere_else",
        gtk_new_mail_scroll::new_mail_reveals_itself_at_the_top_and_nowhere_else as fn(),
    ),
    (
        "gtk_move_picker::m_opens_the_folder_picker_and_the_folder_picked_becomes_the_move",
        gtk_move_picker::m_opens_the_folder_picker_and_the_folder_picked_becomes_the_move as fn(),
    ),
    (
        "gtk_next_scope::g_a_cycles_the_strip_the_same_way_clicking_its_rows_does",
        gtk_next_scope::g_a_cycles_the_strip_the_same_way_clicking_its_rows_does as fn(),
    ),
    (
        "gtk_parts::the_parts_panel_walks_a_message_without_fetching_any_of_it",
        gtk_parts::the_parts_panel_walks_a_message_without_fetching_any_of_it as fn(),
    ),
    (
        "gtk_prev_view::h_steps_back_out_of_a_thread_the_same_way_escape_does",
        gtk_prev_view::h_steps_back_out_of_a_thread_the_same_way_escape_does as fn(),
    ),
    (
        "gtk_reader_scroll::page_down_and_page_up_move_a_marker_at_a_time",
        gtk_reader_scroll::page_down_and_page_up_move_a_marker_at_a_time as fn(),
    ),
    (
        "gtk_reader_scroll::a_new_message_resets_the_scroll_position",
        gtk_reader_scroll::a_new_message_resets_the_scroll_position as fn(),
    ),
    (
        "gtk_reader_scroll::paging_with_nothing_open_does_nothing",
        gtk_reader_scroll::paging_with_nothing_open_does_nothing as fn(),
    ),
    (
        "gtk_reading_pane::the_reading_pane_shows_a_message_and_yields_it_to_the_composer",
        gtk_reading_pane::the_reading_pane_shows_a_message_and_yields_it_to_the_composer as fn(),
    ),
    (
        "gtk_row::the_row_draws_the_canvas_anatomy_at_every_density",
        gtk_row::the_row_draws_the_canvas_anatomy_at_every_density as fn(),
    ),
    (
        "gtk_saved_searches_live::pinned_filters_reach_the_sidebar_and_ctrl_s_adds_one",
        gtk_saved_searches_live::pinned_filters_reach_the_sidebar_and_ctrl_s_adds_one as fn(),
    ),
    (
        "gtk_saved_searches_live::keyboard_reaches_saved_searches_and_their_move_verbs",
        gtk_saved_searches_live::keyboard_reaches_saved_searches_and_their_move_verbs as fn(),
    ),
    (
        "gtk_saved_searches_live::a_pinned_saved_search_does_not_auto_run_on_first_present",
        gtk_saved_searches_live::a_pinned_saved_search_does_not_auto_run_on_first_present as fn(),
    ),
    (
        "gtk_result_order::the_sort_control_tells_the_truth_over_results",
        gtk_result_order::the_sort_control_tells_the_truth_over_results as fn(),
    ),
    (
        "gtk_search_live::the_readout_answers_the_query_on_screen_and_no_other",
        gtk_search_live::the_readout_answers_the_query_on_screen_and_no_other as fn(),
    ),
    (
        "gtk_search_panel::the_scope_column_narrows_a_search_without_retyping_it",
        gtk_search_panel::the_scope_column_narrows_a_search_without_retyping_it as fn(),
    ),
    (
        "gtk_reader_pane_owner::the_reading_pane_has_one_visible_occupant_at_a_time",
        gtk_reader_pane_owner::the_reading_pane_has_one_visible_occupant_at_a_time as fn(),
    ),
    (
        "gtk_search_preview::the_preview_follows_the_focus_and_answers_the_query_on_screen",
        gtk_search_preview::the_preview_follows_the_focus_and_answers_the_query_on_screen as fn(),
    ),
    (
        "gtk_selection::the_cursor_and_the_selection_are_two_different_things",
        gtk_selection::the_cursor_and_the_selection_are_two_different_things as fn(),
    ),
    (
        "gtk_settings::the_settings_panel_edits_the_file_in_place",
        gtk_settings::the_settings_panel_edits_the_file_in_place as fn(),
    ),
    (
        "gtk_settings::a_keymap_problem_shows_up_on_the_settings_footer_not_only_a_debug_log",
        gtk_settings::a_keymap_problem_shows_up_on_the_settings_footer_not_only_a_debug_log as fn(),
    ),
    (
        "gtk_settings_accounts::accounts_render_as_rows_and_hide_when_there_are_none",
        gtk_settings_accounts::accounts_render_as_rows_and_hide_when_there_are_none as fn(),
    ),
    (
        "gtk_settings_accounts::flipping_the_switch_reports_the_account_and_the_new_state",
        gtk_settings_accounts::flipping_the_switch_reports_the_account_and_the_new_state as fn(),
    ),
    (
        "gtk_settings_accounts::the_context_menu_reaches_the_action_handler_with_the_right_account",
        gtk_settings_accounts::the_context_menu_reaches_the_action_handler_with_the_right_account
            as fn(),
    ),
    (
        "gtk_sidebar::the_sidebar_lists_folders_and_says_where_sync_stands",
        gtk_sidebar::the_sidebar_lists_folders_and_says_where_sync_stands as fn(),
    ),
    (
        "gtk_sidebar::a_manual_sync_is_reachable_in_every_connection_state",
        gtk_sidebar::a_manual_sync_is_reachable_in_every_connection_state as fn(),
    ),
    (
        "gtk_sidebar_backfill_exclusion::the_menu_offers_one_entry_worded_for_the_current_state",
        gtk_sidebar_backfill_exclusion::the_menu_offers_one_entry_worded_for_the_current_state
            as fn(),
    ),
    (
        "gtk_pane_cycle::tab_walks_the_panes_and_shift_tab_walks_back",
        gtk_pane_cycle::tab_walks_the_panes_and_shift_tab_walks_back as fn(),
    ),
    (
        "gtk_sidebar_keys::a_mailbox_can_be_chosen_without_touching_the_mouse",
        gtk_sidebar_keys::a_mailbox_can_be_chosen_without_touching_the_mouse as fn(),
    ),
    (
        "gtk_sidebar_saved_searches::saved_searches_list_keyboard_navigate_and_report_their_query",
        gtk_sidebar_saved_searches::saved_searches_list_keyboard_navigate_and_report_their_query
            as fn(),
    ),
    (
        "gtk_sidebar_saved_searches::the_context_menu_reaches_the_action_handler_with_the_right_key",
        gtk_sidebar_saved_searches::the_context_menu_reaches_the_action_handler_with_the_right_key
            as fn(),
    ),
    (
        "gtk_sidebar_saved_searches::the_first_row_has_no_move_up_entry",
        gtk_sidebar_saved_searches::the_first_row_has_no_move_up_entry as fn(),
    ),
    (
        "gtk_sidebar_saved_searches::the_last_row_has_no_move_down_entry",
        gtk_sidebar_saved_searches::the_last_row_has_no_move_down_entry as fn(),
    ),
    (
        "gtk_sidebar_tree::folders_nest_collapse_and_a_noselect_parent_only_toggles",
        gtk_sidebar_tree::folders_nest_collapse_and_a_noselect_parent_only_toggles as fn(),
    ),
    (
        "gtk_style::the_generated_stylesheet_works_in_gtk",
        gtk_style::the_generated_stylesheet_works_in_gtk as fn(),
    ),
    (
        "gtk_thread_dwell_cancel::opening_a_conversation_stops_the_lists_clock",
        gtk_thread_dwell_cancel::opening_a_conversation_stops_the_lists_clock as fn(),
    ),
    (
        "gtk_thread::t_drills_into_a_thread_and_esc_puts_the_list_back_exactly",
        gtk_thread::t_drills_into_a_thread_and_esc_puts_the_list_back_exactly as fn(),
    ),
    (
        "gtk_thread_scope::drilling_in_shows_the_thread_and_not_just_this_folders_part_of_it",
        gtk_thread_scope::drilling_in_shows_the_thread_and_not_just_this_folders_part_of_it
            as fn(),
    ),
    (
        "gtk_toggle_sidebar::toggle_sidebar_moves_the_sidebar_from_the_palette_and_from_ctrl_b",
        gtk_toggle_sidebar::toggle_sidebar_moves_the_sidebar_from_the_palette_and_from_ctrl_b
            as fn(),
    ),
    (
        "gtk_window::the_window_opens_and_wears_the_design",
        gtk_window::the_window_opens_and_wears_the_design as fn(),
    ),
    (
        "gtk_window_run_search::run_search_opens_the_box_and_answers_immediately",
        gtk_window_run_search::run_search_opens_the_box_and_answers_immediately as fn(),
    ),
    (
        "no_stray_prints::no_source_file_prints_outside_its_tests",
        no_stray_prints::no_source_file_prints_outside_its_tests as fn(),
    ),
    (
        "gtk_accelerators::menu_items_carry_parseable_accelerators",
        gtk_accelerators::menu_items_carry_parseable_accelerators as fn(),
    ),
    (
        "gtk_cheatsheet::the_cheat_sheet_opens_and_reprints_on_a_rebind",
        gtk_cheatsheet::the_cheat_sheet_opens_and_reprints_on_a_rebind as fn(),
    ),
    (
        "gtk_composer_action_row::the_escape_hint_is_the_one_allowed_to_shrink",
        gtk_composer_action_row::the_escape_hint_is_the_one_allowed_to_shrink as fn(),
    ),
    (
        "gtk_composer_attachments::attaching_shows_the_row_and_removing_cleans_it_up",
        gtk_composer_attachments::attaching_shows_the_row_and_removing_cleans_it_up as fn(),
    ),
    (
        "gtk_composer_detach::the_composer_detaches_into_its_own_window_and_comes_back",
        gtk_composer_detach::the_composer_detaches_into_its_own_window_and_comes_back as fn(),
    ),
    (
        "gtk_composer_header::the_compose_button_tracks_the_composer_and_closes_it_when_pressed_again",
        gtk_composer_header::the_compose_button_tracks_the_composer_and_closes_it_when_pressed_again as fn(),
    ),
    (
        "gtk_composer_inline_image::a_pasted_image_becomes_an_inline_attachment_and_renders_at_the_caret",
        gtk_composer_inline_image::a_pasted_image_becomes_an_inline_attachment_and_renders_at_the_caret as fn(),
    ),
    (
        "gtk_composer_keymap::a_composer_built_after_a_rebind_starts_on_the_rebound_key",
        gtk_composer_keymap::a_composer_built_after_a_rebind_starts_on_the_rebound_key as fn(),
    ),
    (
        "gtk_composer_recipients::typing_a_prefix_offers_suggestions_and_accepting_one_completes_it",
        gtk_composer_recipients::typing_a_prefix_offers_suggestions_and_accepting_one_completes_it as fn(),
    ),
    (
        "gtk_composer_reply::e_shift_e_and_f_open_reply_reply_all_and_forward",
        gtk_composer_reply::e_shift_e_and_f_open_reply_reply_all_and_forward as fn(),
    ),
    (
        "gtk_composer_resume::resuming_replaces_the_draft_the_composer_was_holding",
        gtk_composer_resume::resuming_replaces_the_draft_the_composer_was_holding as fn(),
    ),
    (
        "gtk_composer_schedule_send::ctrl_shift_return_opens_the_schedule_send_picker",
        gtk_composer_schedule_send::ctrl_shift_return_opens_the_schedule_send_picker as fn(),
    ),
    (
        "gtk_composer_toolbar::the_toolbar_reaches_the_registry_commands_and_reflects_the_caret",
        gtk_composer_toolbar::the_toolbar_reaches_the_registry_commands_and_reflects_the_caret as fn(),
    ),
    (
        "gtk_composer_tracking_notice::replying_to_a_tracking_link_shows_the_notice_and_a_same_domain_link_does_not",
        gtk_composer_tracking_notice::replying_to_a_tracking_link_shows_the_notice_and_a_same_domain_link_does_not as fn(),
    ),
    (
        "gtk_cursor_preview::the_cursor_reports_every_row_it_lands_on",
        gtk_cursor_preview::the_cursor_reports_every_row_it_lands_on as fn(),
    ),
    (
        "gtk_dispatch::every_gesture_reaches_the_bus_exactly_once",
        gtk_dispatch::every_gesture_reaches_the_bus_exactly_once as fn(),
    ),
    (
        "gtk_dwell::a_message_is_marked_read_by_resting_on_it_not_by_passing_over_it",
        gtk_dwell::a_message_is_marked_read_by_resting_on_it_not_by_passing_over_it as fn(),
    ),
    (
        "gtk_editable_dialect::webkit_editing_gestures_stay_inside_the_canonical_subset",
        gtk_editable_dialect::webkit_editing_gestures_stay_inside_the_canonical_subset as fn(),
    ),
    (
        "gtk_editor_bridge::an_edit_becomes_the_document_and_undo_walks_typing_runs",
        gtk_editor_bridge::an_edit_becomes_the_document_and_undo_walks_typing_runs as fn(),
    ),
    (
        "gtk_editor_format::every_formatting_command_lands_as_canonical_structure",
        gtk_editor_format::every_formatting_command_lands_as_canonical_structure as fn(),
    ),
    (
        "gtk_editor_images::inline_images_render_from_the_blob_store_and_remote_ones_never_load",
        gtk_editor_images::inline_images_render_from_the_blob_store_and_remote_ones_never_load as fn(),
    ),
    (
        "gtk_editor_profile::the_editing_profile_runs_our_script_and_nothing_else",
        gtk_editor_profile::the_editing_profile_runs_our_script_and_nothing_else as fn(),
    ),
    (
        "gtk_finder_focus::leaving_the_search_field_gives_the_single_key_bindings_back",
        gtk_finder_focus::leaving_the_search_field_gives_the_single_key_bindings_back as fn(),
    ),
    (
        "gtk_folder_sections::the_feed_reads_every_account_it_is_given_and_keeps_their_order",
        gtk_folder_sections::the_feed_reads_every_account_it_is_given_and_keeps_their_order as fn(),
    ),
    (
        "gtk_identity::the_reply_comes_from_the_address_it_was_sent_to_and_signs_once",
        gtk_identity::the_reply_comes_from_the_address_it_was_sent_to_and_signs_once as fn(),
    ),
    (
        "gtk_list_select_message::select_message_lands_the_cursor_once_the_row_is_resident",
        gtk_list_select_message::select_message_lands_the_cursor_once_the_row_is_resident as fn(),
    ),
    (
        "gtk_list_state::offline_becomes_a_banner_over_rows_and_a_full_plate_over_none",
        gtk_list_state::offline_becomes_a_banner_over_rows_and_a_full_plate_over_none as fn(),
    ),
    (
        "gtk_live_config::editing_config_toml_rebinds_the_running_window",
        gtk_live_config::editing_config_toml_rebinds_the_running_window as fn(),
    ),
    (
        "gtk_onboarding::a_repair_arrives_with_the_address_and_the_servers_already_filled_in",
        gtk_onboarding::a_repair_arrives_with_the_address_and_the_servers_already_filled_in as fn(),
    ),
    (
        "gtk_onboarding_enter::return_does_the_right_thing_in_every_field",
        gtk_onboarding_enter::return_does_the_right_thing_in_every_field as fn(),
    ),
    (
        "gtk_onboarding_guess::a_guess_fills_the_manual_form_and_opens_it",
        gtk_onboarding_guess::a_guess_fills_the_manual_form_and_opens_it as fn(),
    ),
    (
        "gtk_onboarding_name::a_typed_name_reaches_the_submission_and_a_blank_one_stays_empty",
        gtk_onboarding_name::a_typed_name_reaches_the_submission_and_a_blank_one_stays_empty as fn(),
    ),
    (
        "gtk_reader_account::the_header_names_the_account_only_when_there_is_more_than_one",
        gtk_reader_account::the_header_names_the_account_only_when_there_is_more_than_one as fn(),
    ),
    (
        "gtk_reader_actions::the_action_bar_follows_the_pane_carries_the_keymap_and_runs_registry_commands",
        gtk_reader_actions::the_action_bar_follows_the_pane_carries_the_keymap_and_runs_registry_commands as fn(),
    ),
    (
        "gtk_reader_fonts::the_faces_are_fetched_over_the_scheme_and_not_carried_by_the_document",
        gtk_reader_fonts::the_faces_are_fetched_over_the_scheme_and_not_carried_by_the_document as fn(),
    ),
    (
        "gtk_shell::the_plate_layout_matches_the_canvas",
        gtk_shell::the_plate_layout_matches_the_canvas as fn(),
    ),
    (
        "gtk_shell::nothing_in_the_stylesheet_outruns_the_motion_budget",
        gtk_shell::nothing_in_the_stylesheet_outruns_the_motion_budget as fn(),
    ),
    (
        "gtk_sidebar_accounts::the_strip_names_every_account_and_is_absent_with_one",
        gtk_sidebar_accounts::the_strip_names_every_account_and_is_absent_with_one as fn(),
    ),
    (
        "gtk_sidebar_height::a_sidebar_full_of_folders_still_fits_in_the_window",
        gtk_sidebar_height::a_sidebar_full_of_folders_still_fits_in_the_window as fn(),
    ),
    (
        "gtk_sidebar_sections::each_account_folds_away_its_own_folders",
        gtk_sidebar_sections::each_account_folds_away_its_own_folders as fn(),
    ),
    (
        "gtk_signature_placement::the_configured_placement_decides_which_side_of_the_quote_signs",
        gtk_signature_placement::the_configured_placement_decides_which_side_of_the_quote_signs as fn(),
    ),
    (
        "gtk_toast::the_undo_toast_coalesces_and_offers_undo_only_when_there_is_something_to_undo",
        gtk_toast::the_undo_toast_coalesces_and_offers_undo_only_when_there_is_something_to_undo as fn(),
    ),
    (
        "gtk_unavailable::the_screen_shows_what_it_was_told_and_asks_to_try_again_once",
        gtk_unavailable::the_screen_shows_what_it_was_told_and_asks_to_try_again_once as fn(),
    ),
    (
        "gtk_undo_toast::u_and_the_toasts_button_both_reach_command_id_undo",
        gtk_undo_toast::u_and_the_toasts_button_both_reach_command_id_undo as fn(),
    ),
    (
        "gtk_window_open_message::open_mailbox_and_open_message_switch_the_window_from_outside",
        gtk_window_open_message::open_mailbox_and_open_message_switch_the_window_from_outside as fn(),
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
