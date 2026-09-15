//! The `postio` binary: a thin `main` over the library half.
//!
//! Everything the application actually does lives in `lib.rs`, so that
//! `tests/` can link it. A binary crate cannot be linked by an integration
//! test, which is why the composition root — the one layer that joins the
//! store, the runtime and the view — had no integration coverage at all, and
//! why eight capabilities were found implemented, tested and never called.
//! See `postio-bl2`.

/// The allocator, chosen here because this is the binary.
///
/// The store engine's crate used to install this itself as a default
/// feature, which put a global allocator inside a library's feature set;
/// that feature is off now, and the choice is made where it belongs. It
/// stays mimalloc rather than reverting to the system allocator because the
/// engine's page cache and the fts index churn small allocations on the hot
/// path, and the engine chose mimalloc for exactly that.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> gtk::glib::ExitCode {
    postio_app::run()
}
