//! Scope and refine: the two ways canvas 2b narrows a search without
//! retyping it.
//!
//! The left column of artboard 2b is a **Scope** list (All mail / Inbox only
//! / Lists, each with a count) and a **Refine** row of chips (`is:unread`,
//! `larger:1M`, `is:flagged`, …). They are the same idea twice: take the
//! query the user already typed, ask what *else* is true of the messages it
//! matched, and offer that as one click.
//!
//! # Why the counts come first
//!
//! A facet without a count is a guess, and half of them are dead ends — the
//! rule in `/ux-architect` is that no surface may offer a step that leads
//! nowhere. So [`Facets`] is *measured* against the live result set (see
//! `postio_index::executor::facets`), and [`Facets::suggested`] then drops every
//! refinement that would match nothing (there is nothing behind it) or
//! everything (it would narrow nothing). What survives is guaranteed to
//! change the result set and guaranteed not to empty it.
//!
//! # Why the scope is not a chip
//!
//! Scope rescopes the query; it does not edit it. Typing `in:inbox` into the
//! box would do the same thing, but then switching back to All mail means
//! finding and deleting a token — and the acceptance criterion is that scope
//! switching happens *without retyping*. So the scope rides alongside the
//! query in `postio_index::executor::SearchRequest`, and the box keeps saying
//! what the user typed.
//!
//! Refinements are the opposite, and deliberately so: clicking one *appends a
//! chip*, because it is a token the user could have typed.

use chrono::{Datelike, Months, NaiveDate};
use postio_model::{AddressId, LabelId, MailboxId};

/// Which slice of the mailbox a search looks at.
///
/// Not a mailbox id: this is the standing, no-typing rescope from the canvas'
/// left column, and it has to mean the same thing before any folder has been
/// picked. `in:` is still there for naming one folder exactly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// Every folder except drafts, junk and trash. The default, because
    /// search is how you find the thing you filed somewhere and forgot --
    /// and those three are the folders a search almost never means, the ones
    /// that turned one correspondent's name into sixty-six hits (#1523). Sent
    /// stays in. An `in:` naming one of the three reaches it; the executor
    /// lifts the exclusion for any query that names a folder.
    #[default]
    AllMail,
    /// Only what is still in the inbox.
    Inbox,
    /// Mailing-list traffic.
    ///
    /// **Still an approximation, though `List-Id` is indexed now (#9,
    /// `messages.list_id`; see [`Filter::List`]):** this scope is "in a
    /// mailbox that is not one of the standard roles" — the folders list
    /// mail is filed into — rather than "carries a `List-Id`", which is a
    /// deliberately different question the sidebar's own Lists section (not
    /// yet built) would answer instead. That is right for the setup the
    /// canvas draws and wrong for a mailbox that files list mail into the
    /// inbox; the count says so either way, which is what keeps it honest
    /// rather than silently empty.
    ///
    /// [`Filter::List`]: crate::query::Filter::List
    Lists,
}

impl Scope {
    /// Every scope, in the order the canvas' left column lists them.
    pub const ALL: [Scope; 3] = [Scope::AllMail, Scope::Inbox, Scope::Lists];

    /// What the column calls it.
    pub const fn label(self) -> &'static str {
        match self {
            Scope::AllMail => "All mail",
            Scope::Inbox => "Inbox only",
            Scope::Lists => "Lists",
        }
    }
}

/// One scope and how many of the query's matches are in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeCount {
    /// The scope.
    pub scope: Scope,
    /// Matches inside it, counted the same way and to the same cap as
    /// [`SearchResults::total_hits`](crate::results::SearchResults::total_hits).
    pub hits: u64,
}

/// One offered narrowing: a token to append, and what it would leave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refinement {
    /// The query token, exactly as it would be typed — `is:unread`.
    pub token: String,
    /// How many of the current matches it keeps.
    pub hits: u64,
}

/// How many refine chips the column offers at once.
///
/// Four, from the canvas. It is a shortlist, not a filter panel: a column of
/// twenty chips is a thing to read rather than a thing to click.
pub const MAX_REFINEMENTS: usize = 4;

/// What a query's result set turned out to be made of.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facets {
    /// Matches per scope, in [`Scope::ALL`] order.
    pub scopes: Vec<ScopeCount>,
    /// Every narrowing that was measured, before [`Facets::suggested`] picks
    /// which are worth offering.
    pub refinements: Vec<Refinement>,
}

impl Facets {
    /// Matches in `scope`, or zero if it was not measured.
    pub fn hits(&self, scope: Scope) -> u64 {
        self.scopes
            .iter()
            .find(|count| count.scope == scope)
            .map(|count| count.hits)
            .unwrap_or(0)
    }

    /// The refinements worth offering, best first, at most
    /// [`MAX_REFINEMENTS`] of them.
    ///
    /// `total` is the size of the result set they were measured against. A
    /// refinement that keeps none of it is a dead end; one that keeps all of
    /// it narrows nothing and would appear to do nothing when clicked.
    /// Neither is offered.
    ///
    /// The survivors are ordered by how much they keep, largest first: the
    /// gentlest useful narrowing is the one it is safest to try, and the
    /// drastic ones stay reachable one row down rather than leading.
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

/// The query `query` becomes when `token` is appended.
///
/// Idempotent: a token already in the query comes back unchanged, so
/// clicking a chip twice cannot produce `is:unread is:unread`. In practice a
/// refinement disappears from [`Facets::suggested`] the moment it is applied
/// — it then matches every remaining hit — but that depends on a round trip,
/// and a double click does not wait for one.
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

/// The facets of a conversation search (spec 010): what the filter
/// buttons, their popovers and the timeline count.
///
/// Beside [`Facets`], which GTK's search keeps. Every count is of
/// conversations, and of the query with that term added (SC-008). Names and
/// addresses are not here: `postio-session` resolves the ids it returns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFacets {
    /// Senders, the top 50 by count.
    pub senders: Vec<Count<AddressId>>,
    /// Recipients (To, Cc), the top 50 by count.
    pub recipients: Vec<Count<AddressId>>,
    /// Labels.
    pub labels: Vec<Count<LabelId>>,
    /// Folders.
    pub folders: Vec<Count<MailboxId>>,
    /// Conversations with an attachment.
    pub attachment: u64,
    /// Conversations with an open marker (`has:action`).
    pub action: u64,
    /// Conversations with something unread.
    pub unread: u64,
    /// The timeline: oldest first, ending with today's month
    /// ([`months_ending`]).
    pub months: [MonthCount; 12],
    /// The Date popover's presets: last 7 days, last 30 days, this quarter,
    /// this year, any time.
    pub presets: [u64; 5],
    /// The match was capped: every count is a floor.
    pub capped: bool,
}

/// How many conversations one facet value holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Count<T> {
    /// The value: a person, a label, a folder.
    pub id: T,
    /// Conversations.
    pub conversations: u64,
}

/// One bar of the timeline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MonthCount {
    /// The month's first day.
    pub month: NaiveDate,
    /// Conversations whose newest match fell in it (D4).
    pub conversations: u64,
}

/// The first days of the twelve months ending with `today`'s, oldest first:
/// the timeline's bars.
pub fn months_ending(today: NaiveDate) -> [NaiveDate; 12] {
    let this = today.with_day(1).unwrap_or(today);
    std::array::from_fn(|index| {
        let back = u32::try_from(11 - index).unwrap_or(0);
        this.checked_sub_months(Months::new(back)).unwrap_or(this)
    })
}

/// Where the Date popover's five presets start, in [`SearchFacets::presets`]
/// order: last 7 days, last 30 days, this quarter, this year, and any time
/// (`None`, no start).
///
/// Each is the `after:` its words lower to, so a preset's count is the
/// count of the query with that `after:` added: "the last 30 days" is
/// [`crate::natural`]'s rolling `after:` thirty days back, and a quarter or
/// a year starts on its first day.
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
        // "the last 7 days" and "the last 30 days" lower to an `after:` that
        // many days back (`natural`); a quarter and a year from their first
        // day; any time has no start.
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

    // -- appending --------------------------------------------------------

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
