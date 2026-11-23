//! The user's digest rules: a name and queries in the one query language; a message is held when
//! any query matches, asked in memory through `postio_search::matcher::Matcher` (ADR 0008). Rules
//! arrive as plain names and queries: `postio-config` watches files, and nothing network-capable
//! may reach here.

use std::sync::LazyLock;

use chrono::NaiveDate;
use postio_model::Message;
use postio_search::matcher::Matcher;

use crate::outcome::RuleName;

#[derive(Debug, Clone, Default)]
pub struct Digests {
    rules: Vec<Digest>,
}

#[derive(Debug, Clone)]
struct Digest {
    name: RuleName,
    queries: Vec<Matcher>,
}

static NONE: LazyLock<Digests> = LazyLock::new(Digests::default);

impl Digests {
    pub fn none() -> &'static Digests {
        &NONE
    }

    /// A rule with a blank name, no query, or a query the matcher cannot read (beyond `from:`,
    /// `to:`, `subject:`, `filename:`, `list:`, or half-typed) is left out and the others still
    /// apply (ADR 0008 Q6): a rule that holds more than it says is the worst a digest can do.
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

    pub fn holding(&self, message: &Message) -> Option<&RuleName> {
        self.rules
            .iter()
            .find(|rule| rule.queries.iter().any(|query| query.matches(message)))
            .map(|rule| &rule.name)
    }
}
