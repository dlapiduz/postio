//! The `postio` binary: a thin `main` over the library half.
//!
//! Everything the application actually does lives in `lib.rs`, so that
//! `tests/` can link it. A binary crate cannot be linked by an integration
//! test, which is why the composition root — the one layer that joins the
//! store, the runtime and the view — had no integration coverage at all, and
//! why eight capabilities were found implemented, tested and never called.
//! See `postio-bl2`.

fn main() -> gtk::glib::ExitCode {
    postio_app::run()
}
