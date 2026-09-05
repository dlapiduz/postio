//! One binary for `postio-runtime`'s integration tests.
//!
//! Each file in `tests/` gets its own executable from cargo, and each links
//! this workspace's dependency stack whether it uses it or not. That is where
//! the test time goes: 125 integration targets across 18 crates (#1128).
//! `postio-gtk` and `postio-app` were consolidated first; this is the same
//! pass over the leaves.
//!
//! Nothing here needs a display, so unlike `postio-gtk`'s `gtk_suite` this
//! keeps libtest's ordinary harness and its thread pool. The cases already ran
//! in parallel within each old binary and now do so across all of them, which
//! is the same guarantee and one link.
//!
//! **Two cases stayed out, and their own comments say why.**
//! `shutdown.rs` and `logging_privacy.rs` both call
//! `tracing::subscriber::set_global_default`, which succeeds once per
//! process -- "this test binary runs one test and owns the subscriber", as
//! `shutdown.rs` puts it. They set it *globally* rather than per-thread on
//! purpose, because the engine works on a thread of its own and the warning
//! they exist to catch is raised there. Two of those in one binary is one
//! panic, so they keep their own.
//!
//! **A case that needs its own process does not belong here.** None of these
//! set a process-global -- no `set_var`, no crate attributes -- and that was
//! checked rather than assumed. A test that grows one has to move back out, or
//! it will change what its neighbours see.

mod concurrent_accounts;
mod engine;
mod list_refresh;
mod mail_store;
mod network;
mod read_state;
mod search;
mod snooze_wake;
mod sqlite_store;
mod sync_progress;
mod sync_wave;
mod thread_store;

// Shared by `engine`, `sync_wave` and `concurrent_accounts`, which each
// declared it separately when they were separate binaries.
mod harness;
