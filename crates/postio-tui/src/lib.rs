//! Postio in the terminal: a frontend that reads and writes only through a `postio_client::Client`
//! of its own host and decides nothing about mail itself.
//! The loop is `app::update(&mut App, Input) -> Vec<Effect>`, pure, so tests drive it with
//! synthetic
//! input and assert on the rendered buffer.

pub mod app;
pub mod ask;
pub mod bar;
pub mod caps;
pub mod capture;
pub mod clipboard;
pub mod composer;
pub mod config_file;
pub mod conversation;
pub mod digest;
pub mod external;
pub mod filtered;
pub mod first_run;
pub mod focus;
pub mod folders;
pub mod input;
pub mod layout;
pub mod paths;
pub mod pickers;
pub mod places;
pub mod reader;
pub mod results;
pub mod row;
pub mod rule_dialog;
pub mod rules;
pub mod run;
pub mod settings;
pub mod sheet;
pub mod surface;
pub mod term;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod theme;
pub mod view;
