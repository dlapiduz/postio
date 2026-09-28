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

mod chrome;
mod colours;
mod harness;
mod list_contract;
mod marked_rows;
mod rows;
mod selection;
mod starts_offline;
mod support;
mod undo;
mod visible_window;

/// Cases held out of a default run, by name -- the table-driven spelling of
/// `#[ignore]`, which means one thing here: this machine may not have what
/// the case needs. A name here still runs when asked for explicitly, and still
/// appears in `--list`, exactly as an ignored libtest case does. Say in a
/// comment beside the name which issue or task takes it back.
const IGNORED: &[&str] = &[]; // nothing held out

const CASES: &[(&str, fn())] = &[
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
        "colours::the_roles_resolve_and_follow_the_system_into_dark",
        colours::the_roles_resolve_and_follow_the_system_into_dark as fn(),
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
