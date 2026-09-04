//! `[[rules]]` — an ordered array of filing rules.
//!
//! ```toml
//! [[rules]]
//! name    = "receipts"
//! query   = "from:billing has:attach"
//! actions = ["move:Receipts", "mark-read"]
//! stop    = true
//! ```
//!
//! An **array of tables**, not a map, because issue #5 requires that rule
//! evaluation order be deterministic and documented, and the only way to get
//! order out of a map is an `order = 3` field on every entry — which users
//! duplicate, skip, and have to renumber to insert a rule in the middle. The
//! file is the order (ADR 0008 Q4).
//!
//! Everything here is **text**, including the query and each action. That is
//! the same line `[filters]` already draws and the reason this crate's
//! dependency list is four crates and none of them domain:
//! `postio_model::rule` is what turns `"move:Receipts"` into an
//! [`Action`](postio_model::rule::Action), and
//! [`Config::rules`](crate::Config::rules) is the seam.

use postio_model::rule::{Rule, RuleError, RuleSource};
use serde::{Deserialize, Serialize};

use crate::{Config, Extras, yes};

/// One entry of `[[rules]]`, as written.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RuleConfig {
    /// What to call it, in Attention and in the settings panel.
    #[serde(default)]
    pub name: String,
    /// An inline query, in the search bar's own language.
    ///
    /// Kept as text: parsing it belongs to `postio-search`, which this crate
    /// does not depend on and must not — that is what `[filters]` already
    /// does with the identical string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// The `[filters]` entry whose query to reuse instead.
    ///
    /// So a query somebody already tuned in the sidebar is not written a
    /// second time here, to drift from the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// What to do, in order.
    #[serde(default)]
    pub actions: Vec<String>,
    /// Whether a match stops the rules below this one.
    ///
    /// Default `false`, deliberately: a `stop`-by-default engine makes "add a
    /// label to everything from this list" silently disable everything under
    /// it (ADR 0008 Q4).
    #[serde(default)]
    pub stop: bool,
    /// Whether the rule runs at all. `false` is how a rule is dry-run.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

impl Config {
    /// Every `[[rules]]` entry, typed, in file order.
    ///
    /// **One result per entry, and nothing dropped.** A rule the user
    /// believes is running and is not is the failure ADR 0008 Q6 is about, so
    /// a broken entry comes back carrying the reason rather than vanishing —
    /// which is also what lets a caller run the rules that are fine and raise
    /// Attention about the one that is not, instead of refusing the file.
    ///
    /// `filter = "…"` is resolved here, against this same config's
    /// `[filters]`, because that is the only place both halves are in scope.
    pub fn rules(&self) -> Vec<Result<Rule, RuleError>> {
        self.rules
            .iter()
            .map(|entry| {
                Rule::parse(
                    &RuleSource {
                        name: entry.name.clone(),
                        query: entry.query.clone(),
                        filter: entry.filter.clone(),
                        actions: entry.actions.clone(),
                        stop: entry.stop,
                        enabled: entry.enabled,
                    },
                    |name| self.filters.get(name).map(|filter| filter.query.as_str()),
                )
            })
            .collect()
    }
}
