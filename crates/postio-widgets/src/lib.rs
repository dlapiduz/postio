//! The GTK both desktop apps draw with (ADR 0043; `specs/007-postio-focus`,
//! research R1).
//!
//! The classic app (`postio-gtk`, `postio-app`) and Postio Focus
//! (`postio-focus`) each depend on this crate, and neither depends on the
//! other. What lives here is what both of them show: the message view on the
//! reading renderer, the composer, the keycap, key-hint, chip, action-bar,
//! notice and toast widgets, the list model, and the presenters that join them
//! to `postio-client`. What does not: anything only one app draws, and
//! anything that opens the store.

pub mod body_view;
pub mod widgets;
