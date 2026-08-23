//! Acceptance: `postio-model` must depend on no storage or protocol crate.
//!
//! CI greps for forbidden imports across crate boundaries, but the invariant is
//! load-bearing enough that the crate polices its own manifest too.

/// Crates that would drag storage or protocol concerns into the domain model.
const FORBIDDEN: &[&str] = &[
    "rusqlite",
    "libsqlite3-sys",
    "sqlx",
    "io-imap",
    "io_imap",
    "io-smtp",
    "io_smtp",
    "imap",
    "smtp",
    "lettre",
    "postio-storage",
    "postio-search",
    "postio-imap",
    "postio-smtp",
    "postio-sync",
    "postio-core",
    "postio-gtk",
    "postio-config",
    "gtk4",
    "libadwaita",
    "tokio",
];

#[test]
fn manifest_declares_no_storage_or_protocol_dependency() {
    let manifest = include_str!("../Cargo.toml");
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((key, _)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().trim_matches('"');
        for forbidden in FORBIDDEN {
            assert_ne!(
                key, *forbidden,
                "postio-model must not depend on `{forbidden}` (see CLAUDE.md architectural invariants)"
            );
        }
    }
}

#[test]
fn sources_contain_no_storage_or_protocol_imports() {
    // A cheap stand-in for the CI grep: no `use` of a forbidden crate anywhere.
    let sources: &[&str] = &[
        include_str!("../src/lib.rs"),
        include_str!("../src/ids.rs"),
        include_str!("../src/address.rs"),
        include_str!("../src/account.rs"),
        include_str!("../src/mailbox.rs"),
        include_str!("../src/flag.rs"),
        include_str!("../src/label.rs"),
        include_str!("../src/headers.rs"),
        include_str!("../src/attachment.rs"),
        include_str!("../src/message.rs"),
        include_str!("../src/mime.rs"),
        include_str!("../src/outgoing.rs"),
        include_str!("../src/subject.rs"),
        include_str!("../src/thread.rs"),
        include_str!("../src/contact.rs"),
        include_str!("../src/draft.rs"),
    ];
    for src in sources {
        for line in src.lines() {
            let line = line.trim();
            if !line.starts_with("use ") {
                continue;
            }
            for forbidden in FORBIDDEN {
                let root = forbidden.replace('-', "_");
                assert!(
                    !line.starts_with(&format!("use {root}::")) && line != format!("use {root};"),
                    "forbidden import in postio-model: {line}"
                );
            }
        }
    }
}
