//! Toolkit-free presentation logic, shared by every frontend (ADR 0019).
//!
//! `postio-gtk` accumulated ~2,850 lines of logic with no toolkit in it —
//! selection semantics, the reader's document assembly, keymap resolution,
//! design tokens — which a second frontend would otherwise have to
//! reimplement, fork, or link GTK to borrow. This crate is where that logic
//! lives instead: **one implementation, called by both frontends**, which is
//! the structural answer to ADR 0019 Q6's risk that the privacy invariants
//! silently fork.
//!
//! Nothing toolkit-shaped may enter — no GTK, no WebKit, no SQL —
//! and `check-crate-boundaries.py` enforces it, dev-dependencies included.

pub mod account;
pub mod cheatsheet;
pub mod contacts;
pub mod conversation;
pub mod dwell;
pub mod editor;
pub mod finder;
pub mod focus;
pub mod format;
pub mod hints;
pub mod keymap;
pub mod list;
pub mod list_state;
pub mod notify;
pub mod paging;
pub mod palette;
pub mod reader;
pub mod row;
pub mod search;
pub mod selection;
pub mod settings;
pub mod sidebar;
pub mod status;
pub mod test_support;
pub mod tokens;
