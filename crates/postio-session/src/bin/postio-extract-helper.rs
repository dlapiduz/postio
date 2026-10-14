//! Read the text out of one attachment, in a process the indexer can kill
//! (spec 010 D28).
//!
//! One request on stdin, one reply on stdout, in `postio_extract::wire`'s
//! frame; the whole program is `postio_extract::helper::run`. It lives in
//! this crate rather than that one so the session's integration tests,
//! which drive the indexer that spawns it, are handed its path by cargo
//! (`CARGO_BIN_EXE_postio-extract-helper`) and always run the helper built
//! from the same source. It links nothing of this crate: only the pure
//! leaf and the standard library.
//!
//! It ships beside the application's executable: `Contents/MacOS` in
//! `Postio.app` (`scripts/macos-bundle.sh`), `/app/bin` in the flatpak.
//! Nobody runs it by hand; to see what it would read, call
//! `postio_extract::extract`.

fn main() -> std::process::ExitCode {
    postio_extract::helper::run()
}
