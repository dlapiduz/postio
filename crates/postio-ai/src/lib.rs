//! The client for the model a person runs on this computer (spec 007
//! milestone 2, research R16, ADR 0009).
//!
//! Postio embeds no model and starts no runtime. When the person has one --
//! Ollama, a llama.cpp server, anything serving the OpenAI-compatible chat
//! completions -- and names it in `[focus.model]`, this crate asks it
//! questions, and only on this machine:
//!
//! - **Loopback or a local socket.** A [`Client`] takes a
//!   [`postio_config::ModelEndpoint`], which cannot name another host; the
//!   refusal is a sentence saying why (FR-168). The crate links no TLS, so
//!   it could not speak to another machine's https if it tried.
//! - **Fixed, flat schemas.** Every question constrains its answer with
//!   `response_format: json_schema`, and every answer is checked against
//!   the same [`Schema`] before it is believed.
//! - **Message text is data.** It is fenced in a message of its own, the
//!   instructions say it is third parties' text, and no request carries a
//!   tool (ADR 0009 Q4).
//! - **The egress log.** Every connection is recorded under the `model`
//!   subsystem, by where and outcome (FR-168, ADR 0009 Q6).
//! - **No send path.** Nothing that sends mail is in this crate's graph,
//!   and `scripts/checks/check-crate-boundaries.py` keeps it that way.
//!
//! POSTIO-CONSENT: a connection is made only to the endpoint the person
//! wrote in `[focus.model]`, only on this computer, and only for a feature
//! whose switch in that section is on. With no section, no client exists
//! and nothing connects anywhere (FR-166, SC-016).

mod client;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
mod needs_action;
mod schema;
mod summary;
mod transport;

pub use client::{AiError, Client, Question, RETRY_AFTER};
pub use needs_action::NeedsActionModel;
pub use schema::{Field, Kind, Schema, SchemaError};
pub use summary::{MAX_SOURCES, SOURCE_CHARS, Source, Summariser, resolve};
pub use transport::{LocalTransport, Stream, TIMEOUT, Transport};

#[cfg(test)]
mod manifest {
    //! What the resolved graph cannot say. Cargo unifies features across the
    //! workspace, so io-http's TLS is in the graph `check-crate-boundaries.py`
    //! reads however this crate asks for it; this reads what it asks for.

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
