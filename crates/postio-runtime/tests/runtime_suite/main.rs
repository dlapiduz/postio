//! One binary for the integration tests, to cut link time (#1128). `shutdown.rs` and
//! `logging_privacy.rs` stay out: each sets
//! the global tracing subscriber. A case reading shared state starts with
//! `postio_test_support::alone!` (#1976).

mod concurrent_accounts;
mod engine;
mod last_synced;
mod list_refresh;
mod local_store;
mod mail_store;
mod network;
mod read_state;
mod search;
mod snooze_wake;
mod sync_progress;
mod sync_wave;
mod thread_store;

mod engine_stack;
mod harness;
