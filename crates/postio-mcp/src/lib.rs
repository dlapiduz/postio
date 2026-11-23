//! Postio over the Model Context Protocol (ADR 0010): JSON-RPC on stdin and
//! stdout, no socket. Every read and write goes through `postio-client`.
//! Mail is untrusted: bodies come back as text in a content block labelled as data,
//! and tool names and schemas are literals nothing from mail can reach (ADR 0010 Q5).

pub mod audit;
pub mod backend;
pub mod scope;
pub mod server;
pub mod socket;
pub mod tools;

pub use audit::Audit;
pub use backend::{Backend, Held, Installed, Lease};
pub use scope::Scope;
pub use server::{Server, serve};
