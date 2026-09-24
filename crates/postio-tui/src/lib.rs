//! Postio in the terminal (`specs/005-tui-frontend`).
//!
//! A frontend like the desktop app: it holds a [`postio_client::Client`] and
//! never opens the store, which belongs to `postio-daemon` (ADR 0041). What
//! it adds is a terminal's way of showing and being driven -- and nothing it
//! decides about mail is its own: keys resolve through `postio_ui::keymap`,
//! the list is `postio_ui::list`, the rules of a verb are the host's.
//!
//! The loop is `app::update(&mut App, Input) -> Vec<Effect>`: pure, so every
//! behaviour is testable with synthetic input and no terminal at all, and
//! asserted on the rendered buffer, which is what a person sees.

pub mod app;
pub mod caps;
pub mod input;
pub mod layout;
pub mod term;
