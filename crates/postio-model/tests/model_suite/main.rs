//! One binary for the integration tests, so the dependency stack links once (#1128). Nothing here
//! may set a process-global
//! (`set_var`, crate attributes): a case that needs its own process belongs outside.

mod corpus_loader;
mod invariants;
mod malformed_mime;
mod mention;
mod mime;
mod reply;
mod rfc2045;
mod rfc5322;
mod serde_roundtrip;
mod size;
