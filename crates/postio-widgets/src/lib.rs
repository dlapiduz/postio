//! The GTK the desktop app draws with (ADR 0043; `specs/007-postio-focus`,
//! research R1).
//!
//! Postio Focus (`postio-gtk`) depends on this crate, and the shared GTK
//! lives here so it stays free of the store and the protocols. What lives
//! here is what Focus shows: the message view on the
//! reading renderer, the composer, the account form, the keycap, key-hint,
//! chip, action-bar, notice and toast widgets, the list model, and the
//! presenters that join them
//! to `postio-client`. What does not: anything only Focus draws, and
//! anything that opens the store.

pub mod body_view;
pub mod capture;
pub mod composer;
pub mod drag_out;
pub mod editor;
pub mod jank;
pub mod keys;
pub mod list_model;
pub mod onboarding;
pub mod present;
pub mod reader;
pub mod settings;
pub mod startup;
pub mod state;
pub mod storyboard;
pub mod style;
pub mod widgets;
