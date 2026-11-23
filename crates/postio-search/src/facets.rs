//! Scope and refine: narrowing a search without retyping it. Refinements are measured against the
//! live result set, and [`Facets::suggested`] drops any that match nothing or everything.
//! Scope rescopes the query rather than editing it, so switching back to All mail needs no token
//! deletion; it rides beside the query in `postio_index::executor::SearchRequest`.

use chrono::{Datelike, Months, NaiveDate};
use postio_model::{AddressId, LabelId, MailboxId};

/// Not a mailbox id: a standing rescope that must mean the same thing before any folder is picked.
/// `in:` still names one folder exactly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// Every folder except drafts, junk and trash, which turned one correspondent's name into
    /// sixty-six hits (#1523). Sent stays in; an `in:` naming one of the three reaches it.
    #[default]
    AllMail,
    Inbox,
    /// An approximation: "in a mailbox that is not one of the standard roles" rather than
    /// "carries a `List-Id`". Wrong for a mailbox that files list mail into the inbox; the count
    /// says so.
    Lists,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::AllMail, Scope::Inbox, Scope::Lists];

    pub const fn label(self) -> &'static str {
        match self {
            Scope::AllMail => "All mail",
            Scope::Inbox => "Inbox only",
            Scope::Lists => "Lists",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeCount {
    pub scope: Scope,
    pub hits: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refinement {
    pub token: String,
    pub hits: u64,
}

/// A shortlist, not a filter panel: twenty chips is a thing to read rather than click.
pub const MAX_REFINEMENTS: usize = 4;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facets {
    pub scopes: Vec<ScopeCount>,
    pub refinements: Vec<Refinement>,
}

impl Facets {
    pub fn hits(&self, scope: Scope) -> u64 {
        self.scopes
            .iter()
            .find(|count| count.scope == scope)
            .map(|count| count.hits)
            .unwrap_or(0)
    }

    /// A refinement that keeps none of the set is a dead end; one that keeps all of it does nothing
    /// when clicked. Neither is offered; the rest are largest first, the gentlest narrowing
    /// leading.
    pub fn suggested(&self, total: u64) -> Vec<&Refinement> {
        let mut worth: Vec<&Refinement> = self
            .refinements
            .iter()
            .filter(|refinement| refinement.hits > 0 && refinement.hits < total)
            .collect();
        // Stable, so equal counts keep the order they were measured in —
        // which is the order the canvas draws them in.
        worth.sort_by_key(|refinement| std::cmp::Reverse(refinement.hits));
        worth.truncate(MAX_REFINEMENTS);
        worth
    }
}

/// Idempotent: a double click does not wait for the round trip that would remove the chip, so a
/// token already present comes back unchanged.
pub fn append(query: &str, token: &str) -> String {
    let token = token.trim();
    if token.is_empty() {
        return query.to_owned();
    }
    let query = query.trim();
    if query.split_whitespace().any(|word| word == token) {
        return query.to_owned();
    }
    if query.is_empty() {
        return token.to_owned();
    }
    format!("{query} {token}")
}

/// Every count is of conversations, and of the query with that term added. Names are not here:
/// `postio-session` resolves the ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFacets {
    /// Senders, the top 50 by count.
    pub senders: Vec<Count<AddressId>>,
    /// Recipients (To, Cc), the top 50 by count.
    pub recipients: Vec<Count<AddressId>>,
    pub labels: Vec<Count<LabelId>>,
    pub folders: Vec<Count<MailboxId>>,
    pub attachment: u64,
    pub action: u64,
    pub unread: u64,
    pub months: [MonthCount; 12],
    pub presets: [u64; 5],
    pub capped: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Count<T> {
    pub id: T,
    pub conversations: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MonthCount {
    pub month: NaiveDate,
    pub conversations: u64,
}

pub fn months_ending(today: NaiveDate) -> [NaiveDate; 12] {
    let this = today.with_day(1).unwrap_or(today);
    std::array::from_fn(|index| {
        let back = u32::try_from(11 - index).unwrap_or(0);
        this.checked_sub_months(Months::new(back)).unwrap_or(this)
    })
}

/// Each is the `after:` its words lower to, so a preset's count is the count of the
/// query with that `after:` added: "the last 30 days" is [`crate::natural`]'s rolling
/// `after:`, and a quarter or a year starts on its first day.
pub fn preset_starts(today: NaiveDate) -> [Option<NaiveDate>; 5] {
    let back = |days: u64| today.checked_sub_days(chrono::Days::new(days));
    let quarter = (today.month0() / 3) * 3 + 1;
    [
        back(7),
        back(30),
        NaiveDate::from_ymd_opt(today.year(), quarter, 1),
        NaiveDate::from_ymd_opt(today.year(), 1, 1),
        None,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timeline_is_twelve_months_ending_with_this_one() {
        let day = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let months = months_ending(day(2026, 2, 14));
        assert_eq!(months.len(), 12);
        assert_eq!(months[0], day(2025, 3, 1), "oldest first, across a year");
        assert_eq!(months[9], day(2025, 12, 1));
        assert_eq!(months[10], day(2026, 1, 1));
        assert_eq!(months[11], day(2026, 2, 1), "ending with today's month");
        assert!(months.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            months_ending(day(2026, 12, 31))[0],
            day(2026, 1, 1),
            "December's twelve are one calendar year"
        );
    }

    #[test]
    fn the_date_presets_start_where_their_words_would() {
        let day = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(
            preset_starts(day(2026, 9, 30)),
            [
                Some(day(2026, 9, 23)),
                Some(day(2026, 8, 31)),
                Some(day(2026, 7, 1)),
                Some(day(2026, 1, 1)),
                None,
            ]
        );
        assert_eq!(preset_starts(day(2026, 2, 3))[2], Some(day(2026, 1, 1)));
        assert_eq!(preset_starts(day(2026, 12, 31))[2], Some(day(2026, 10, 1)));
    }

    fn refinement(token: &str, hits: u64) -> Refinement {
        Refinement {
            token: token.to_owned(),
            hits,
        }
    }

    #[test]
    fn every_scope_says_what_it_is() {
        for scope in Scope::ALL {
            assert!(!scope.label().is_empty());
        }
        assert_eq!(Scope::default(), Scope::AllMail);
        assert_eq!(Scope::Lists.label(), "Lists", "the canvas' word, not ours");
    }

    #[test]
    fn a_scope_nobody_measured_has_no_hits_rather_than_no_answer() {
        let facets = Facets {
            scopes: vec![ScopeCount {
                scope: Scope::AllMail,
                hits: 14,
            }],
            refinements: Vec::new(),
        };

        assert_eq!(facets.hits(Scope::AllMail), 14);
        assert_eq!(facets.hits(Scope::Inbox), 0);
    }

    #[test]
    fn a_refinement_that_keeps_nothing_is_a_dead_end() {
        let facets = Facets {
            scopes: Vec::new(),
            refinements: vec![refinement("is:unread", 0), refinement("is:flagged", 3)],
        };

        let offered: Vec<&str> = facets
            .suggested(14)
            .iter()
            .map(|refinement| refinement.token.as_str())
            .collect();
        assert_eq!(offered, ["is:flagged"]);
    }

    #[test]
    fn a_refinement_that_keeps_everything_narrows_nothing() {
        let facets = Facets {
            scopes: Vec::new(),
            refinements: vec![refinement("has:attach", 14), refinement("is:unread", 5)],
        };

        let offered: Vec<&str> = facets
            .suggested(14)
            .iter()
            .map(|refinement| refinement.token.as_str())
            .collect();
        assert_eq!(
            offered,
            ["is:unread"],
            "clicking `has:attach` here would look like the app ignored it"
        );
    }

    #[test]
    fn the_gentlest_useful_narrowing_leads() {
        let facets = Facets {
            scopes: Vec::new(),
            refinements: vec![
                refinement("is:flagged", 1),
                refinement("is:unread", 9),
                refinement("larger:1M", 4),
            ],
        };

        let offered: Vec<&str> = facets
            .suggested(14)
            .iter()
            .map(|refinement| refinement.token.as_str())
            .collect();
        assert_eq!(offered, ["is:unread", "larger:1M", "is:flagged"]);
    }

    #[test]
    fn the_column_is_a_shortlist_not_a_filter_panel() {
        let facets = Facets {
            scopes: Vec::new(),
            refinements: (1..=10)
                .map(|n| refinement(&format!("in:folder{n}"), n))
                .collect(),
        };

        assert_eq!(facets.suggested(100).len(), MAX_REFINEMENTS);
    }

    #[test]
    fn nothing_matched_means_nothing_to_refine() {
        let facets = Facets {
            scopes: Vec::new(),
            refinements: vec![refinement("is:unread", 0)],
        };

        assert!(facets.suggested(0).is_empty());
    }

    #[test]
    fn a_refinement_appends_as_a_token_the_user_could_have_typed() {
        assert_eq!(append("from:lena", "is:unread"), "from:lena is:unread");
    }

    #[test]
    fn appending_to_an_empty_box_is_just_the_token() {
        assert_eq!(append("", "is:unread"), "is:unread");
        assert_eq!(append("   ", "is:unread"), "is:unread");
    }

    #[test]
    fn appending_the_same_token_twice_changes_nothing() {
        let once = append("from:lena", "is:unread");
        assert_eq!(append(&once, "is:unread"), once);
    }

    #[test]
    fn a_token_that_merely_contains_another_is_still_appended() {
        assert_eq!(
            append("in:lists", "in:list"),
            "in:lists in:list",
            "`in:lists` is not `in:list`"
        );
    }

    #[test]
    fn an_empty_token_is_not_a_refinement() {
        assert_eq!(append("from:lena", "  "), "from:lena");
    }
}
