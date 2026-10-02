//! The `postio` binary, which is Focus (spec 007, C27): everything is in the
//! library, which the suite drives too.

fn main() -> gtk::glib::ExitCode {
    postio_focus::app::run()
}
