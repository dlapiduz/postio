//! Opening the composer, against the 16ms interaction budget.
//!
//! `c` takes the reading pane. Structurally that is a `set_visible` over
//! widgets that already exist — `postio-gtk`'s `gtk_composer.rs` is what holds
//! it to that, asserting the open is synchronous and rebuilds nothing — and
//! this is where the claim becomes a number.
//!
//! **Why the number lives here and not in that test (#796).** It used to be a
//! wall-clock assertion inside `cargo test`, and it failed whenever another
//! worktree was compiling: 23.7ms against the 16ms budget on a busy box, nine
//! consecutive passes alone on the same commit. A budget asserted on the
//! landing path measures the machine, and this repository's normal state is
//! several sessions building at once — so a spurious failure arrived at the
//! last gate before a merge and read as a regression in the change being
//! landed. A bench blocks nothing, so a slow reading here costs a look rather
//! than a diagnosis.
//!
//! **What runs this.** `bench.yml` compiles the bench targets nightly and
//! deliberately times nothing, because a shared runner cannot defend 16ms —
//! so the assertion below fires when somebody runs this on a quiet machine,
//! not on every pull request. That is the whole arrangement `#100` settled:
//! what gates a PR is the *cause* of a budget, counted or asserted
//! structurally, and `gtk_composer.rs` is where this one's cause lives. A
//! number nobody can defend on a shared runner is worth measuring
//! deliberately and worth gating on never.

#![allow(missing_docs)]
// `criterion_group!` expands to a `pub fn`, and the workspace lint floor
// reaches bench targets. A bench is not public API.

use std::hint::black_box;
use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use gtk::gdk;
use postio_core::perf_budget::{INTERACTION_BUDGET, check_budget};
use postio_gtk::composer::Composer;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::{AccountId, Draft};

/// A window with a composer installed, mounted and settled.
fn mounted() -> Option<(Window, Composer)> {
    if adw::init().is_err() {
        return None;
    }
    let display = gdk::Display::default()?;
    // Before the first widget, or a `PangoContext` caches the fallback family
    // for the process and this would be timing the wrong typeface.
    fonts::install().ok()?;
    style::install(&display);

    let window = Window::default();
    style::track(&window);
    let composer = postio_gtk::composer::install(&window);
    window.present();
    for _ in 0..200 {
        gtk::glib::MainContext::default().iteration(false);
    }
    Some((window, composer))
}

fn a_draft() -> Draft {
    Draft::new(AccountId::new(1))
}

/// Open it and put it back, which is what `c` then `Esc` costs.
fn open_and_close(composer: &Composer) {
    composer.open(a_draft());
    black_box(composer.is_open());
    composer.close();
}

fn bench_composer_open(c: &mut Criterion) {
    let Some((_window, composer)) = mounted() else {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    };

    c.bench_function("composer open", |b| {
        b.iter(|| open_and_close(&composer));
    });

    // Criterion reports; this fails. A bench that only reports is a bench
    // nobody notices regressing, which is the same reason `list_scroll`
    // asserts as well as measures.
    open_and_close(&composer); // warm the styles and the first layout
    let start = Instant::now();
    composer.open(a_draft());
    let measured = start.elapsed();
    composer.close();
    if let Err(exceeded) = check_budget(measured, INTERACTION_BUDGET) {
        panic!(
            "opening the composer is over budget: {exceeded:?}. It is meant to \
             be a `set_visible` over widgets that already exist -- if that is \
             still true, `gtk_composer.rs` will still be passing and the cost \
             is somewhere else in the open path."
        );
    }
}

criterion_group!(benches, bench_composer_open);
criterion_main!(benches);
