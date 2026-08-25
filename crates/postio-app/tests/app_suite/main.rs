//! One binary for postio-app's single-test suites — the same custom harness
//! as postio-gtk's `gtk_suite`, for the same two reasons: GTK initializes on
//! exactly one thread per process (#41) while libtest runs tests on a pool,
//! and every test binary here links the entire application. See
//! crates/postio-gtk/tests/gtk_suite/main.rs for the full rationale (#329).
//!
//! The `e2e*` binaries stay out on purpose: they run under the headless
//! runner's watchdog by *name* (#272), in isolation.

mod bulk_keystroke;
mod compose_detach;
mod cursor_preview;
mod drag_out_portal;
mod drag_out_wiring;
mod dwell_wiring;
mod keystroke;
mod parts_open_wiring;
mod reading;
mod reading_offline;
mod resume_draft;
mod search_index;
mod search_results;
mod search_wiring;
mod startup_repair;
mod window_drain;
mod wiring;

const CASES: &[(&str, fn())] = &[
    (
        "bulk_keystroke::ctrl_a_then_shift_u_marks_the_whole_folder_read",
        bulk_keystroke::ctrl_a_then_shift_u_marks_the_whole_folder_read as fn(),
    ),
    (
        "compose_detach::the_detach_key_reaches_the_composer_in_a_wired_application",
        compose_detach::the_detach_key_reaches_the_composer_in_a_wired_application as fn(),
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
        "reading::opening_a_message_fills_the_pane_and_its_chips_open_the_parts_tree",
        reading::opening_a_message_fills_the_pane_and_its_chips_open_the_parts_tree as fn(),
    ),
    (
        "reading_offline::the_pane_says_offline_and_updates_the_moment_the_connection_does",
        reading_offline::the_pane_says_offline_and_updates_the_moment_the_connection_does as fn(),
    ),
    (
        "resume_draft::return_on_a_draft_row_opens_the_composer_on_that_draft",
        resume_draft::return_on_a_draft_row_opens_the_composer_on_that_draft as fn(),
    ),
    (
        "search_index::a_store_the_application_opened_can_be_searched",
        search_index::a_store_the_application_opened_can_be_searched as fn(),
    ),
    (
        "search_results::a_query_puts_the_matching_messages_in_the_list",
        search_results::a_query_puts_the_matching_messages_in_the_list as fn(),
    ),
    (
        "search_wiring::typing_in_the_box_searches_the_store_and_fills_every_search_surface",
        search_wiring::typing_in_the_box_searches_the_store_and_fills_every_search_surface
            as fn(),
    ),
    (
        "startup_repair::an_account_with_no_credential_lands_on_the_repair_screen",
        startup_repair::an_account_with_no_credential_lands_on_the_repair_screen as fn(),
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
