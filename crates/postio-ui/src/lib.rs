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

pub mod conversation;
pub mod format;
pub mod keymap;
pub mod list;
pub mod reader;
pub mod selection;
pub mod tokens;
