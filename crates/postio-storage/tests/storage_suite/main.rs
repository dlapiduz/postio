//! One binary for `postio-storage`'s integration tests.
//!
//! 24 files became 24 executables, because cargo gives every file
//! in `tests/` its own. Each links SQLCipher, the model and this crate --
//! **54 MB apiece** -- and linking is where this workspace's test
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

mod accounts;
mod blob;
mod blob_encryption;
mod body;
mod bulk;
mod cold_jump_cost;
mod connection;
mod connection_priority;
mod contact_groups;
mod contacts;
mod drafts;
mod encrypt_migration;
mod encryption;
mod labels;
mod list_statement_count;
mod mailbox_counts;
mod mailbox_size;
mod mailboxes;
mod messages;
mod migrations;
mod operations;
mod schema_fidelity;
mod seed_is_honest;
mod store_key;
mod sync_state;
mod threading;
mod threads;
mod unified_threads;
mod write_gate;
