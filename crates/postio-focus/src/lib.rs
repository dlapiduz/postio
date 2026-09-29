//! Postio Focus: a dense, keyboard-first inbox on the Postio engine
//! (`specs/007-postio-focus`).
//!
//! It runs `postio-host` in this process and reaches mail only through
//! `postio-client` (ADR 0041), and it draws with `postio-widgets`, never with
//! the classic app's crates (ADR 0043). The binary is a thin `main` over
//! [`app::run`]; everything else is here, where `tests/focus_suite` can drive
//! the same startup the binary runs.

pub mod app;
pub mod banner;
pub mod bar;
pub mod bulk;
pub mod capture;
pub mod chooser;
pub mod chrome;
pub mod compose;
pub mod digest;
pub mod empty;
pub mod filtered;
pub mod keymap_dialog;
pub mod keys;
pub mod label_picker;
pub mod list;
pub mod move_picker;
pub mod names;
pub mod open;
pub mod places;
pub mod rule_dialog;
pub mod rules;
pub mod source;
pub mod startup;
pub mod style;
pub mod window;
