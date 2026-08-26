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
//! headless runner's watchdog, in isolation — #272) and `gtk_accessibility`
//! (its own display races, #45/#114).
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
mod gtk_composer_autosave;
mod gtk_composer_document;
mod gtk_composer_focus;
mod gtk_display_required;
mod gtk_feeds;
mod gtk_finder;
mod gtk_flagged;
mod gtk_focus_visible;
mod gtk_list_recycling;
mod gtk_list_reload;
mod gtk_move_picker;
mod gtk_parts;
mod gtk_reading_pane;
mod gtk_row;
mod gtk_saved_searches_live;
mod gtk_search_live;
mod gtk_search_panel;
mod gtk_search_preview;
mod gtk_selection;
mod gtk_settings;
mod gtk_sidebar;
mod gtk_sidebar_keys;
mod gtk_sidebar_saved_searches;
mod gtk_sidebar_tree;
mod gtk_style;
mod gtk_thread;
mod gtk_thread_scope;
mod gtk_window;
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
        "gtk_finder::one_box_searches_mail_runs_commands_and_jumps_to_folders",
        gtk_finder::one_box_searches_mail_runs_commands_and_jumps_to_folders as fn(),
    ),
    (
        "gtk_finder::at_finds_a_correspondent_and_searches_their_mail",
        gtk_finder::at_finds_a_correspondent_and_searches_their_mail as fn(),
    ),
    (
        "gtk_focus_visible::taking_focus_changes_what_is_drawn",
        gtk_focus_visible::taking_focus_changes_what_is_drawn as fn(),
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
        "gtk_move_picker::m_opens_the_folder_picker_and_the_folder_picked_becomes_the_move",
        gtk_move_picker::m_opens_the_folder_picker_and_the_folder_picked_becomes_the_move as fn(),
    ),
    (
        "gtk_parts::the_parts_panel_walks_a_message_without_fetching_any_of_it",
        gtk_parts::the_parts_panel_walks_a_message_without_fetching_any_of_it as fn(),
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
        "gtk_search_live::the_readout_answers_the_query_on_screen_and_no_other",
        gtk_search_live::the_readout_answers_the_query_on_screen_and_no_other as fn(),
    ),
    (
        "gtk_search_panel::the_scope_column_narrows_a_search_without_retyping_it",
        gtk_search_panel::the_scope_column_narrows_a_search_without_retyping_it as fn(),
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
        "gtk_sidebar::the_sidebar_lists_folders_and_says_where_sync_stands",
        gtk_sidebar::the_sidebar_lists_folders_and_says_where_sync_stands as fn(),
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
        "gtk_sidebar_tree::folders_nest_collapse_and_a_noselect_parent_only_toggles",
        gtk_sidebar_tree::folders_nest_collapse_and_a_noselect_parent_only_toggles as fn(),
    ),
    (
        "gtk_style::the_generated_stylesheet_works_in_gtk",
        gtk_style::the_generated_stylesheet_works_in_gtk as fn(),
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
