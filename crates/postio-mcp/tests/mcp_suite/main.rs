//! `postio-mcp`, driven as an agent drives it: JSON-RPC in, JSON-RPC out,
//! over a real host and a store with mail in it (ADR 0010).

mod app_socket;
mod audit;
mod drafts;
mod harness;
mod injection;
mod one_store_at_a_time;
mod opt_in;
mod organize;
mod protocol;
mod reading;
mod stdio;
mod tools;
