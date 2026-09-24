//! Postio in the terminal (`specs/005-tui-frontend`).
//!
//! A frontend like the desktop app: it opens the store in its own process --
//! one Postio at a time has it, the terminal or the desktop app -- starts a
//! `postio_host::Host` over it, and reads and writes only through a
//! [`postio_client::Client`] of that host. What it adds is a terminal's way
//! of showing and being driven -- and nothing it decides about mail is its
//! own: keys resolve through `postio_ui::keymap`, the list is
//! `postio_ui::list`, the rules of a verb are the host's.
//!
//! The loop is `app::update(&mut App, Input) -> Vec<Effect>`: pure, so every
//! behaviour is testable with synthetic input and no terminal at all, and
//! asserted on the rendered buffer, which is what a person sees.

pub mod app;
pub mod caps;
pub mod clipboard;
pub mod composer;
pub mod config_file;
pub mod conversation;
pub mod external;
pub mod first_run;
pub mod input;
pub mod layout;
pub mod paths;
pub mod reader;
pub mod row;
pub mod run;
pub mod settings;
pub mod sidebar;
pub mod state;
pub mod term;
pub mod theme;
pub mod view;
