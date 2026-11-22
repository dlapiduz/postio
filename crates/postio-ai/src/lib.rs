//! The client for the model a person runs on this computer: loopback or a local socket only, flat
//! schemas checked on the way back, message text fenced as data, every connection logged.
//!
//! POSTIO-CONSENT: a connection is made only to the endpoint the person
//! wrote in `[focus.model]`, only on this computer, and only for a feature
//! whose switch in that section is on. With no section, no client exists
//! and nothing connects anywhere.

mod client;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
mod like_this;
mod needs_action;
mod schema;
mod summary;
mod transport;

pub use client::{AiError, Client, Question, RETRY_AFTER};
pub use like_this::{Candidate, Example, LikeThis, MAX_CANDIDATES};
pub use needs_action::NeedsActionModel;
pub use schema::{Field, Kind, Schema, SchemaError};
pub use summary::{MAX_SOURCES, SOURCE_CHARS, Source, Summariser, resolve};
pub use transport::{LocalTransport, Stream, TIMEOUT, Transport};

#[cfg(test)]
mod manifest {
    //! Cargo unifies features workspace-wide, so the resolved graph shows io-http's TLS however
    //! this
    //! crate asks for it; this reads what it asks for.

    fn manifest() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    #[test]
    fn io_http_is_taken_for_its_framing_and_nothing_that_reaches_another_machine() {
        let manifest = manifest();
        let line = manifest
            .lines()
            .find(|line| line.trim_start().starts_with("io-http ="))
            .expect("postio-ai frames HTTP with io-http");
        assert!(line.contains("default-features = false"), "{line}");
        assert!(line.contains(r#"features = ["client"]"#), "{line}");
        for other in [
            "pimalaya-stream",
            "rustls",
            "native-tls",
            "reqwest",
            "hyper",
            "ureq",
        ] {
            assert!(
                !manifest.contains(&format!("\n{other} =")),
                "postio-ai must not depend on `{other}`"
            );
        }
    }
}
