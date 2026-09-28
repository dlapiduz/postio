//! The user's digest rules, as the filing stage asks them (spec 007 T133,
//! research R13, FR-120 to FR-127).
//!
//! A rule is a name and a list of queries in the one query language, and it
//! holds a message when any of its queries matches (`[[focus.digests]]`,
//! contracts/config.md). Search answers those queries with the index; filing
//! asks them of one message in memory, before the message is anywhere a
//! query could see it, through `postio_search::matcher::Matcher` -- which
//! ADR 0008's differential test holds equal to the executor, so a rule
//! holds the mail its preview showed.
//!
//! The rules arrive as plain names and queries rather than as the config
//! crate's type: `postio-config` watches files, which puts a network-capable
//! crate in its graph, and nothing of that kind may reach the classifier
//! (its boundary rule). The filing pass reads `[focus]` and hands over what
//! is here.

use std::sync::LazyLock;

use chrono::NaiveDate;
use postio_model::Message;
use postio_search::matcher::Matcher;

use crate::outcome::RuleName;

/// The digest rules that apply, in the file's order.
#[derive(Debug, Clone, Default)]
pub struct Digests {
    rules: Vec<Digest>,
}

/// One rule: its name, and the queries any one of which holds a message.
#[derive(Debug, Clone)]
struct Digest {
    name: RuleName,
    queries: Vec<Matcher>,
}

static NONE: LazyLock<Digests> = LazyLock::new(Digests::default);

impl Digests {
    /// No rules: nothing is held.
    pub fn none() -> &'static Digests {
        &NONE
    }

    /// `rules` -- each a name and its queries, in the file's order -- read
    /// as of `today`, which dates a relative query.
    ///
    /// A rule with a blank name, no query, or a query the matcher cannot
    /// read (anything beyond `from:`, `to:`, `subject:`, `filename:` and
    /// `list:`, or one still half-typed) is left out, and the others still apply (ADR 0008 Q6): a rule that
    /// holds more than it says is the worst thing a digest can do.
    /// Validation is what tells the user (`postio_ui::digest`).
    pub fn new<'a>(
        rules: impl IntoIterator<Item = (&'a str, &'a [String])>,
        today: NaiveDate,
    ) -> Self {
        let rules = rules
            .into_iter()
            .filter_map(|(name, queries)| {
                let name = name.trim();
                if name.is_empty() || queries.is_empty() {
                    return None;
                }
                let queries = queries
                    .iter()
                    .map(|query| Matcher::new(&postio_search::parse(query, today)).ok())
                    .collect::<Option<Vec<_>>>()?;
                Some(Digest {
                    name: RuleName(name.to_owned()),
                    queries,
                })
            })
            .collect();
        Digests { rules }
    }

    /// The first rule that holds `message`, if any does.
    pub fn holding(&self, message: &Message) -> Option<&RuleName> {
        self.rules
            .iter()
            .find(|rule| rule.queries.iter().any(|query| query.matches(message)))
            .map(|rule| &rule.name)
    }
}
