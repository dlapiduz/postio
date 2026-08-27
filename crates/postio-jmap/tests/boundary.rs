//! The two protocol crates never see each other.
//!
//! Same shape and same reason as `postio-sync/tests/boundary.rs`: workspace
//! feature unification puts `io-imap` in the resolved graph whatever this
//! crate's manifest asks for, so the graph check cannot express this rule —
//! the manifest is what holds the line, and this test guards the manifest.

use std::path::PathBuf;

fn manifest() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn io_imap_is_not_a_dependency() {
    let manifest = manifest();
    for crate_name in ["io-imap", "io-sasl"] {
        assert!(
            !manifest.contains(&format!("\n{crate_name} =")),
            "`{crate_name}` must not be a dependency of postio-jmap: a JMAP adapter that \
             reaches IMAP types has put a protocol behind the seam ADR 0018 keeps neutral"
        );
    }
}

#[test]
fn the_backend_seam_is_taken_without_its_protocol_features() {
    let manifest = manifest();
    let line = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("postio-imap ="))
        .expect("postio-jmap depends on postio-imap for the seam");
    assert!(
        line.contains("default-features = false"),
        "postio-jmap wants the MailBackend seam, not io-imap and a TLS stack: {line}"
    );
}
