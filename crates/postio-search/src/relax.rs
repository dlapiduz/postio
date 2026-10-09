//! Relaxations: the ways out of a search that found nothing.
//!
//! Screen 13 answers zero results with a short list of looser searches, each
//! with how many conversations it would find: "Remove ‘before March’ (12)",
//! "Anyone, not just Ada Moreno (4)", "Look for ‘budget v4’ anywhere, not
//! just the subject (3)". [`relax`] proposes them; the executor counts them
//! and `postio-ui` words them, so neither the count nor the sentence is here.
//!
//! Each relaxation differs from the query by **exactly one token**, so its
//! sentence can say what changed and nothing else changes behind the
//! person's back. The rest of the query keeps its raw text.

use crate::ParsedQuery;
use crate::query::{Clause, Field, Filter, Token, TokenKind, spell};

/// The most relaxations offered: the screen has room for a short list, and
/// a longer one is a sign the query should be cleared instead (D24).
pub const MAX_RELAXATIONS: usize = 8;

/// What a relaxation does to one token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loosen {
    /// Take the token out: "Remove ‘before March’", "Anyone, not just Ada".
    Drop {
        /// The token's index in the query's tokens.
        token: usize,
    },
    /// `subject:v` becomes the words `v`, looked for anywhere.
    Anywhere {
        /// The token's index in the query's tokens.
        token: usize,
    },
    /// `label:v` becomes `in:v`: what was named as a label may be a folder.
    FolderNotLabel {
        /// The token's index in the query's tokens.
        token: usize,
    },
}

/// One looser search: what changed, and the query it gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relaxation {
    /// What was loosened.
    pub loosen: Loosen,
    /// The query with that one change.
    pub query: String,
}

/// The looser searches `query` offers, in token order, at most
/// [`MAX_RELAXATIONS`].
///
/// Every complete filter can be dropped, a set as one term (D26: dropping
/// one of its values narrows, so it is no way out). A positive `subject:`
/// can also be looked for anywhere, and a positive `label:` (or a set of
/// them) can also be a folder; an
/// exclusion is never turned into something else, because `-subject:x` as
/// `-x` would exclude more, not less. A free word is dropped only when
/// there are two or more -- with one, it is the search itself. Half-typed
/// operators constrain nothing, so dropping one would loosen nothing.
pub fn relax(query: &ParsedQuery) -> Vec<Relaxation> {
    let tokens = query.tokens();
    let words = query.text_terms().count();
    let mut out = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match &token.kind {
            TokenKind::Partial(_) => continue,
            TokenKind::Text(_) if words < 2 => continue,
            _ => {}
        }
        out.push(Relaxation {
            loosen: Loosen::Drop { token: index },
            query: rebuilt(tokens, index, None),
        });
        let TokenKind::Filter(Clause {
            negated: false,
            filter,
        }) = &token.kind
        else {
            continue;
        };
        match filter {
            Filter::Subject(value) => out.push(Relaxation {
                loosen: Loosen::Anywhere { token: index },
                query: rebuilt(tokens, index, Some(words_for(value))),
            }),
            // A set of labels is a set of folders just as well (D26).
            filter if filter.field() == Field::Label => {
                let folders = Filter::any_of(
                    filter
                        .alternatives()
                        .iter()
                        .filter_map(|label| match label {
                            Filter::Label(name) => Some(Filter::In(name.clone())),
                            _ => None,
                        })
                        .collect(),
                );
                if let Some(folders) = folders {
                    out.push(Relaxation {
                        loosen: Loosen::FolderNotLabel { token: index },
                        query: rebuilt(
                            tokens,
                            index,
                            Some(spell(&Clause {
                                negated: false,
                                filter: folders,
                            })),
                        ),
                    });
                }
            }
            _ => {}
        }
    }
    out.truncate(MAX_RELAXATIONS);
    out
}

/// The tokens' raw text with the one at `index` replaced, or dropped.
fn rebuilt(tokens: &[Token], index: usize, with: Option<String>) -> String {
    crate::edit::join(tokens.iter().enumerate().filter_map(|(i, token)| {
        if i == index {
            with.clone()
        } else {
            Some(token.raw.clone())
        }
    }))
}

/// `value` as free text: a phrase in quotes when it is more than a word, or
/// when bare it would read as an operator or an exclusion.
fn words_for(value: &str) -> String {
    if value.chars().any(|c| c.is_whitespace() || c == ':') || value.starts_with('-') {
        format!("\"{value}\"")
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;
    use chrono::NaiveDate;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 26).unwrap()
    }

    fn relaxed(query: &str) -> Vec<Relaxation> {
        relax(&parse(query, today()))
    }

    fn raws(query: &str) -> Vec<String> {
        parse(query, today())
            .tokens()
            .iter()
            .map(|token| token.raw.clone())
            .collect()
    }

    #[test]
    fn every_filter_can_be_dropped_in_token_order() {
        let found = relaxed("from:ada has:attach after:2026-07-01 is:unread budget");
        assert_eq!(
            found,
            vec![
                Relaxation {
                    loosen: Loosen::Drop { token: 0 },
                    query: "has:attach after:2026-07-01 is:unread budget".into(),
                },
                Relaxation {
                    loosen: Loosen::Drop { token: 1 },
                    query: "from:ada after:2026-07-01 is:unread budget".into(),
                },
                Relaxation {
                    loosen: Loosen::Drop { token: 2 },
                    query: "from:ada has:attach is:unread budget".into(),
                },
                Relaxation {
                    loosen: Loosen::Drop { token: 3 },
                    query: "from:ada has:attach after:2026-07-01 budget".into(),
                },
            ],
            "one free word is the search itself and is not dropped"
        );
    }

    #[test]
    fn a_subject_can_also_be_looked_for_anywhere() {
        let found = relaxed(r#"subject:"budget v4" from:ada"#);
        assert_eq!(found[0].loosen, Loosen::Drop { token: 0 });
        assert_eq!(
            found[1],
            Relaxation {
                loosen: Loosen::Anywhere { token: 0 },
                query: r#""budget v4" from:ada"#.into(),
            }
        );
        assert_eq!(
            relaxed("subject:budget")[1].query,
            "budget",
            "one word needs no quotes"
        );
    }

    #[test]
    fn a_label_can_also_be_a_folder() {
        let found = relaxed("label:Receipts invoice");
        assert_eq!(
            found,
            vec![
                Relaxation {
                    loosen: Loosen::Drop { token: 0 },
                    query: "invoice".into(),
                },
                Relaxation {
                    loosen: Loosen::FolderNotLabel { token: 0 },
                    query: "in:Receipts invoice".into(),
                },
            ]
        );
    }

    #[test]
    fn a_set_is_dropped_as_one_term_and_never_a_member_at_a_time() {
        // D26: a member dropped from `from:{ada tomas}` narrows the search,
        // so it is no way out of one that found nothing.
        assert_eq!(
            relaxed("from:{ada tomas} has:attach budget"),
            vec![
                Relaxation {
                    loosen: Loosen::Drop { token: 0 },
                    query: "has:attach budget".into(),
                },
                Relaxation {
                    loosen: Loosen::Drop { token: 1 },
                    query: "from:{ada tomas} budget".into(),
                },
            ]
        );
    }

    #[test]
    fn a_set_of_labels_can_also_be_a_set_of_folders() {
        assert_eq!(
            relaxed(r#"label:{Receipts "Q3 close"} invoice"#)[1],
            Relaxation {
                loosen: Loosen::FolderNotLabel { token: 0 },
                query: r#"in:{Receipts "Q3 close"} invoice"#.into(),
            }
        );
        assert_eq!(
            relaxed("subject:{budget plan} x")
                .iter()
                .map(|r| r.loosen)
                .collect::<Vec<_>>(),
            vec![Loosen::Drop { token: 0 }],
            "either subject is not something free words can say"
        );
    }

    #[test]
    fn exclusions_are_dropped_but_never_turned_into_something_wider() {
        // `-subject:x` as `-x` excludes more, not less.
        let found = relaxed("-subject:budget -label:atlas q3");
        assert_eq!(
            found.iter().map(|r| r.loosen).collect::<Vec<_>>(),
            vec![Loosen::Drop { token: 0 }, Loosen::Drop { token: 1 }]
        );
    }

    #[test]
    fn free_words_drop_one_at_a_time_only_when_there_are_two_or_more() {
        assert!(relaxed("budget").is_empty());
        assert_eq!(
            relaxed("atlas budget -draft")
                .into_iter()
                .map(|r| r.query)
                .collect::<Vec<_>>(),
            vec!["budget -draft", "atlas -draft", "atlas budget"]
        );
    }

    #[test]
    fn half_typed_operators_are_not_offered() {
        assert_eq!(
            relaxed("from: budget is:unread")
                .into_iter()
                .map(|r| r.loosen)
                .collect::<Vec<_>>(),
            vec![Loosen::Drop { token: 2 }]
        );
    }

    #[test]
    fn at_most_eight() {
        let found = relaxed(
            "from:ada to:bo subject:x label:y has:attach is:unread after:2026-01-01 \
             before:2026-09-01 larger:1M one two",
        );
        assert_eq!(found.len(), MAX_RELAXATIONS);
        assert_eq!(MAX_RELAXATIONS, 8);
    }

    #[test]
    fn each_relaxed_query_differs_by_exactly_one_token() {
        for query in [
            "from:ada has:attach after:2026-07-01 is:unread budget",
            r#"subject:"budget v4" label:"Q3 close" atlas plan"#,
            r#"label:atlas "q3 plan" -draft in:archive"#,
        ] {
            let before = raws(query);
            for relaxation in relaxed(query) {
                let after = raws(&relaxation.query);
                match relaxation.loosen {
                    Loosen::Drop { token } => {
                        let mut expected = before.clone();
                        expected.remove(token);
                        assert_eq!(after, expected, "{query} → {}", relaxation.query);
                    }
                    Loosen::Anywhere { token } | Loosen::FolderNotLabel { token } => {
                        assert_eq!(after.len(), before.len(), "{}", relaxation.query);
                        let differing: Vec<_> = (0..before.len())
                            .filter(|&i| before[i] != after[i])
                            .collect();
                        assert_eq!(differing, vec![token], "{}", relaxation.query);
                    }
                }
            }
        }
    }
}
