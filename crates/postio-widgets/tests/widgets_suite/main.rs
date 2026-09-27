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

mod harness;
mod list_contract;

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
    while glib::MainContext::default().iteration(false) {}
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
