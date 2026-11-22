//! The 16 ms budget lives in a bench, not a test (#796): a wall-clock assertion in `cargo test`
//! measured
//! the machine, failing whenever another worktree was compiling. CI compiles benches and times
//! nothing.

#![allow(missing_docs)]

use std::hint::black_box;
use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use gtk::gdk;
use gtk::prelude::*;
use postio_core::CommandId;
use postio_core::perf_budget::{INTERACTION_BUDGET, check_budget};
use postio_gtk::startup::Session;
use postio_gtk::window::FocusWindow;
use postio_host::Host;
use postio_storage::BlobStore;
use postio_storage::test_support;

fn settle() {
    for _ in 0..200 {
        gtk::glib::MainContext::default().iteration(false);
    }
}

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
    let session = postio_gtk::startup::adopt(&window, host, &postio_config::Config::default());
    settle();
    // The first open builds the composer; every open after it is what a person pays for `c`.
    window.act(CommandId::Compose);
    settle();
    close(&window);
    Some((window, session, blobs_dir))
}

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

        // Criterion reports; this fails, so a regression does not go unnoticed.
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
