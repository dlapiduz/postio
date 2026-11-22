//! Presenters both desktop apps would otherwise write twice (ADR 0043): the joints between a widget
//! and what feeds it, reaching no window, store or network of their own.

pub mod compose;
pub mod config;
pub mod export;
pub mod onboarding;
pub mod reading;
pub mod settings;
