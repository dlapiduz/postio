//! `postio-mcp`, driven as an agent drives it: JSON-RPC in, JSON-RPC out,
//! over a real host and a store with mail in it (ADR 0010).

mod audit;
mod drafts;
mod harness;
mod injection;
mod opt_in;
mod protocol;
mod reading;
mod stdio;
mod tools;
