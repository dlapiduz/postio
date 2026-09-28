//! Postio Focus's binary: everything is in the library, which the suite
//! drives too.

fn main() -> gtk::glib::ExitCode {
    postio_focus::app::run()
}
