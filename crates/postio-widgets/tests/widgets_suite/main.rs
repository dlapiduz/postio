//! One binary for the GTK cases of the crate both desktop apps draw with
//! (ADR 0043) -- the custom harness of postio-gtk's `gtk_suite` and
//! postio-app's `app_suite`, for the same two reasons:
//!
//!   * GTK may be initialized from exactly one thread per process (#41), and
//!     libtest runs `#[test]` functions on a thread pool;
//!   * every extra test *binary* links the whole GTK stack, and linking was
//!     once the dominant cost of a GTK crate's tests (#329).
//!
//! So: `harness = false`, one `adw::init`, every case a plain `pub fn` in
//! `widgets_suite/`, run in sequence under `catch_unwind` so one failure does
//! not hide the rest. See crates/postio-gtk/tests/gtk_suite/main.rs for the
//! whole rationale.
//!
//! **A new case is a module here and a row in `CASES`.** `--list` and name
//! filtering behave enough like libtest for `cargo test`, nextest and the
//! tooling's test counting to work, and that output is a contract:
//! `list_contract.rs` is what notices when it breaks, because a runner that
//! misreads it runs nothing and reports success.
//!
//! A panicking case can leave toolkit state behind that fails a later case:
//! when several cases fail at once, trust the first.

mod body_view_fallback;
mod body_view_highlight;
mod body_view_resets;
mod capture;
mod drag_out;
mod harness;
mod jank;
mod list_contract;
mod list_model_generic;
mod pickers;
mod present_compose;
mod present_config;
mod present_onboarding;
mod present_reading;
mod quote_folds;
mod reader_treatment;
mod reader_verbs;
mod recipient_chips;
mod support;
mod body_view;
mod body_view_a11y;
mod body_view_find;
mod body_view_select;
mod body_view_theme;
mod body_view_zoom;
mod reader_fallback;
mod reader_fonts;
mod reader_teardown;
mod reader_scroll;
mod reader_anchor;
mod reader_notices;
mod reader_corpus;
mod list_model;
mod list_recycling;
mod one_allowlist;
mod small_widgets;
mod checkrow;
mod segmented;
mod toast;
mod toast_tone_and_undo;
mod first_frame;
mod components;
mod support_reader;
mod support_compose;
mod composer_attachments;
mod composer_autosave;
mod composer_confirms;
mod composer_document;
mod composer_focus;
mod composer_host;
mod composer_inline_image;
mod composer_markdown;
mod composer_recipient_select;
mod composer_recipients;
mod composer_resume;
mod composer_schedule_send;
mod composer_signature_default;
mod composer_size;
mod composer_toolbar;
mod composer_tracking_notice;
mod composer_warm;
mod editable_dialect;
mod editor_appearance;
mod editor_bridge;
mod editor_format;
mod editor_images;
mod editor_markdown;
mod editor_profile;
mod editor_teardown;
mod onboarding;
mod onboarding_enter;
mod onboarding_guess;
mod onboarding_name;
mod onboarding_sync_window;
mod settings_account_detail;
mod settings_accounts;
mod settings_filters;
mod settings_frame;
mod settings_keys;
mod settings_privacy;
mod settings_sync;
mod widgets_css;

/// Cases held out of a default run, by name -- the table-driven spelling of
/// `#[ignore]`, which means one thing here: this machine may not have what
/// the case needs. A name here still runs when asked for explicitly, and still
/// appears in `--list`, exactly as an ignored libtest case does. Say in a
/// comment beside the name which issue or task takes it back.
const IGNORED: &[&str] = &[]; // nothing held out

const CASES: &[(&str, fn())] = &[
    (
        "jank::a_blocked_main_loop_is_reported_with_the_action_before_it",
        jank::a_blocked_main_loop_is_reported_with_the_action_before_it as fn(),
    ),
    (
        "drag_out::nothing_is_written_until_the_drop_asks",
        drag_out::nothing_is_written_until_the_drop_asks as fn(),
    ),
    (
        "drag_out::the_sandboxed_spelling_is_offered_too",
        drag_out::the_sandboxed_spelling_is_offered_too as fn(),
    ),
    (
        "drag_out::an_export_that_produced_nothing_refuses_the_drop",
        drag_out::an_export_that_produced_nothing_refuses_the_drop as fn(),
    ),
    (
        "drag_out::a_failed_export_fails_the_drop",
        drag_out::a_failed_export_fails_the_drop as fn(),
    ),
    (
        "reader_treatment::a_reader_reports_its_treatment_and_tells_when_it_changes",
        reader_treatment::a_reader_reports_its_treatment_and_tells_when_it_changes as fn(),
    ),
    (
        "reader_treatment::a_reader_that_draws_no_treatments_answers_app_colours",
        reader_treatment::a_reader_that_draws_no_treatments_answers_app_colours as fn(),
    ),
    (
        "reader_verbs::a_header_can_leave_the_subject_to_its_surface",
        reader_verbs::a_header_can_leave_the_subject_to_its_surface as fn(),
    ),
    (
        "reader_verbs::a_card_placed_under_the_header_sits_between_it_and_the_body",
        reader_verbs::a_card_placed_under_the_header_sits_between_it_and_the_body as fn(),
    ),
    (
        "quote_folds::a_quote_is_folded_behind_its_line_count_and_opens_when_clicked",
        quote_folds::a_quote_is_folded_behind_its_line_count_and_opens_when_clicked as fn(),
    ),
    (
        "quote_folds::the_dialog_s_fold_line_opens_the_quote_it_names",
        quote_folds::the_dialog_s_fold_line_opens_the_quote_it_names as fn(),
    ),
    (
        "body_view_highlight::a_highlighted_range_is_drawn_over_its_rectangles_and_scrolled_into_view",
        body_view_highlight::a_highlighted_range_is_drawn_over_its_rectangles_and_scrolled_into_view
            as fn(),
    ),
    (
        "recipient_chips::choosing_a_suggestion_adds_a_chip_with_the_name_and_the_address",
        recipient_chips::choosing_a_suggestion_adds_a_chip_with_the_name_and_the_address as fn(),
    ),
    (
        "recipient_chips::a_chip_s_remove_button_takes_the_recipient_off",
        recipient_chips::a_chip_s_remove_button_takes_the_recipient_off as fn(),
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
        "body_view_fallback::a_fallback_is_laid_out_as_a_plain_text_body",
        body_view_fallback::a_fallback_is_laid_out_as_a_plain_text_body as fn(),
    ),
    (
        "body_view_fallback::the_render_mode_line_says_a_body_fell_back",
        body_view_fallback::the_render_mode_line_says_a_body_fell_back as fn(),
    ),
    (
        "body_view_fallback::a_finished_render_is_shown_when_the_main_loop_was_late",
        body_view_fallback::a_finished_render_is_shown_when_the_main_loop_was_late as fn(),
    ),
    (
        "body_view_resets::a_message_shown_after_a_darkened_one_is_not_darkened",
        body_view_resets::a_message_shown_after_a_darkened_one_is_not_darkened as fn(),
    ),
    (
        "body_view_resets::a_message_shown_after_another_has_no_selection_and_no_focused_link",
        body_view_resets::a_message_shown_after_another_has_no_selection_and_no_focused_link
            as fn(),
    ),
    (
        "body_view_resets::a_message_shown_again_folds_as_it_was_sent",
        body_view_resets::a_message_shown_again_folds_as_it_was_sent as fn(),
    ),
    (
        "widgets_css::the_shared_sheet_dresses_the_shared_widgets",
        widgets_css::the_shared_sheet_dresses_the_shared_widgets as fn(),
    ),
    (
        "widgets_css::the_shared_sheet_dresses_the_account_form",
        widgets_css::the_shared_sheet_dresses_the_account_form as fn(),
    ),
    (
        "widgets_css::the_shared_sheet_brings_the_shared_metrics",
        widgets_css::the_shared_sheet_brings_the_shared_metrics as fn(),
    ),
    (
        "reader_verbs::a_reader_draws_the_verbs_it_is_given_and_none_when_given_none",
        reader_verbs::a_reader_draws_the_verbs_it_is_given_and_none_when_given_none as fn(),
    ),
    (
        "list_model_generic::a_list_of_another_row_type_is_windowed_filled_and_refreshed",
        list_model_generic::a_list_of_another_row_type_is_windowed_filled_and_refreshed as fn(),
    ),
    (
        "present_config::an_edit_reaches_the_app_and_a_broken_one_keeps_the_last_good_keys",
        present_config::an_edit_reaches_the_app_and_a_broken_one_keeps_the_last_good_keys as fn(),
    ),
    (
        "present_compose::dispatching_send_queues_the_draft_through_the_client",
        present_compose::dispatching_send_queues_the_draft_through_the_client as fn(),
    ),
    (
        "present_onboarding::the_credential_dialog_reads_the_account_and_saves_through_the_client",
        present_onboarding::the_credential_dialog_reads_the_account_and_saves_through_the_client
            as fn(),
    ),
    (
        "present_reading::fetched_images_come_back_under_the_documents_spelling",
        present_reading::fetched_images_come_back_under_the_documents_spelling as fn(),
    ),
    (
        "present_reading::a_cid_reference_resolves_through_the_client_for_the_message_on_screen",
        present_reading::a_cid_reference_resolves_through_the_client_for_the_message_on_screen
            as fn(),
    ),
    (
        "present_reading::nothing_showing_resolves_to_nothing_without_asking_the_host",
        present_reading::nothing_showing_resolves_to_nothing_without_asking_the_host as fn(),
    ),
    (
        "capture::a_window_the_compositor_never_showed_is_an_error",
        capture::a_window_the_compositor_never_showed_is_an_error as fn(),
    ),
    (
        "pickers::two_picks_the_second_preset_and_tab_focuses_the_date_entry",
        pickers::two_picks_the_second_preset_and_tab_focuses_the_date_entry as fn(),
    ),
    (
        "capture::an_open_popover_is_in_the_picture",
        capture::an_open_popover_is_in_the_picture as fn(),
    ),
    (
        "capture::a_capture_that_fails_leaves_no_file",
        capture::a_capture_that_fails_leaves_no_file as fn(),
    ),
    (
        "capture::a_presented_window_is_captured_without_the_caller_counting_frames",
        capture::a_presented_window_is_captured_without_the_caller_counting_frames as fn(),
    ),
    (
        "composer_attachments::attaching_shows_the_row_and_removing_cleans_it_up",
        composer_attachments::attaching_shows_the_row_and_removing_cleans_it_up as fn(),
    ),
    (
        "composer_autosave::typing_debounces_into_one_autosave_and_closing_flushes_what_is_pending",
        composer_autosave::typing_debounces_into_one_autosave_and_closing_flushes_what_is_pending as fn(),
    ),
    (
        "composer_autosave::saving_twice_carries_the_assigned_id_forward_into_the_second_save",
        composer_autosave::saving_twice_carries_the_assigned_id_forward_into_the_second_save as fn(),
    ),
    (
        "composer_confirms::the_composer_asks_before_the_two_things_it_cannot_take_back",
        composer_confirms::the_composer_asks_before_the_two_things_it_cannot_take_back as fn(),
    ),
    (
        "composer_document::the_body_round_trips_through_the_neutral_document",
        composer_document::the_body_round_trips_through_the_neutral_document as fn(),
    ),
    (
        "composer_focus::focus_lands_when_the_composer_opens_before_the_window_is_ever_mapped",
        composer_focus::focus_lands_when_the_composer_opens_before_the_window_is_ever_mapped as fn(),
    ),
    (
        "composer_focus::the_keyboard_walks_the_composer_in_a_defined_order",
        composer_focus::the_keyboard_walks_the_composer_in_a_defined_order as fn(),
    ),
    (
        "composer_host::a_composer_on_a_test_host_autosaves_and_sends",
        composer_host::a_composer_on_a_test_host_autosaves_and_sends as fn(),
    ),
    (
        "composer_inline_image::a_pasted_image_becomes_an_inline_attachment_and_renders_at_the_caret",
        composer_inline_image::a_pasted_image_becomes_an_inline_attachment_and_renders_at_the_caret as fn(),
    ),
    (
        "composer_markdown::a_draft_the_desktop_saves_carries_no_markdown",
        composer_markdown::a_draft_the_desktop_saves_carries_no_markdown as fn(),
    ),
    (
        "composer_recipients::typing_a_prefix_offers_suggestions_and_accepting_one_completes_it",
        composer_recipients::typing_a_prefix_offers_suggestions_and_accepting_one_completes_it as fn(),
    ),
    (
        "composer_recipients::revealing_cc_and_bcc_keeps_what_was_already_typed",
        composer_recipients::revealing_cc_and_bcc_keeps_what_was_already_typed as fn(),
    ),
    (
        "composer_recipients::cc_and_bcc_put_themselves_away_only_while_they_are_empty",
        composer_recipients::cc_and_bcc_put_themselves_away_only_while_they_are_empty as fn(),
    ),
    (
        "composer_recipient_select::destroying_a_composer_releases_recipient_completion",
        composer_recipient_select::destroying_a_composer_releases_recipient_completion as fn(),
    ),
    (
        "composer_recipient_select::clicking_a_suggestion_puts_that_one_in_the_field",
        composer_recipient_select::clicking_a_suggestion_puts_that_one_in_the_field as fn(),
    ),
    (
        "composer_recipient_select::return_commits_the_suggestion_the_popover_has_selected",
        composer_recipient_select::return_commits_the_suggestion_the_popover_has_selected as fn(),
    ),
    (
        "composer_recipient_select::accepting_a_group_inserts_every_member",
        composer_recipient_select::accepting_a_group_inserts_every_member as fn(),
    ),
    (
        "composer_recipient_select::nothing_is_offered_until_four_characters_are_typed",
        composer_recipient_select::nothing_is_offered_until_four_characters_are_typed as fn(),
    ),
    (
        "composer_resume::resuming_replaces_the_draft_the_composer_was_holding",
        composer_resume::resuming_replaces_the_draft_the_composer_was_holding as fn(),
    ),
    (
        "composer_resume::composing_after_a_kept_draft_starts_blank",
        composer_resume::composing_after_a_kept_draft_starts_blank as fn(),
    ),
    (
        "composer_resume::resuming_an_unsaved_draft_over_another_unsaved_one_replaces_it",
        composer_resume::resuming_an_unsaved_draft_over_another_unsaved_one_replaces_it as fn(),
    ),
    (
        "composer_resume::reopening_restores_the_formatting_and_the_attachments_too",
        composer_resume::reopening_restores_the_formatting_and_the_attachments_too as fn(),
    ),
    (
        "composer_schedule_send::ctrl_shift_return_opens_the_schedule_send_picker",
        composer_schedule_send::ctrl_shift_return_opens_the_schedule_send_picker as fn(),
    ),
    (
        "composer_signature_default::a_resolved_signature_wins_over_the_identity_s_own",
        composer_signature_default::a_resolved_signature_wins_over_the_identity_s_own as fn(),
    ),
    (
        "composer_signature_default::a_resolved_signature_the_account_does_not_have_falls_back_to_the_identity",
        composer_signature_default::a_resolved_signature_the_account_does_not_have_falls_back_to_the_identity as fn(),
    ),
    (
        "composer_signature_default::no_resolution_resets_a_picker_a_previous_compose_left_pointed_elsewhere",
        composer_signature_default::no_resolution_resets_a_picker_a_previous_compose_left_pointed_elsewhere as fn(),
    ),
    (
        "composer_size::an_oversize_draft_is_refused_before_it_reaches_the_send_handler",
        composer_size::an_oversize_draft_is_refused_before_it_reaches_the_send_handler as fn(),
    ),
    (
        "composer_toolbar::the_toolbar_reaches_the_registry_commands_and_reflects_the_caret",
        composer_toolbar::the_toolbar_reaches_the_registry_commands_and_reflects_the_caret as fn(),
    ),
    (
        "composer_toolbar::the_toolbar_offers_attaching_a_file",
        composer_toolbar::the_toolbar_offers_attaching_a_file as fn(),
    ),
    (
        "composer_tracking_notice::replying_to_a_tracking_link_shows_the_notice_and_a_same_domain_link_does_not",
        composer_tracking_notice::replying_to_a_tracking_link_shows_the_notice_and_a_same_domain_link_does_not as fn(),
    ),
    (
        "composer_warm::the_editing_surface_can_be_warmed_before_anyone_composes",
        composer_warm::the_editing_surface_can_be_warmed_before_anyone_composes as fn(),
    ),
    (
        "editor_appearance::the_editing_surface_is_dark_in_dark_mode_and_never_white",
        editor_appearance::the_editing_surface_is_dark_in_dark_mode_and_never_white as fn(),
    ),
    (
        "editor_bridge::an_edit_becomes_the_document_and_undo_walks_typing_runs",
        editor_bridge::an_edit_becomes_the_document_and_undo_walks_typing_runs as fn(),
    ),
    (
        "editor_bridge::the_editor_page_says_when_its_script_is_listening",
        editor_bridge::the_editor_page_says_when_its_script_is_listening as fn(),
    ),
    (
        "editor_format::every_formatting_command_lands_as_canonical_structure",
        editor_format::every_formatting_command_lands_as_canonical_structure as fn(),
    ),
    (
        "editor_images::inline_images_render_from_the_blob_store_and_remote_ones_never_load",
        editor_images::inline_images_render_from_the_blob_store_and_remote_ones_never_load as fn(),
    ),
    (
        "editor_markdown::typed_markdown_becomes_the_formatting_its_command_produces",
        editor_markdown::typed_markdown_becomes_the_formatting_its_command_produces as fn(),
    ),
    (
        "editor_profile::the_editing_profile_runs_our_script_and_nothing_else",
        editor_profile::the_editing_profile_runs_our_script_and_nothing_else as fn(),
    ),
    (
        "editor_teardown::closing_editors_releases_their_webviews",
        editor_teardown::closing_editors_releases_their_webviews as fn(),
    ),
    (
        "editable_dialect::webkit_editing_gestures_stay_inside_the_canonical_subset",
        editable_dialect::webkit_editing_gestures_stay_inside_the_canonical_subset as fn(),
    ),
    (
        "onboarding::a_repair_arrives_with_the_address_and_the_servers_already_filled_in",
        onboarding::a_repair_arrives_with_the_address_and_the_servers_already_filled_in as fn(),
    ),
    (
        "onboarding_enter::return_does_the_right_thing_in_every_field",
        onboarding_enter::return_does_the_right_thing_in_every_field as fn(),
    ),
    (
        "onboarding_guess::a_guess_fills_the_manual_form_and_opens_it",
        onboarding_guess::a_guess_fills_the_manual_form_and_opens_it as fn(),
    ),
    (
        "onboarding_name::a_typed_name_reaches_the_submission_and_a_blank_one_stays_empty",
        onboarding_name::a_typed_name_reaches_the_submission_and_a_blank_one_stays_empty as fn(),
    ),
    (
        "onboarding_sync_window::picking_a_window_updates_the_estimate_and_start_sync_fires_it",
        onboarding_sync_window::picking_a_window_updates_the_estimate_and_start_sync_fires_it as fn(),
    ),
    (
        "settings_accounts::accounts_render_as_rows_and_hide_when_there_are_none",
        settings_accounts::accounts_render_as_rows_and_hide_when_there_are_none as fn(),
    ),
    (
        "settings_accounts::flipping_the_switch_reports_the_account_and_the_new_state",
        settings_accounts::flipping_the_switch_reports_the_account_and_the_new_state as fn(),
    ),
    (
        "settings_accounts::the_context_menu_reaches_the_action_handler_with_the_right_account",
        settings_accounts::the_context_menu_reaches_the_action_handler_with_the_right_account as fn(),
    ),
    (
        "settings_accounts::an_account_row_says_what_its_mail_weighs",
        settings_accounts::an_account_row_says_what_its_mail_weighs as fn(),
    ),
    (
        "settings_accounts::an_account_row_says_how_it_connects",
        settings_accounts::an_account_row_says_how_it_connects as fn(),
    ),
    (
        "settings_accounts::an_account_row_says_whether_its_token_is_still_good",
        settings_accounts::an_account_row_says_whether_its_token_is_still_good as fn(),
    ),
    (
        "settings_accounts::an_account_row_says_when_its_search_index_is_being_rebuilt",
        settings_accounts::an_account_row_says_when_its_search_index_is_being_rebuilt as fn(),
    ),
    (
        "settings_accounts::the_default_account_says_so_and_the_others_do_not",
        settings_accounts::the_default_account_says_so_and_the_others_do_not as fn(),
    ),
    (
        "settings_account_detail::activating_a_row_opens_the_detail_view_with_its_current_settings",
        settings_account_detail::activating_a_row_opens_the_detail_view_with_its_current_settings as fn(),
    ),
    (
        "settings_account_detail::the_form_appears_under_the_list_rather_than_in_place_of_it",
        settings_account_detail::the_form_appears_under_the_list_rather_than_in_place_of_it as fn(),
    ),
    (
        "settings_account_detail::editing_the_display_name_reports_the_account_and_the_new_value",
        settings_account_detail::editing_the_display_name_reports_the_account_and_the_new_value as fn(),
    ),
    (
        "settings_account_detail::editing_the_imap_port_reports_the_account_and_the_new_value",
        settings_account_detail::editing_the_imap_port_reports_the_account_and_the_new_value as fn(),
    ),
    (
        "settings_account_detail::opening_a_second_account_populates_its_own_settings_not_the_firsts",
        settings_account_detail::opening_a_second_account_populates_its_own_settings_not_the_firsts as fn(),
    ),
    (
        "settings_account_detail::the_detail_view_offers_the_accounts_signatures_and_starts_on_its_default",
        settings_account_detail::the_detail_view_offers_the_accounts_signatures_and_starts_on_its_default as fn(),
    ),
    (
        "settings_account_detail::an_account_with_signatures_and_no_default_opens_on_none",
        settings_account_detail::an_account_with_signatures_and_no_default_opens_on_none as fn(),
    ),
    (
        "settings_account_detail::an_account_with_no_signatures_gets_no_picker_at_all",
        settings_account_detail::an_account_with_no_signatures_gets_no_picker_at_all as fn(),
    ),
    (
        "settings_account_detail::choosing_a_signature_reports_the_account_and_the_choice",
        settings_account_detail::choosing_a_signature_reports_the_account_and_the_choice as fn(),
    ),
    (
        "settings_account_detail::opening_an_account_reports_no_edit_of_its_own",
        settings_account_detail::opening_an_account_reports_no_edit_of_its_own as fn(),
    ),
    (
        "settings_account_detail::test_connection_reports_the_account_and_then_shows_what_happened",
        settings_account_detail::test_connection_reports_the_account_and_then_shows_what_happened as fn(),
    ),
    (
        "settings_account_detail::the_mailboxes_group_offers_automatic_first_and_names_what_it_resolved_to",
        settings_account_detail::the_mailboxes_group_offers_automatic_first_and_names_what_it_resolved_to as fn(),
    ),
    (
        "settings_account_detail::signatures_can_be_added_edited_and_deleted_from_the_detail_view",
        settings_account_detail::signatures_can_be_added_edited_and_deleted_from_the_detail_view as fn(),
    ),
    (
        "settings_account_detail::picking_a_folder_for_a_role_reports_the_account_and_the_path",
        settings_account_detail::picking_a_folder_for_a_role_reports_the_account_and_the_path as fn(),
    ),
    (
        "settings_account_detail::a_mapping_the_server_no_longer_has_is_shown_rather_than_dropped",
        settings_account_detail::a_mapping_the_server_no_longer_has_is_shown_rather_than_dropped as fn(),
    ),
    (
        "settings_account_detail::an_account_with_no_folders_yet_says_so_instead_of_offering_nothing",
        settings_account_detail::an_account_with_no_folders_yet_says_so_instead_of_offering_nothing as fn(),
    ),
    (
        "settings_account_detail::a_role_the_server_refused_to_create_says_so_and_says_why",
        settings_account_detail::a_role_the_server_refused_to_create_says_so_and_says_why as fn(),
    ),
    (
        "settings_filters::filters_render_as_rows_and_hide_when_there_are_none",
        settings_filters::filters_render_as_rows_and_hide_when_there_are_none as fn(),
    ),
    (
        "settings_filters::pinned_filters_come_first_in_order_then_unpinned_ones_alphabetically",
        settings_filters::pinned_filters_come_first_in_order_then_unpinned_ones_alphabetically as fn(),
    ),
    (
        "settings_filters::toggling_pinned_writes_straight_to_the_buffer",
        settings_filters::toggling_pinned_writes_straight_to_the_buffer as fn(),
    ),
    (
        "settings_filters::deleting_a_filter_removes_its_row_and_leaves_everything_else_alone",
        settings_filters::deleting_a_filter_removes_its_row_and_leaves_everything_else_alone as fn(),
    ),
    (
        "settings_filters::reordering_moves_a_pinned_filter_and_disables_at_the_ends",
        settings_filters::reordering_moves_a_pinned_filter_and_disables_at_the_ends as fn(),
    ),
    (
        "settings_frame::exactly_one_pane_is_ever_on_screen",
        settings_frame::exactly_one_pane_is_ever_on_screen as fn(),
    ),
    (
        "settings_frame::the_sidebar_selection_is_what_chooses_the_pane",
        settings_frame::the_sidebar_selection_is_what_chooses_the_pane as fn(),
    ),
    (
        "settings_frame::the_frame_is_identical_on_every_pane",
        settings_frame::the_frame_is_identical_on_every_pane as fn(),
    ),
    (
        "settings_frame::the_footer_names_the_table_the_pane_writes",
        settings_frame::the_footer_names_the_table_the_pane_writes as fn(),
    ),
    (
        "settings_frame::the_sidebar_groups_its_sections_under_two_headings",
        settings_frame::the_sidebar_groups_its_sections_under_two_headings as fn(),
    ),
    (
        "settings_frame::finding_a_setting_narrows_the_sidebar_to_the_panes_that_have_it",
        settings_frame::finding_a_setting_narrows_the_sidebar_to_the_panes_that_have_it as fn(),
    ),
    (
        "settings_keys::rows_render_one_per_command_with_its_current_binding",
        settings_keys::rows_render_one_per_command_with_its_current_binding as fn(),
    ),
    (
        "settings_keys::an_override_in_the_file_is_what_the_row_shows",
        settings_keys::an_override_in_the_file_is_what_the_row_shows as fn(),
    ),
    (
        "settings_keys::capturing_a_free_key_writes_the_new_binding_to_the_buffer",
        settings_keys::capturing_a_free_key_writes_the_new_binding_to_the_buffer as fn(),
    ),
    (
        "settings_keys::capturing_a_binding_already_in_use_is_surfaced_not_silently_overwritten",
        settings_keys::capturing_a_binding_already_in_use_is_surfaced_not_silently_overwritten as fn(),
    ),
    (
        "settings_keys::escape_cancels_capture_without_changing_anything",
        settings_keys::escape_cancels_capture_without_changing_anything as fn(),
    ),
    (
        "settings_privacy::allowed_senders_render_as_rows_and_hide_when_there_are_none",
        settings_privacy::allowed_senders_render_as_rows_and_hide_when_there_are_none as fn(),
    ),
    (
        "settings_privacy::every_allowed_sender_gets_its_own_row",
        settings_privacy::every_allowed_sender_gets_its_own_row as fn(),
    ),
    (
        "settings_privacy::revoking_a_sender_removes_its_row_and_persists",
        settings_privacy::revoking_a_sender_removes_its_row_and_persists as fn(),
    ),
    (
        "settings_privacy::no_activations_hides_the_unsubscribe_section_and_shows_the_empty_state",
        settings_privacy::no_activations_hides_the_unsubscribe_section_and_shows_the_empty_state as fn(),
    ),
    (
        "settings_privacy::every_activation_gets_its_own_row_newest_first",
        settings_privacy::every_activation_gets_its_own_row_newest_first as fn(),
    ),
    (
        "settings_privacy::the_read_receipt_count_states_zero_rather_than_going_blank",
        settings_privacy::the_read_receipt_count_states_zero_rather_than_going_blank as fn(),
    ),
    (
        "settings_privacy::the_read_receipt_count_states_the_number_and_says_none_are_sent",
        settings_privacy::the_read_receipt_count_states_the_number_and_says_none_are_sent as fn(),
    ),
    (
        "settings_sync::the_pane_shows_the_files_values",
        settings_sync::the_pane_shows_the_files_values as fn(),
    ),
    (
        "settings_sync::the_interval_the_file_actually_holds_is_still_stated",
        settings_sync::the_interval_the_file_actually_holds_is_still_stated as fn(),
    ),
    (
        "settings_sync::pressing_manual_writes_straight_to_the_buffer_and_leaves_the_rest_alone",
        settings_sync::pressing_manual_writes_straight_to_the_buffer_and_leaves_the_rest_alone as fn(),
    ),
    (
        "settings_sync::an_interval_somebody_set_by_hand_survives_pressing_the_segment_it_is_on",
        settings_sync::an_interval_somebody_set_by_hand_survives_pressing_the_segment_it_is_on as fn(),
    ),
    (
        "settings_sync::typing_new_roles_and_pressing_enter_writes_the_new_list",
        settings_sync::typing_new_roles_and_pressing_enter_writes_the_new_list as fn(),
    ),
    (
        "body_view::a_snapshot_fills_the_view_and_scrolls_with_it",
        body_view::a_snapshot_fills_the_view_and_scrolls_with_it as fn(),
    ),
    (
        "body_view::no_frame_shows_only_the_ground",
        body_view::no_frame_shows_only_the_ground as fn(),
    ),
    (
        "body_view::a_render_past_its_deadline_shows_the_plain_text",
        body_view::a_render_past_its_deadline_shows_the_plain_text as fn(),
    ),
    (
        "body_view::a_very_tall_message_scrolls_to_its_end_within_budget",
        body_view::a_very_tall_message_scrolls_to_its_end_within_budget as fn(),
    ),
    (
        "body_view_a11y::the_accessible_text_is_the_text_index",
        body_view_a11y::the_accessible_text_is_the_text_index as fn(),
    ),
    (
        "body_view_find::find_highlights_steps_and_survives_a_re_render",
        body_view_find::find_highlights_steps_and_survives_a_re_render as fn(),
    ),
    (
        "body_view_select::a_drag_across_cells_selects_and_copies_them_as_rows",
        body_view_select::a_drag_across_cells_selects_and_copies_them_as_rows as fn(),
    ),
    (
        "body_view_select::double_and_triple_clicks_select_a_word_and_a_line",
        body_view_select::double_and_triple_clicks_select_a_word_and_a_line as fn(),
    ),
    (
        "body_view_select::links_are_followed_by_pointer_and_keyboard",
        body_view_select::links_are_followed_by_pointer_and_keyboard as fn(),
    ),
    (
        "body_view_select::clicking_a_fold_opens_it",
        body_view_select::clicking_a_fold_opens_it as fn(),
    ),
    (
        "body_view_select::the_rail_follows_the_snapshot",
        body_view_select::the_rail_follows_the_snapshot as fn(),
    ),
    (
        "body_view_theme::a_theme_change_re_renders_once_and_keeps_the_place",
        body_view_theme::a_theme_change_re_renders_once_and_keeps_the_place as fn(),
    ),
    (
        "body_view_zoom::zoom_steps_keep_the_place_and_the_selection",
        body_view_zoom::zoom_steps_keep_the_place_and_the_selection as fn(),
    ),
    (
        "body_view_zoom::a_pinch_snaps_to_a_step_and_renders_once",
        body_view_zoom::a_pinch_snaps_to_a_step_and_renders_once as fn(),
    ),
    (
        "reader_fallback::a_message_past_its_deadline_shows_its_own_text",
        reader_fallback::a_message_past_its_deadline_shows_its_own_text as fn(),
    ),
    (
        "reader_fallback::a_conversation_past_its_deadline_shows_each_messages_text",
        reader_fallback::a_conversation_past_its_deadline_shows_each_messages_text as fn(),
    ),
    (
        "reader_fonts::the_faces_are_the_readers_own_and_not_carried_by_the_document",
        reader_fonts::the_faces_are_the_readers_own_and_not_carried_by_the_document as fn(),
    ),
    (
        "reader_fonts::a_warmed_reader_draws_its_first_message_in_full",
        reader_fonts::a_warmed_reader_draws_its_first_message_in_full as fn(),
    ),
    (
        "reader_teardown::a_dropped_reader_releases_its_view",
        reader_teardown::a_dropped_reader_releases_its_view as fn(),
    ),
    (
        "reader_teardown::readers_do_not_accumulate_views",
        reader_teardown::readers_do_not_accumulate_views as fn(),
    ),
    (
        "reader_scroll::page_down_and_page_up_move_a_screen_at_a_time",
        reader_scroll::page_down_and_page_up_move_a_screen_at_a_time as fn(),
    ),
    (
        "reader_scroll::a_new_message_resets_the_scroll_position",
        reader_scroll::a_new_message_resets_the_scroll_position as fn(),
    ),
    (
        "reader_scroll::paging_with_nothing_open_does_nothing",
        reader_scroll::paging_with_nothing_open_does_nothing as fn(),
    ),
    (
        "reader_anchor::showing_a_messages_images_keeps_its_place",
        reader_anchor::showing_a_messages_images_keeps_its_place as fn(),
    ),
    (
        "reader_notices::the_body_starts_at_the_same_place_whatever_the_notices",
        reader_notices::the_body_starts_at_the_same_place_whatever_the_notices as fn(),
    ),
    (
        "reader_notices::a_waiting_plate_carries_no_notice_from_the_message_before",
        reader_notices::a_waiting_plate_carries_no_notice_from_the_message_before as fn(),
    ),
    (
        "reader_notices::the_body_starts_at_the_same_place_whoever_the_message_went_to",
        reader_notices::the_body_starts_at_the_same_place_whoever_the_message_went_to as fn(),
    ),
    (
        "reader_corpus::the_reader_renders_and_hardens_the_corpus",
        reader_corpus::the_reader_renders_and_hardens_the_corpus as fn(),
    ),
    (
        "reader_corpus::the_counters_see_what_the_reader_actually_does",
        reader_corpus::the_counters_see_what_the_reader_actually_does as fn(),
    ),
    (
        "reader_corpus::a_new_reader_loads_nothing_until_it_is_asked_to",
        reader_corpus::a_new_reader_loads_nothing_until_it_is_asked_to as fn(),
    ),
    (
        "list_model::an_empty_list_has_nothing_in_it",
        list_model::an_empty_list_has_nothing_in_it as fn(),
    ),
    (
        "list_model::pointing_at_a_folder_costs_nothing_until_a_row_is_read",
        list_model::pointing_at_a_folder_costs_nothing_until_a_row_is_read as fn(),
    ),
    (
        "list_model::a_row_that_is_not_here_yet_is_a_placeholder_and_a_request",
        list_model::a_row_that_is_not_here_yet_is_a_placeholder_and_a_request as fn(),
    ),
    (
        "list_model::a_page_is_never_asked_for_twice",
        list_model::a_page_is_never_asked_for_twice as fn(),
    ),
    (
        "list_model::scrolling_a_hundred_thousand_messages_keeps_memory_flat",
        list_model::scrolling_a_hundred_thousand_messages_keeps_memory_flat as fn(),
    ),
    (
        "list_model::the_pages_that_go_are_the_ones_nobody_is_looking_at",
        list_model::the_pages_that_go_are_the_ones_nobody_is_looking_at as fn(),
    ),
    (
        "list_model::a_redelivered_page_keeps_the_row_objects_it_already_had",
        list_model::a_redelivered_page_keeps_the_row_objects_it_already_had as fn(),
    ),
    (
        "list_model::a_flag_change_touches_one_row_and_nothing_else",
        list_model::a_flag_change_touches_one_row_and_nothing_else as fn(),
    ),
    (
        "list_model::a_message_that_is_not_on_screen_needs_no_update",
        list_model::a_message_that_is_not_on_screen_needs_no_update as fn(),
    ),
    (
        "list_model::new_mail_arrives_as_an_insertion_at_the_top",
        list_model::new_mail_arrives_as_an_insertion_at_the_top as fn(),
    ),
    (
        "list_model::nothing_happens_when_no_mail_arrives",
        list_model::nothing_happens_when_no_mail_arrives as fn(),
    ),
    (
        "list_model::a_shrinking_folder_drops_the_rows_that_went",
        list_model::a_shrinking_folder_drops_the_rows_that_went as fn(),
    ),
    (
        "list_model::switching_folders_forgets_the_one_before",
        list_model::switching_folders_forgets_the_one_before as fn(),
    ),
    (
        "list_model::a_reordered_list_is_asked_for_again_from_the_top",
        list_model::a_reordered_list_is_asked_for_again_from_the_top as fn(),
    ),
    (
        "list_model::there_is_nothing_past_the_end",
        list_model::there_is_nothing_past_the_end as fn(),
    ),
    (
        "list_model::the_model_can_say_which_page_holds_a_message",
        list_model::the_model_can_say_which_page_holds_a_message as fn(),
    ),
    (
        "list_model::the_model_can_say_where_a_resident_message_sits",
        list_model::the_model_can_say_where_a_resident_message_sits as fn(),
    ),
    (
        "list_model::a_source_that_answers_too_soon_is_held_until_it_is_safe",
        list_model::a_source_that_answers_too_soon_is_held_until_it_is_safe as fn(),
    ),
    (
        "list_model::a_held_delivery_is_not_postponed_by_a_window_that_is_repainting",
        list_model::a_held_delivery_is_not_postponed_by_a_window_that_is_repainting as fn(),
    ),
    (
        "list_model::giving_up_a_first_page_there_is_none_of_ends_the_wait",
        list_model::giving_up_a_first_page_there_is_none_of_ends_the_wait as fn(),
    ),
    (
        "list_recycling::a_list_view_builds_a_bounded_window_however_big_the_model_is",
        list_recycling::a_list_view_builds_a_bounded_window_however_big_the_model_is as fn(),
    ),
    (
        "one_allowlist::two_readers_in_one_app_see_one_always_allow",
        one_allowlist::two_readers_in_one_app_see_one_always_allow as fn(),
    ),
    (
        "one_allowlist::a_revoke_in_settings_reaches_the_apps_readers",
        one_allowlist::a_revoke_in_settings_reaches_the_apps_readers as fn(),
    ),
    (
        "small_widgets::a_keycap_shows_the_key_or_nothing_at_all",
        small_widgets::a_keycap_shows_the_key_or_nothing_at_all as fn(),
    ),
    (
        "small_widgets::an_action_bar_dispatches_the_command_its_cap_advertises",
        small_widgets::an_action_bar_dispatches_the_command_its_cap_advertises as fn(),
    ),
    (
        "small_widgets::a_notice_never_wraps_however_long_the_sentence",
        small_widgets::a_notice_never_wraps_however_long_the_sentence as fn(),
    ),
    (
        "small_widgets::a_notice_overflow_replaces_rather_than_appends",
        small_widgets::a_notice_overflow_replaces_rather_than_appends as fn(),
    ),
    (
        "small_widgets::a_notice_survives_an_overflow_entry_that_rebuilds_the_menu",
        small_widgets::a_notice_survives_an_overflow_entry_that_rebuilds_the_menu as fn(),
    ),
    (
        "small_widgets::the_blocked_images_notice_counts_and_elides",
        small_widgets::the_blocked_images_notice_counts_and_elides as fn(),
    ),
    (
        "small_widgets::every_chip_measures_the_same_height",
        small_widgets::every_chip_measures_the_same_height as fn(),
    ),
    (
        "checkrow::showing_the_files_value_is_not_changing_it",
        checkrow::showing_the_files_value_is_not_changing_it as fn(),
    ),
    (
        "checkrow::a_person_toggling_it_is_reported_once",
        checkrow::a_person_toggling_it_is_reported_once as fn(),
    ),
    (
        "segmented::setting_the_value_moves_the_group_without_reporting_a_change",
        segmented::setting_the_value_moves_the_group_without_reporting_a_change as fn(),
    ),
    (
        "segmented::pressing_a_segment_reports_it_exactly_once",
        segmented::pressing_a_segment_reports_it_exactly_once as fn(),
    ),
    (
        "segmented::pressing_the_active_segment_changes_nothing",
        segmented::pressing_the_active_segment_changes_nothing as fn(),
    ),
    (
        "toast::the_undo_toast_coalesces_and_offers_undo_only_when_there_is_something_to_undo",
        toast::the_undo_toast_coalesces_and_offers_undo_only_when_there_is_something_to_undo as fn(),
    ),
    (
        "toast_tone_and_undo::a_toast_says_its_tone_and_whether_it_offers_undo",
        toast_tone_and_undo::a_toast_says_its_tone_and_whether_it_offers_undo as fn(),
    ),
    (
        "first_frame::work_deferred_to_the_first_frame_runs_even_if_the_window_is_up",
        first_frame::work_deferred_to_the_first_frame_runs_even_if_the_window_is_up as fn(),
    ),
    (
        "components::every_settings_heading_keeps_one_rhythm",
        components::every_settings_heading_keeps_one_rhythm as fn(),
    ),
];

/// Destroy every window a case left open, and let the teardown run, so the
/// next case starts on an empty display.
fn close_all_windows() {
    use gtk::prelude::*;
    if !gtk::is_initialized() {
        return;
    }
    let toplevels = gtk::Window::toplevels();
    let windows: Vec<gtk::Window> = (0..toplevels.n_items())
        .filter_map(|item| toplevels.item(item))
        .filter_map(|object| object.downcast::<gtk::Window>().ok())
        .collect();
    for window in windows {
        window.destroy();
    }
    while gtk::glib::MainContext::default().iteration(false) {}
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
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

    // One initialisation for the process, before any case: each case's own
    // guard then becomes a harmless re-init. No display is not an error
    // here; a case that needs one says so itself.
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
        close_all_windows();
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
