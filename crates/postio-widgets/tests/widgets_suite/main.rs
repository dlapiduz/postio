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

mod body_view_highlight;
mod body_view_resets;
mod capture;
mod harness;
mod list_contract;
mod list_model_generic;
mod present_config;
mod present_onboarding;
mod present_reading;
mod quote_folds;
mod reader_verbs;
mod support;
mod widgets_css;

/// Cases held out of a default run, by name -- the table-driven spelling of
/// `#[ignore]`, which means one thing here: this machine may not have what
/// the case needs. A name here still runs when asked for explicitly, and still
/// appears in `--list`, exactly as an ignored libtest case does. Say in a
/// comment beside the name which issue or task takes it back.
const IGNORED: &[&str] = &[]; // nothing held out

const CASES: &[(&str, fn())] = &[
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
        "list_contract::the_list_output_stays_libtest_shaped",
        list_contract::the_list_output_stays_libtest_shaped as fn(),
    ),
    ("harness::an_empty_case", harness::an_empty_case as fn()),
    (
        "harness::an_empty_case_is_listed_and_runs",
        harness::an_empty_case_is_listed_and_runs as fn(),
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
        "present_onboarding::the_credential_dialog_reads_the_account_and_saves_through_the_client",
        present_onboarding::the_credential_dialog_reads_the_account_and_saves_through_the_client
            as fn(),
    ),
    (
        "present_reading::fetched_images_come_back_under_the_documents_spelling",
        present_reading::fetched_images_come_back_under_the_documents_spelling as fn(),
    ),
    (
        "capture::a_window_the_compositor_never_showed_is_an_error",
        capture::a_window_the_compositor_never_showed_is_an_error as fn(),
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
