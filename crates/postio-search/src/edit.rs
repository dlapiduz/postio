//! Term edits: the one way anything but the keyboard changes a query.
//!
//! The query **string** is the one source of truth (spec 010, D1). A chip's
//! ✕, a filter button, a popover's check, the timeline's band and a
//! relaxation each send an [`Edit`], and [`apply`] turns the string the
//! person typed into the next one. Nothing builds query text anywhere else,
//! so there is never a second state to drift from the first.
//!
//! The rules, which are what makes it safe to hand the result straight back
//! to the field the person is typing in:
//!
//! * Tokens keep their order and their raw text, except the one edited:
//!   `after:aug1` stays `after:aug1` when a `from:` beside it changes.
//! * A token the person typed is edited **in place** and never duplicated:
//!   adding `from:ada` to `from:Ada atlas` changes nothing, and adding it to
//!   `from: atlas` fills the half-typed `from:` where it stands.
//! * What is written is [`spell`]'s one canonical form (D13).
//! * A popover's check of a second value of a field the query already holds
//!   is **either**, not both (D26): `from:ada` becomes `from:{ada tomas}`
//!   in place, and unchecking one leaves the plain clause of the other.
//!   [`Edit::Add`] -- a typed operator, a "Narrow to" pill -- still asks
//!   for both, as typing two clauses always has.
//! * The result is whitespace-normalised: one space between tokens.

use chrono::NaiveDate;

use crate::parser::parse;
use crate::query::{Clause, Field, Filter, Token, TokenKind, spell};

/// One change to a query, as a control asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Add a clause, spelled by [`spell`]: in place of a half-typed operator
    /// of its field, or appended. A clause already there is not added again.
    Add(Clause),
    /// Remove one token, by index into [`crate::ParsedQuery::tokens`].
    Remove {
        /// The token's index.
        token: usize,
    },
    /// Put `with` where one token stands.
    Replace {
        /// The token's index.
        token: usize,
        /// What it becomes.
        with: Clause,
    },
    /// A filter button or a popover's check: add the filter if no positive
    /// clause holds it -- into the field's positive clause, as a set, when
    /// the field takes one (D26) -- and take it out of every positive clause
    /// that does. An exclusion of it is turned round rather than
    /// contradicted.
    Toggle(Filter),
    /// The timeline and the Date popover: replaces every `after:` and
    /// `before:` (half-typed ones too), the first of each in place.
    SetDates {
        /// The new lower bound, inclusive; `None` removes it.
        after: Option<NaiveDate>,
        /// The new upper bound, exclusive; `None` removes it.
        before: Option<NaiveDate>,
    },
    /// Keep the free words, drop every operator, complete or half typed (D24).
    ClearFilters,
    /// ⌥-click in a popover: [`Edit::Toggle`]'s mirror over the negated
    /// clause.
    Exclude(Filter),
    /// Every complete `after:` and `before:`, in place: relative to today
    /// (`after:90d`) when `rolling`, else its calendar day
    /// (`after:2026-07-01`). How a saved search keeps its dates (D14).
    Dates {
        /// Relative, or fixed.
        rolling: bool,
    },
}

/// `query` with `edit` applied. See the module docs for the rules.
///
/// `today` is what relative dates in `query` resolve against, as for
/// [`crate::parse`]; it decides only which tokens are complete.
pub fn apply(query: &str, edit: Edit, today: NaiveDate) -> String {
    let parsed = parse(query, today);
    let tokens = parsed.tokens();
    let mut words: Vec<Option<String>> = tokens.iter().map(|t| Some(t.raw.clone())).collect();
    let clause_at = |index: usize| match &tokens[index].kind {
        TokenKind::Filter(clause) => Some(clause),
        _ => None,
    };

    match edit {
        Edit::Add(clause) => add(tokens, &mut words, &clause),
        Edit::Remove { token } => {
            if let Some(word) = words.get_mut(token) {
                *word = None;
            }
        }
        Edit::Replace { token, with } => {
            if let Some(word) = words.get_mut(token) {
                *word = Some(spell(&with));
            }
        }
        Edit::Toggle(filter) => flip(tokens, &mut words, &filter, false),
        Edit::Exclude(filter) => flip(tokens, &mut words, &filter, true),
        Edit::SetDates { after, before } => {
            for (field, date) in [(Field::After, after), (Field::Before, before)] {
                let spelled = date.map(|date| {
                    spell(&Clause {
                        negated: false,
                        filter: match field {
                            Field::After => Filter::After(date),
                            _ => Filter::Before(date),
                        },
                    })
                });
                let mut placed = false;
                for (i, token) in tokens.iter().enumerate() {
                    if token.field() == Some(field) {
                        words[i] = if placed { None } else { spelled.clone() };
                        placed = true;
                    }
                }
                if !placed && let Some(spelled) = spelled {
                    words.push(Some(spelled));
                }
            }
        }
        Edit::ClearFilters => {
            for (i, token) in tokens.iter().enumerate() {
                if token.field().is_some() {
                    words[i] = None;
                }
            }
        }
        Edit::Dates { rolling } => {
            for (i, clause) in (0..tokens.len()).filter_map(|i| clause_at(i).map(|c| (i, c))) {
                let date = match clause.filter {
                    Filter::After(date) | Filter::Before(date) => date,
                    _ => continue,
                };
                let field = if matches!(clause.filter, Filter::After(_)) {
                    "after"
                } else {
                    "before"
                };
                let sign = if clause.negated { "-" } else { "" };
                words[i] = Some(if rolling {
                    let days = (today - date).num_days().max(0);
                    format!("{sign}{field}:{days}d")
                } else {
                    spell(clause)
                });
            }
        }
    }

    join(words.into_iter().flatten())
}

/// [`Edit::Toggle`] (`negated` false) and [`Edit::Exclude`] (true): take
/// `filter` out of every clause of this polarity that holds it; or, when
/// none does, take it out of the other polarity's and put it in this one's
/// -- extending a clause of its field into a set (D26), else where the
/// other one stood, else as [`add`] would.
fn flip(tokens: &[Token], words: &mut Vec<Option<String>>, filter: &Filter, negated: bool) {
    let here = holders(tokens, filter, negated);
    if !here.is_empty() {
        for (i, clause) in here {
            words[i] = without(clause, filter).map(|rest| spell(&rest));
        }
        return;
    }
    let mut emptied = None;
    for (i, clause) in holders(tokens, filter, !negated) {
        words[i] = match without(clause, filter) {
            Some(rest) => Some(spell(&rest)),
            None => {
                emptied.get_or_insert(i);
                None
            }
        };
    }
    let clause = Clause {
        negated,
        filter: filter.clone(),
    };
    let field = filter.field();
    let joined = field
        .takes_set()
        .then(|| {
            tokens
                .iter()
                .enumerate()
                .find_map(|(i, token)| match &token.kind {
                    TokenKind::Filter(held)
                        if held.negated == negated && held.filter.field() == field =>
                    {
                        let mut members = held.filter.alternatives().to_vec();
                        members.push(filter.clone());
                        Some((i, Filter::any_of(members)?))
                    }
                    _ => None,
                })
        })
        .flatten();
    match (joined, emptied) {
        (Some((i, set)), _) => {
            words[i] = Some(spell(&Clause {
                negated,
                filter: set,
            }))
        }
        (None, Some(i)) => words[i] = Some(spell(&clause)),
        (None, None) => add(tokens, words, &clause),
    }
}

/// The clauses of polarity `negated` that hold `filter`, as a value of
/// their own or one of a set's.
fn holders<'a>(tokens: &'a [Token], filter: &Filter, negated: bool) -> Vec<(usize, &'a Clause)> {
    tokens
        .iter()
        .enumerate()
        .filter_map(|(i, token)| match &token.kind {
            TokenKind::Filter(clause)
                if clause.negated == negated
                    && clause
                        .filter
                        .alternatives()
                        .iter()
                        .any(|held| same(held, filter)) =>
            {
                Some((i, clause))
            }
            _ => None,
        })
        .collect()
}

/// `clause` without `filter` among its values: the rest, or `None` when
/// nothing is left.
fn without(clause: &Clause, filter: &Filter) -> Option<Clause> {
    let rest: Vec<Filter> = clause
        .filter
        .alternatives()
        .iter()
        .filter(|held| !same(held, filter))
        .cloned()
        .collect();
    Filter::any_of(rest).map(|filter| Clause {
        negated: clause.negated,
        filter,
    })
}

/// [`Edit::Add`]'s rule, shared with a toggle that finds nothing to remove.
fn add(tokens: &[Token], words: &mut Vec<Option<String>>, clause: &Clause) {
    let already = tokens.iter().any(|token| {
        matches!(&token.kind, TokenKind::Filter(c)
        if c.negated == clause.negated
            && clause.filter.alternatives().iter().all(|wanted| {
                c.filter.alternatives().iter().any(|held| same(held, wanted))
            }))
    });
    if already {
        return;
    }
    let half_typed = tokens.iter().rposition(|token| {
        matches!(&token.kind, TokenKind::Partial(p)
            if p.negated == clause.negated && p.field == clause.filter.field())
    });
    match half_typed {
        Some(i) => words[i] = Some(spell(clause)),
        None => words.push(Some(spell(clause))),
    }
}

/// Whether two filters ask for the same thing. A name is the same name
/// whatever its case -- `from:Ada` is `from:ada`, `label:atlas` is
/// `label:Atlas` -- because every executor arm that reads one compares it
/// that way.
pub fn same(a: &Filter, b: &Filter) -> bool {
    let fold = |s: &str| s.to_lowercase();
    if matches!(a, Filter::AnyOf(_)) || matches!(b, Filter::AnyOf(_)) {
        // Two sets are the same when each holds every value of the other,
        // in any order.
        let within = |x: &Filter, y: &Filter| {
            x.alternatives()
                .iter()
                .all(|v| y.alternatives().iter().any(|w| same(v, w)))
        };
        return matches!((a, b), (Filter::AnyOf(_), Filter::AnyOf(_)))
            && within(a, b)
            && within(b, a);
    }
    match (a, b) {
        (Filter::From(x), Filter::From(y))
        | (Filter::To(x), Filter::To(y))
        | (Filter::Subject(x), Filter::Subject(y))
        | (Filter::In(x), Filter::In(y))
        | (Filter::Filename(x), Filter::Filename(y))
        | (Filter::List(x), Filter::List(y))
        | (Filter::Account(x), Filter::Account(y))
        | (Filter::Group(x), Filter::Group(y))
        | (Filter::Label(x), Filter::Label(y)) => fold(x) == fold(y),
        _ => a == b,
    }
}

/// The words, one space apart. A phrase or a set left open (the person was
/// still typing it) is closed when anything follows it, or the parser would
/// read what follows as part of it.
pub(crate) fn join(words: impl Iterator<Item = String>) -> String {
    let words: Vec<String> = words.collect();
    let last = words.len().saturating_sub(1);
    let mut out = String::new();
    for (i, mut word) in words.into_iter().enumerate() {
        if i < last && word.matches('"').count() % 2 == 1 {
            word.push('"');
        }
        if i < last && crate::parser::leaves_set_open(&word) {
            word.push('}');
        }
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{Clause, Filter, State, TokenKind};
    use chrono::NaiveDate;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn edit(query: &str, edit: Edit) -> String {
        apply(query, edit, today())
    }

    fn positive(filter: Filter) -> Clause {
        Clause {
            negated: false,
            filter,
        }
    }

    #[test]
    fn add_appends_the_clause_in_its_one_spelling() {
        assert_eq!(
            edit("atlas", Edit::Add(positive(Filter::From("ada".into())))),
            "atlas from:ada"
        );
        assert_eq!(
            edit("", Edit::Add(positive(Filter::HasAttachment))),
            "has:attachment"
        );
        assert_eq!(
            edit(
                "budget",
                Edit::Add(positive(Filter::Label("Q3 close".into())))
            ),
            r#"budget label:"Q3 close""#
        );
        assert_eq!(
            edit(
                "budget",
                Edit::Add(Clause {
                    negated: true,
                    filter: Filter::From("bo".into()),
                })
            ),
            "budget -from:bo"
        );
    }

    #[test]
    fn add_never_duplicates_what_the_user_typed() {
        assert_eq!(
            edit(
                "from:Ada  atlas",
                Edit::Add(positive(Filter::From("ada".into())))
            ),
            "from:Ada atlas",
            "the typed token stays, as typed"
        );
        assert_eq!(
            edit("has:attach x", Edit::Add(positive(Filter::HasAttachment))),
            "has:attach x"
        );
    }

    #[test]
    fn add_fills_a_half_typed_operator_in_place() {
        assert_eq!(
            edit(
                "from: atlas",
                Edit::Add(positive(Filter::From("ada".into())))
            ),
            "from:ada atlas"
        );
        assert_eq!(
            edit(
                "atlas label:",
                Edit::Add(positive(Filter::Label("Atlas".into())))
            ),
            "atlas label:Atlas"
        );
        assert_eq!(
            edit("has:act x", Edit::Add(positive(Filter::HasAction))),
            "has:action x"
        );
    }

    #[test]
    fn remove_drops_one_token_and_keeps_the_rest_as_typed() {
        assert_eq!(
            edit(
                "Atlas  from:ada after:aug1 budget",
                Edit::Remove { token: 1 }
            ),
            "Atlas after:aug1 budget"
        );
        assert_eq!(
            edit("from:ada atlas", Edit::Remove { token: 9 }),
            "from:ada atlas",
            "no such token: nothing changes"
        );
    }

    #[test]
    fn replace_edits_one_token_in_place() {
        assert_eq!(
            edit(
                "from:ada after:aug1 x",
                Edit::Replace {
                    token: 0,
                    with: positive(Filter::From("bo".into())),
                }
            ),
            "from:bo after:aug1 x"
        );
        assert_eq!(
            edit(
                "x is:unr",
                Edit::Replace {
                    token: 1,
                    with: positive(Filter::Is(State::Unread)),
                }
            ),
            "x is:unread"
        );
    }

    #[test]
    fn toggle_adds_when_absent_and_removes_every_positive_copy_when_present() {
        assert_eq!(
            edit("budget", Edit::Toggle(Filter::HasAttachment)),
            "budget has:attachment"
        );
        assert_eq!(
            edit("budget has:attach", Edit::Toggle(Filter::HasAttachment)),
            "budget",
            "the typed spelling is recognised and taken out"
        );
        assert_eq!(
            edit(
                "has:attach x has:attachments",
                Edit::Toggle(Filter::HasAttachment)
            ),
            "x"
        );
        assert_eq!(
            edit("is:new atlas", Edit::Toggle(Filter::Is(State::Unread))),
            "atlas"
        );
        assert_eq!(
            edit("label:atlas", Edit::Toggle(Filter::Label("Atlas".into()))),
            "",
            "a label is the same label whatever its case"
        );
    }

    #[test]
    fn toggle_turns_an_exclusion_round_in_place_rather_than_contradict_it() {
        assert_eq!(
            edit("-has:attach budget", Edit::Toggle(Filter::HasAttachment)),
            "has:attachment budget"
        );
    }

    #[test]
    fn set_dates_replaces_every_date_term_in_place() {
        assert_eq!(
            edit(
                "after:aug1 budget before:2026-09-01",
                Edit::SetDates {
                    after: Some(day(2026, 7, 1)),
                    before: None,
                }
            ),
            "after:2026-07-01 budget"
        );
        assert_eq!(
            edit(
                "budget after:2026- after:march",
                Edit::SetDates {
                    after: Some(day(2026, 7, 1)),
                    before: Some(day(2026, 8, 1)),
                }
            ),
            "budget after:2026-07-01 before:2026-08-01",
            "a half-typed date is a date term too, and one survives"
        );
        assert_eq!(
            edit(
                "budget after:aug1",
                Edit::SetDates {
                    after: None,
                    before: None,
                }
            ),
            "budget"
        );
    }

    #[test]
    fn clear_filters_keeps_only_the_words() {
        assert_eq!(
            edit(
                r#"invoices from:ada after:aug1 is: -budget "q3 plan" label:atlas"#,
                Edit::ClearFilters
            ),
            r#"invoices -budget "q3 plan""#
        );
    }

    #[test]
    fn an_unclosed_phrase_is_closed_before_anything_follows_it() {
        let edited = edit(r#""q3 plan"#, Edit::Add(positive(Filter::HasAttachment)));
        assert_eq!(edited, r#""q3 plan" has:attachment"#);
        let parsed = crate::parse(&edited, today());
        assert_eq!(parsed.tokens().len(), 2);
        assert!(matches!(
            &parsed.tokens()[1].kind,
            TokenKind::Filter(clause) if clause.filter == Filter::HasAttachment
        ));
    }

    fn from(who: &str) -> Filter {
        Filter::From(who.into())
    }

    #[test]
    fn toggle_extends_a_held_field_into_a_set_in_place() {
        // D26: a second person checked in one popover is either of them.
        assert_eq!(
            edit("atlas from:ada budget", Edit::Toggle(from("tomas"))),
            "atlas from:{ada tomas} budget"
        );
        assert_eq!(
            edit("from:{ada tomas}", Edit::Toggle(from("bo"))),
            "from:{ada tomas bo}"
        );
        assert_eq!(
            edit("x in:Inbox", Edit::Toggle(Filter::In("Old Mail".into()))),
            r#"x in:{Inbox "Old Mail"}"#
        );
        assert_eq!(
            edit(
                "label:atlas",
                Edit::Toggle(Filter::Label("Q3 close".into()))
            ),
            r#"label:{atlas "Q3 close"}"#
        );
        assert_eq!(
            edit("from:ada -from:bo", Edit::Toggle(from("tomas"))),
            "from:{ada tomas} -from:bo",
            "the clause that holds the field positively is the one extended"
        );
        assert_eq!(
            edit("is:unread", Edit::Toggle(Filter::Is(State::Flagged))),
            "is:unread is:flagged",
            "a field that takes no set is asked for again, as before"
        );
    }

    #[test]
    fn toggle_takes_one_member_out_of_a_set() {
        assert_eq!(
            edit("from:{ada tomas} x", Edit::Toggle(from("Tomas"))),
            "from:ada x",
            "a set left with one value is the plain clause"
        );
        assert_eq!(
            edit("from:{ada tomas bo}", Edit::Toggle(from("tomas"))),
            "from:{ada bo}"
        );
    }

    #[test]
    fn toggle_takes_a_value_out_of_a_negated_set_to_include_it() {
        assert_eq!(
            edit("-from:{ada tomas} x", Edit::Toggle(from("ada"))),
            "-from:tomas x from:ada"
        );
        assert_eq!(
            edit("-from:ada x", Edit::Toggle(from("ada"))),
            "from:ada x",
            "a plain exclusion is still turned round in place"
        );
    }

    #[test]
    fn exclude_mirrors_toggle_over_the_negated_clause() {
        assert_eq!(edit("atlas", Edit::Exclude(from("ada"))), "atlas -from:ada");
        assert_eq!(
            edit("atlas -from:ada", Edit::Exclude(from("tomas"))),
            "atlas -from:{ada tomas}",
            "a second exclusion is neither"
        );
        assert_eq!(
            edit("atlas -from:{ada tomas}", Edit::Exclude(from("ada"))),
            "atlas -from:tomas",
            "again takes it back out"
        );
        assert_eq!(edit("atlas -from:ada", Edit::Exclude(from("ada"))), "atlas");
        assert_eq!(
            edit("atlas from:ada budget", Edit::Exclude(from("ada"))),
            "atlas -from:ada budget",
            "an included value is excluded in place"
        );
        assert_eq!(
            edit("atlas from:{ada tomas}", Edit::Exclude(from("ada"))),
            "atlas from:tomas -from:ada"
        );
        assert_eq!(
            edit("has:attach x", Edit::Exclude(Filter::HasAttachment)),
            "-has:attachment x"
        );
    }

    #[test]
    fn add_asks_for_both_and_sees_into_a_set_only_to_not_repeat_it() {
        assert_eq!(
            edit("from:ada", Edit::Add(positive(from("tomas")))),
            "from:ada from:tomas",
            "a typed operator still means both"
        );
        assert_eq!(
            edit("from:{ada tomas} x", Edit::Add(positive(from("Tomas")))),
            "from:{ada tomas} x"
        );
    }

    #[test]
    fn a_set_left_open_is_closed_before_anything_follows_it() {
        let edited = edit(
            "from:{ada tomas",
            Edit::Add(positive(Filter::HasAttachment)),
        );
        assert_eq!(edited, "from:{ada tomas} has:attachment");
        let parsed = crate::parse(&edited, today());
        assert_eq!(parsed.filters().count(), 2);
        let edited = edit(
            r#"label:{atlas "Q3 cl"#,
            Edit::Add(positive(Filter::HasAttachment)),
        );
        assert_eq!(edited, r#"label:{atlas "Q3 cl"} has:attachment"#);
    }

    #[test]
    fn two_sets_of_the_same_values_are_the_same_filter() {
        let a = crate::parse("from:{ada tomas}", today());
        let b = crate::parse("from:{Tomas ADA}", today());
        let filter = |q: &crate::ParsedQuery| q.filters().next().unwrap().filter.clone();
        assert!(same(&filter(&a), &filter(&b)));
        assert!(!same(&filter(&a), &from("ada")));
    }

    #[test]
    fn dates_turn_rolling_or_fixed_and_nothing_else_moves() {
        // D14: a saved search keeps its dates in the one language, relative
        // when they roll and ISO when they stay put.
        let today = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let rolling = |query: &str| apply(query, Edit::Dates { rolling: true }, today);
        let fixed = |query: &str| apply(query, Edit::Dates { rolling: false }, today);
        assert_eq!(
            rolling("from:ada after:2026-07-01 atlas"),
            "from:ada after:90d atlas"
        );
        assert_eq!(
            fixed("from:ada after:90d atlas"),
            "from:ada after:2026-07-01 atlas"
        );
        assert_eq!(fixed("atlas after:jul1"), "atlas after:2026-07-01");
        assert_eq!(
            rolling("before:2026-09-01 after:2026-08-01"),
            "before:28d after:59d"
        );
        assert_eq!(
            rolling("atlas budget"),
            "atlas budget",
            "no dates, no change"
        );
    }
}
