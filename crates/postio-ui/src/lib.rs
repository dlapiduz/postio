//! Toolkit-free presentation logic, shared by every frontend (ADR 0019).
//!
//! The classic app accumulated ~2,850 lines of logic with no toolkit in it —
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
pub mod allowlist;
pub mod capture;
pub mod cheatsheet;
pub mod clock;
pub mod command_bar;
pub mod compose;
pub mod conversation;
pub mod digest;
pub mod dwell;
pub mod editor;
pub mod filtered;
pub mod filtering;
pub mod find;
pub mod finder;
pub mod focus;
pub mod focus_dialog;
pub mod focus_list;
pub mod focus_row;
pub mod focus_state;
pub mod focus_target;
pub mod format;
pub mod handoff;
pub mod hints;
pub mod keymap;
pub mod keymap_sheet;
pub mod keyring_refusal;
pub mod label_colour;
pub mod links;
pub mod list;
pub mod list_state;
pub mod names;
pub mod notify;
pub mod observe;
pub mod onboarding;
pub mod paging;
pub mod palette;
pub mod paste;
pub mod pickers;
pub mod places;
pub mod privacy;
pub mod reader;
pub mod recipients;
pub mod row;
pub mod saved_search;
pub mod schedule;
pub mod search;
pub mod selection;
pub mod sending;
pub mod settings;
pub mod sidebar;
pub mod status;
pub mod surfaced;
pub mod terminal;
pub mod test_support;
pub mod tokens;
pub mod unsubscribe;
