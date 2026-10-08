//! One binary for `postio-ffi`'s integration tests.
//!
//! Each file in `tests/` gets its own executable from cargo, and each links
//! this workspace's dependency stack whether it uses it or not. That is where
//! the test time goes: 125 integration targets across 18 crates (#1128).
//! The app suites were consolidated first; this is the same
//! pass over the leaves.
//!
//! Nothing here needs a display, so unlike a GTK suite this
//! keeps libtest's ordinary harness and its thread pool. The cases already ran
//! in parallel within each old binary and now do so across all of them, which
//! is the same guarantee and one link.
//!
//! **A case that needs its own process does not belong here.** None of these
//! set a process-global -- no `set_var`, no crate attributes -- and that was
//! checked rather than assumed. A test that grows one has to move back out, or
//! it will change what its neighbours see.

mod account_repair;
mod account_switches;
mod aiming;
mod command_coverage;
mod compose;
mod config;
mod conversation;
mod dwell;
mod facts;
mod finder;
mod first_run;
mod focus;
mod focus_bar;
mod focus_keymap;
mod focus_pickers;
mod focus_scroll;
mod focus_states;
mod host;
mod keys;
mod notice;
mod palette;
mod parts;
mod provisioning;
mod reader;
mod reader_cost;
mod recipients;
mod registry;
mod saved_search;
mod session;
mod settings;
mod store_on_disk;
mod syncing;
mod thread_document;
mod unsubscribe;
