//! Postio Focus: a dense, keyboard-first inbox. It runs `postio-host` in this process and reaches
//! mail only through
//! `postio-client` (ADR 0041); it draws with `postio-widgets` (ADR 0043). The binary is a thin
//! `main` over [`app::run`] so `tests/focus_suite` drives the same startup.

pub mod a11y;
pub mod app;
pub mod banner;
pub mod bar;
pub mod bulk;
pub mod capture;
pub mod chooser;
pub mod chrome;
pub mod compose;
pub mod contacts;
pub mod controller_picker;
#[cfg(feature = "demo")]
pub mod demo;
pub mod digest;
pub mod drag;
pub mod dropdown;
pub mod empty;
pub mod filtered;
pub mod keymap_dialog;
pub mod keys;
pub mod label_picker;
pub mod list;
pub mod motion;
pub mod move_picker;
pub mod open;
mod open_header;
pub mod places;
pub mod reading_pane;
mod rebuild;
pub mod results;
pub mod results_files;
pub mod row_menu;
pub mod rule_dialog;
pub mod rules;
pub mod save_popover;
pub mod scope_footer;
pub mod search_popover;
pub mod settings;
pub mod source;
mod split;
pub mod startup;
pub mod style;
pub mod thread_bar;
pub mod thread_popovers;
pub mod verbs;
pub mod window;
