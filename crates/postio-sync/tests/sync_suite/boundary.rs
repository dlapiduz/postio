//! The crate's manifest must not depend on `io-imap` (ADR 0001): the compiler enforces it, a graph
//! check cannot (cargo unifies features), so this guards the manifest.

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
        .find(|line| line.trim_start().starts_with("postio-account ="))
        .expect("postio-sync depends on postio-account");

    assert!(
        line.contains("default-features = false"),
        "postio-sync wants the MailBackend seam and its mock, not io-imap and a TLS stack: {line}"
    );
}

#[test]
fn the_mock_backend_is_reachable_without_the_protocol_feature() {
    let backend = postio_account::backend::MockBackend::new();

    assert_eq!(
        postio_account::backend::MailBackend::describe(&backend),
        "mock"
    );
}
