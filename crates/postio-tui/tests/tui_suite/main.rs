//! One binary for `postio-tui`'s integration tests.
//!
//! Each file in `tests/` gets its own executable from cargo, and each links
//! this workspace's dependency stack whether it uses it or not, so cases
//! live here as modules and share one link.
//!
//! **A case that needs its own process does not belong here**: one that sets
//! a process-global (`set_var`, a global subscriber or allocator), runs
//! itself again as a child by test name, or is a `POSTIO-MEASUREMENT`. Give
//! it a file of its own in `tests/` and say why at its top.

mod budgets;
mod focus_engine;
mod gmail_keys;
mod packaging;
mod registry_parity;
mod store_in_use;
mod undo_key;
