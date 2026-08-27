//! The protocol crates never see each other — same shape and reason as
//! `postio-jmap/tests/boundary.rs`.

use std::path::PathBuf;

fn manifest() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn io_imap_is_not_a_dependency() {
    let manifest = manifest();
    for crate_name in ["io-imap", "io-sasl", "io-jmap"] {
        assert!(
            !manifest.contains(&format!("\n{crate_name} =")),
            "`{crate_name}` must not be a dependency of postio-gmail (ADR 0018)"
        );
    }
}

#[test]
fn the_backend_seam_is_taken_without_its_protocol_features() {
    let manifest = manifest();
    let line = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("postio-imap ="))
        .expect("postio-gmail depends on postio-imap for the seam");
    assert!(
        line.contains("default-features = false"),
        "postio-gmail wants the MailBackend seam, not io-imap: {line}"
    );
}
