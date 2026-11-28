//! Term edits: the one way anything but the keyboard changes a query. The query string is the one
//! source of truth: every control sends an [`Edit`] and [`apply`] turns the typed string into the
//! next one.
//! Tokens keep their order and raw text except the one edited, which is edited in place and never
//! duplicated; what is written is [`spell`]'s canonical form (D13).
//! A popover's check of a second value of a field already held is either, not both (D26):
//! `from:ada` becomes `from:{ada tomas}`; [`Edit::Add`] still asks for both.

use chrono::NaiveDate;

use crate::parser::parse;
use crate::query::{Clause, Field, Filter, Token, TokenKind, spell};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Add(Clause),
    Remove {
        token: usize,
    },
    Replace {
        token: usize,
        with: Clause,
    },
    /// Add the filter if no positive clause holds it (into the field's clause as a set when it
    /// takes one, D26), else take it out of every positive clause that does. An exclusion of it is
    /// turned round rather than contradicted.
    Toggle(Filter),
    SetDates {
        after: Option<NaiveDate>,
        before: Option<NaiveDate>,
    },
    ClearFilters,
    Exclude(Filter),
    /// How a saved search keeps its dates (D14).
    Dates {
        rolling: bool,
    },
}

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

/// A name is the same whatever its case, because every executor arm compares it that way.
pub fn same(a: &Filter, b: &Filter) -> bool {
    let fold = |s: &str| s.to_lowercase();
    if matches!(a, Filter::AnyOf(_)) || matches!(b, Filter::AnyOf(_)) {
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

/// A phrase or set left open is closed when anything follows it, or the parser would read what
/// follows as part of it.
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
