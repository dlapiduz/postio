//! Opening Focus's composer, against the 16ms interaction budget.
//!
//! `c` opens the composer in its dialog over the list (spec 007 US3). The
//! composer and its frame are built once, the first time, and each open
//! after that shows what already exists; this is where that claim becomes a
//! number. `focus_suite::compose_layout` holds the frame's shape; this holds
//! its cost.
//!
//! **Why the number lives here and not in a test (#796).** A wall-clock
//! assertion inside `cargo test` failed whenever another worktree was
//! compiling: 23.7ms against the 16ms budget on a busy box, nine consecutive
//! passes alone on the same commit. A budget asserted on the landing path
//! measures the machine, so it lives in a bench, which blocks nothing.
//!
//! **What runs this.** `bench.yml` compiles the bench targets nightly and
//! deliberately times nothing, because a shared runner cannot defend 16ms --
//! so the assertion below fires when somebody runs this on a quiet machine,
//! not on every pull request.
//!
//! ```sh
//! cargo bench -p postio-bench --bench composer_open
//! ```

#![allow(missing_docs)]
// `criterion_group!` expands to a `pub fn`, and the workspace lint floor
// reaches bench targets. A bench is not public API.

use std::hint::black_box;
use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use gtk::gdk;
use gtk::prelude::*;
use postio_core::CommandId;
use postio_core::perf_budget::{INTERACTION_BUDGET, check_budget};
use postio_focus::startup::Session;
use postio_focus::window::FocusWindow;
use postio_host::Host;
use postio_storage::BlobStore;
use postio_storage::test_support;

/// Turn the main loop until nothing is left to do.
fn settle() {
    for _ in 0..200 {
        gtk::glib::MainContext::default().iteration(false);
    }
}

/// Focus's window over a throwaway store with one account, its inbox shown
/// and settled, and the store's directory, which has to outlive it.
async fn mounted() -> Option<(FocusWindow, Session, tempfile::TempDir)> {
    if adw::init().is_err() {
        return None;
    }
    gdk::Display::default()?;
    let database = test_support::memory().await;
    {
        let connection = database.connect().await.ok()?;
        test_support::account_with_inbox(&connection).await;
    }
    let blobs_dir = tempfile::tempdir().ok()?;
    let blobs = BlobStore::open(blobs_dir.path().to_path_buf(), &test_support::blob_keys()).ok()?;
    let host = Host::start(database, blobs, |wiring| wiring).ok()?;

    let window = FocusWindow::new(None);
    window.present();
    let session = postio_focus::startup::adopt(&window, host, &postio_config::Config::default());
    settle();
    // The first open builds the composer and its frame; every open after it
    // is what a person pays for `c`.
    window.act(CommandId::Compose);
    settle();
    close(&window);
    Some((window, session, blobs_dir))
}

/// What `Esc` costs: the dialog closes, keeping the draft.
fn close(window: &FocusWindow) {
    if let Some(composer) = window.composer() {
        composer.dispatch(CommandId::Back);
    }
    settle();
}

fn bench_composer_open(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let Some((window, session, _blobs)) = mounted().await else {
            eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
            return;
        };

        c.bench_function("focus composer open", |b| {
            b.iter(|| {
                window.act(CommandId::Compose);
                black_box(window.compose_dialog().is_some());
                close(&window);
            });
        });

        // Criterion reports; this fails. A bench that only reports is a
        // bench nobody notices regressing, which is the same reason
        // `list_scroll` asserts as well as measures.
        let start = Instant::now();
        window.act(CommandId::Compose);
        let measured = start.elapsed();
        assert!(
            window.compose_dialog().is_some(),
            "`c` opened no composer, so there is nothing to time"
        );
        close(&window);
        session.stop();
        if let Err(exceeded) = check_budget(measured, INTERACTION_BUDGET) {
            panic!(
                "opening the composer is over budget: {exceeded:?}. After the \
                 first open it shows a composer and frame that already exist; \
                 if that is still true the cost is somewhere else in the open \
                 path."
            );
        }
    });
}

criterion_group!(benches, bench_composer_open);
criterion_main!(benches);
