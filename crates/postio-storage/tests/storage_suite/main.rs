//! One binary for `postio-storage`'s integration tests.
//!
//! 24 files became 24 executables, because cargo gives every file
//! in `tests/` its own. Each links SQLCipher, the model and this crate --
//! **54 MB apiece** -- and linking is where this workspace's test
//! time goes: ~3,000 tests execute in 108s inside a `cargo test` step that
//! takes ~497s (#841).
//!
//! Nothing here touches GTK, so unlike a suite that needs a display, this keeps
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
mod body_failures;
mod bulk;
mod cold_jump_cost;
mod concurrent_open;
mod connections;
mod contact_groups;
mod contact_rank_index;
mod contact_settings;
mod contact_stats;
mod contact_suggestions;
mod contacts;
mod contacts_budget;
mod contacts_catch_up;
mod contacts_discovery;
mod contacts_form;
mod contacts_import;
mod contacts_join;
mod contacts_kind;
mod contacts_lifecycle;
mod contacts_list_views;
mod contacts_sender;
mod contacts_view;
mod content_membership;
mod content_migration;
mod correspondents;
mod digests;
mod draft_indexes;
mod drafts;
mod encryption;
mod expiry;
mod filter_decisions;
mod focus_classified;
mod focus_inbox;
mod focus_plans;
mod glib_main_context;
mod labels;
mod list_mail;
mod list_statement_count;
mod mailbox_counts;
mod mailbox_roles;
mod mailbox_size;
mod mailboxes;
mod markers;
mod messages;
mod migrations;
mod operations;
mod promoted_headers;
mod read_receipt_requested;
mod reclaim_pages;
mod refuses_a_foreign_store;
mod refuses_a_stale_schema;
mod refuses_a_store_in_use;
mod reminders;
mod schema_fidelity;
mod searches;
mod seed_is_honest;
mod seed_shapes;
mod sender_names;
mod snoozed_due_index;
mod sort_at;
mod sql_each;
mod statement_cache;
mod store_key;
mod sync_state;
mod sync_statement_bounds;
mod threading;
mod threading_lookup_cost;
mod threading_statement_count;
mod threads;
mod turso_capabilities;
mod unified_threads;
mod upsert_identity_cost;
mod wal_ceiling;
mod walk_columns;
mod write_gate;
