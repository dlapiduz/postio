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

#![allow(clippy::disallowed_methods)] // the crate's own code prepares through `sql::statement`; a test may reach the engine directly

mod accounts;
mod actions;
mod blob;
mod blob_encryption;
mod bulk;
mod cold_jump_cost;
mod concurrent_open;
mod connections;
mod contact_groups;
mod contact_rank_index;
mod contacts;
mod contacts_budget;
mod contacts_join;
mod contacts_lifecycle;
mod contacts_list_views;
mod draft_indexes;
mod drafts;
mod encryption;
mod labels;
mod list_statement_count;
mod mailbox_counts;
mod mailbox_roles;
mod mailbox_size;
mod mailboxes;
mod messages;
mod operations;
mod reclaim_pages;
mod schema_fidelity;
mod seed_is_honest;
mod sender_names;
mod snoozed_due_index;
mod statement_cache;
mod store_key;
mod sync_state;
mod threading;
mod threading_lookup_cost;
mod threading_statement_count;
mod threads;
mod unified_threads;
mod wal_ceiling;
mod write_gate;
