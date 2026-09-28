//! Postio Focus: a dense, keyboard-first inbox on the Postio engine
//! (`specs/007-postio-focus`).
//!
//! It runs `postio-host` in this process and reaches mail only through
//! `postio-client` (ADR 0041), and it draws with `postio-widgets`, never with
//! the classic app's crates (ADR 0043). The binary is a thin `main` over
//! [`app::run`]; everything else is here, where `tests/focus_suite` can drive
//! the same startup the binary runs.

pub mod app;
pub mod chrome;
pub mod keys;
pub mod list;
pub mod startup;
pub mod window;
