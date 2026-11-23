//! Read the text out of one attachment, in a process the indexer can kill: one request on stdin,
//! one reply on stdout.
//! It lives in this crate so integration tests get its path from cargo; it ships beside the
//! executable.

fn main() -> std::process::ExitCode {
    postio_extract::helper::run()
}
