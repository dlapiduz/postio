//! The GTK the desktop app draws with: shared widgets, the composer, the list model and the
//! presenters that join them to `postio-client`. Nothing here opens the store (ADR 0043).

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
