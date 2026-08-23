//! The sync engine never names a protocol crate.
//!
//! # Why this is a manifest test and not a dependency-graph one
//!
//! `scripts/check-crate-boundaries.py` reads `cargo metadata`'s resolved graph,
//! which is the right tool for `postio-core` and `postio-gtk` — neither may
//! reach GTK or SQLite by any route, transitive ones included. It cannot
//! express *this* invariant, because cargo unifies features across a workspace:
//! `postio-imap` is itself a workspace member and builds with its default
//! `imap` feature on, so `io-imap` appears in the resolved graph whatever this
//! crate asks for, and a graph check would fail on a crate that is entirely
//! correct.
//!
//! What actually holds the line is this crate's *manifest*. `io-imap` is not a
//! dependency of `postio-sync`, so `use io_imap::…` does not compile — the
//! compiler is the enforcement. This test guards the manifest itself, which is
//! the one thing a well-meaning edit could quietly change.
//!
//! See ADR 0001 and the feature comment in `crates/postio-imap/Cargo.toml`.

use std::path::PathBuf;

fn manifest() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn no_protocol_crate_is_a_direct_dependency() {
    let manifest = manifest();

    for crate_name in ["io-imap", "io-sasl", "io-smtp"] {
        assert!(
            !manifest.contains(&format!("\n{crate_name} =")),
            "`{crate_name}` must not be a dependency of postio-sync: the engine is written \
             against the MailBackend trait so that a pre-1.0 protocol crate cannot reach it, \
             and so that a second protocol needs no change above the seam (ADR 0001)"
        );
    }
}

#[test]
fn the_backend_seam_is_taken_without_its_protocol_features() {
    let manifest = manifest();
    let line = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("postio-imap ="))
        .expect("postio-sync depends on postio-imap");

    assert!(
        line.contains("default-features = false"),
        "postio-sync wants the MailBackend seam and its mock, not io-imap and a TLS stack: {line}"
    );
}

#[test]
fn the_mock_backend_is_reachable_without_the_protocol_feature() {
    // The point of the arrangement above: the whole engine is testable with no
    // server and no network. If this stops compiling, the seam has moved.
    let backend = postio_imap::backend::MockBackend::new();

    assert_eq!(
        postio_imap::backend::MailBackend::describe(&backend),
        "mock"
    );
}
