//! The `postio` binary, which is Focus (spec 007, C27): everything is in the
//! library, which the suite drives too.

/// The allocator, chosen here because this is the binary: mimalloc, as the
/// shipped `postio` has used since the store engine stopped installing it
/// itself, for the small allocations its page cache and fts index churn.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> gtk::glib::ExitCode {
    postio_gtk::app::run()
}
