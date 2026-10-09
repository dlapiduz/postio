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
//! * The result is whitespace-normalised: one space between tokens.

use chrono::NaiveDate;

use crate::parser::parse;
use crate::query::{Clause, Field, Filter, TokenKind, spell};

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
    /// A filter button: add the filter if no positive clause of it is there,
    /// remove every positive clause of it if one is. An exclusion of it is
    /// turned round in place rather than contradicted.
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
        Edit::Toggle(filter) => {
            let of_it = |negated: bool| {
                (0..tokens.len())
                    .filter(|&i| {
                        clause_at(i)
                            .is_some_and(|c| c.negated == negated && same(&c.filter, &filter))
                    })
                    .collect::<Vec<_>>()
            };
            let positive = of_it(false);
            let negative = of_it(true);
            let clause = Clause {
                negated: false,
                filter,
            };
            if !positive.is_empty() {
                for i in positive {
                    words[i] = None;
                }
            } else if let Some((&first, rest)) = negative.split_first() {
                words[first] = Some(spell(&clause));
                for &i in rest {
                    words[i] = None;
                }
            } else {
                add(tokens, &mut words, &clause);
            }
        }
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
    }

    join(words.into_iter().flatten())
}

/// [`Edit::Add`]'s rule, shared with a toggle that finds nothing to remove.
fn add(tokens: &[crate::query::Token], words: &mut Vec<Option<String>>, clause: &Clause) {
    let already = tokens.iter().any(|token| {
        matches!(&token.kind, TokenKind::Filter(c)
            if c.negated == clause.negated && same(&c.filter, &clause.filter))
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

/// The words, one space apart. A phrase left open (the person was still
/// typing it) is closed when anything follows it, or the parser would read
/// what follows as part of the phrase.
pub(crate) fn join(words: impl Iterator<Item = String>) -> String {
    let words: Vec<String> = words.collect();
    let last = words.len().saturating_sub(1);
    let mut out = String::new();
    for (i, mut word) in words.into_iter().enumerate() {
        if i < last && word.matches('"').count() % 2 == 1 {
            word.push('"');
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
}
