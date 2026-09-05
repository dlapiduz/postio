//! One binary for `postio-sync`'s integration tests.
//!
//! 16 files became 16 executables, because cargo gives every file
//! in `tests/` its own. Each links SQLCipher, the model and this crate --
//! **50 MB apiece** -- and linking is where this workspace's test
//! time goes: ~3,000 tests execute in 108s inside a `cargo test` step that
//! takes ~497s (#841).
//!
//! Nothing here touches GTK, so unlike `postio-gtk`'s `gtk_suite` this keeps
//! libtest's ordinary harness and its thread pool. The cases were already
//! running in parallel *within* each of the old binaries; they now do so
//! across all of them, which is the same guarantee and one link.
//!
//! **A case that needs its own process does not belong here.** None of these
//! set a process-global -- no `set_var`, no `#![...]` crate attributes -- and
//! that was checked before they were merged rather than assumed. A test that
//! grows one has to move back out, or it will change what its neighbours see.

mod backfill;
mod blob_sink;
mod boundary;
mod concurrent_writers;
mod connect;
mod cross_account_move;
mod discover;
mod drafts;
mod drain;
mod initial;
mod loopback;
mod resync;
mod send;
mod status;
mod watch;
mod write_batch;
