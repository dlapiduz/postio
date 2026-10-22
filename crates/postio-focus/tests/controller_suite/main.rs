//! One binary for `postio-focus`'s integration tests.
//!
//! Each file in `tests/` gets its own executable from cargo, and each links
//! this workspace's dependency stack whether it uses it or not, so cases
//! live here as modules and share one link.
//!
//! **A case that needs its own process does not belong here**: one that sets
//! a process-global (`set_var`, a global subscriber or allocator), runs
//! itself again as a child by test name, or is a `POSTIO-MEASUREMENT`. Give
//! it a file of its own in `tests/` and say why at its top.

mod bar;
mod capture;
mod compose;
mod contact_edit;
mod contact_merge;
mod contact_popover;
mod contacts;
mod contacts_settings;
mod cursor;
mod digest;
mod feed;
mod filtered;
mod history;
mod keymap;
mod keys;
mod pickers;
mod results;
mod states;
mod surfaces;
mod verbs;
mod search_support;
