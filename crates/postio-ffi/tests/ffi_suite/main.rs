//! One binary for `postio-ffi`'s integration tests.
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
//! **A case that needs its own process does not belong here.** None of these
//! set a process-global -- no `set_var`, no crate attributes -- and that was
//! checked rather than assumed. A test that grows one has to move back out, or
//! it will change what its neighbours see.

mod aiming;
mod config;
mod keys;
mod list;
mod reader;
mod registry;
mod selection;
mod session;
mod store_on_disk;
mod syncing;
