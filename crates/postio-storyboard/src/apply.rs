//! Which apps a storyboard applies to.
//!
//! Applicability is static (research R7): the loader and the lint can answer
//! it with no runner built, because it is a question about the command
//! registry and about what each runner declares, not about a run.

use serde::{Deserialize, Serialize};

/// A Postio frontend a storyboard can be played against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum App {
    /// The GTK desktop client.
    Classic,
    /// The GTK client's focus layout (spec 007).
    Focus,
    /// The terminal client.
    Terminal,
    /// The macOS client.
    Macos,
}
