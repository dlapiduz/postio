/// mimalloc, for the small allocations the store's page cache and fts index churn.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> gtk::glib::ExitCode {
    postio_gtk::app::run()
}
