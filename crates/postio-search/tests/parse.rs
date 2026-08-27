//! Acceptance tests for the query-operator parser (bead postio-djq).
//!
//! These exercise the parser only through its public API, and they are the
//! contract the search bar and the query executor code against.

use chrono::NaiveDate;
use postio_search::parse;
use postio_search::query::{Field, Filter, ParsedQuery, State, TokenKind};

/// Fixed reference date so every relative-date expectation is deterministic.
/// 2026-08-22 is a Saturday.
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 8, 22).unwrap()
}

fn q(input: &str) -> ParsedQuery {
    parse(input, today())
}

fn filters(input: &str) -> Vec<Filter> {
    q(input).filters().map(|c| c.filter.clone()).collect()
}

fn text(input: &str) -> Vec<String> {
    q(input).text_terms().map(|t| t.value.clone()).collect()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

// ---------------------------------------------------------------------------
// Every operator from the design canvas
// ---------------------------------------------------------------------------

#[test]
fn from_operator() {
    assert_eq!(filters("from:lena"), vec![Filter::From("lena".into())]);
}

#[test]
fn from_operator_with_full_address() {
    assert_eq!(
        filters("from:alice@example.com"),
        vec![Filter::From("alice@example.com".into())]
    );
}

#[test]
fn to_operator() {
    assert_eq!(
        filters("to:bob@example.com"),
        vec![Filter::To("bob@example.com".into())]
    );
}

#[test]
fn subject_operator() {
    assert_eq!(
        filters("subject:invoice"),
        vec![Filter::Subject("invoice".into())]
    );
}

#[test]
fn has_attach_and_has_attachment_are_the_same_filter() {
    assert_eq!(filters("has:attach"), vec![Filter::HasAttachment]);
    assert_eq!(filters("has:attachment"), vec![Filter::HasAttachment]);
    assert_eq!(filters("has:attachments"), vec![Filter::HasAttachment]);
}

#[test]
fn is_unread_and_is_flagged() {
    assert_eq!(filters("is:unread"), vec![Filter::Is(State::Unread)]);
    assert_eq!(filters("is:flagged"), vec![Filter::Is(State::Flagged)]);
    assert_eq!(filters("is:read"), vec![Filter::Is(State::Read)]);
}

#[test]
fn is_starred_is_accepted_as_a_synonym_for_flagged() {
    // An earlier brief said `is:starred`; the canvas renamed it to Flagged, and
    // docs/PRODUCT.md §7 keeps the old spelling so muscle memory from other
    // clients still works.
    assert_eq!(filters("is:starred"), vec![Filter::Is(State::Flagged)]);
}

#[test]
fn before_and_after_operators() {
    assert_eq!(
        filters("after:2026-01-01"),
        vec![Filter::After(date(2026, 1, 1))]
    );
    assert_eq!(
        filters("before:2026-02-01"),
        vec![Filter::Before(date(2026, 2, 1))]
    );
}

#[test]
fn in_operator() {
    assert_eq!(filters("in:archive"), vec![Filter::In("archive".into())]);
}

#[test]
fn filename_operator() {
    assert_eq!(
        filters("filename:contract"),
        vec![Filter::Filename("contract".into())]
    );
}

#[test]
fn list_operator() {
    assert_eq!(filters("list:lkml"), vec![Filter::List("lkml".into())]);
}

#[test]
fn account_operator() {
    // The value stays text here. This crate never resolves it to an id — that
    // needs the store, and keeping the parse pure is what lets a saved search
    // survive in `[filters]` as the string the user typed (ADR 0005 Q5, #186).
    assert_eq!(
        filters("account:work"),
        vec![Filter::Account("work".into())]
    );
    assert_eq!(
        filters(r#"account:"Work Mail""#),
        vec![Filter::Account("Work Mail".into())]
    );
    // An address is a perfectly good way to name an account, and the one a
    // person is most likely to remember.
    assert_eq!(
        filters("account:ada@example.com"),
        vec![Filter::Account("ada@example.com".into())]
    );
}

#[test]
fn account_composes_with_other_operators_and_with_negation() {
    // The whole point of the orthogonal shape #186 chose: "this account, and
    // unread" is one query rather than two mutually exclusive scopes.
    assert_eq!(
        filters("account:work is:unread"),
        vec![Filter::Account("work".into()), Filter::Is(State::Unread),]
    );

    let parsed = q("-account:work");
    let clause = parsed.filters().next().unwrap();
    assert!(clause.negated, "`-account:` means every other account");
    assert_eq!(clause.filter, Filter::Account("work".into()));
}

#[test]
fn group_operator() {
    // Like account:, this stays text: postio-search never resolves a group
    // name to its members -- that needs the store, and postio-index is
    // where `group:family` becomes an address set (ADR 0007 Q3).
    assert_eq!(
        filters("group:family"),
        vec![Filter::Group("family".into())]
    );
    assert_eq!(
        filters(r#"group:"Book club""#),
        vec![Filter::Group("Book club".into())]
    );
}

#[test]
fn group_composes_with_other_operators_and_with_negation() {
    assert_eq!(
        filters("group:family is:unread"),
        vec![Filter::Group("family".into()), Filter::Is(State::Unread)]
    );

    let parsed = q("-group:family");
    let clause = parsed.filters().next().unwrap();
    assert!(clause.negated, "`-group:` means everyone outside the group");
    assert_eq!(clause.filter, Filter::Group("family".into()));
}

#[test]
fn larger_operator_with_size_suffixes() {
    assert_eq!(filters("larger:1M"), vec![Filter::Larger(1024 * 1024)]);
    assert_eq!(filters("larger:1m"), vec![Filter::Larger(1024 * 1024)]);
    assert_eq!(filters("larger:1MB"), vec![Filter::Larger(1024 * 1024)]);
    assert_eq!(filters("larger:512k"), vec![Filter::Larger(512 * 1024)]);
    assert_eq!(
        filters("larger:2G"),
        vec![Filter::Larger(2 * 1024 * 1024 * 1024)]
    );
    assert_eq!(filters("larger:1024"), vec![Filter::Larger(1024)]);
    assert_eq!(filters("larger:1.5M"), vec![Filter::Larger(1_572_864)]);
}

#[test]
fn smaller_operator() {
    assert_eq!(filters("smaller:1M"), vec![Filter::Smaller(1024 * 1024)]);
}

#[test]
fn operator_names_are_case_insensitive_but_values_are_not() {
    assert_eq!(filters("FROM:Alice"), vec![Filter::From("Alice".into())]);
    assert_eq!(filters("Is:Unread"), vec![Filter::Is(State::Unread)]);
}

// ---------------------------------------------------------------------------
// Composition with free text
// ---------------------------------------------------------------------------

#[test]
fn operators_compose_with_each_other_and_with_free_text() {
    let parsed = q("from:alice after:2026-01-01 has:attachment kubernetes");
    assert_eq!(
        parsed
            .filters()
            .map(|c| c.filter.clone())
            .collect::<Vec<_>>(),
        vec![
            Filter::From("alice".into()),
            Filter::After(date(2026, 1, 1)),
            Filter::HasAttachment,
        ]
    );
    assert_eq!(
        parsed
            .text_terms()
            .map(|t| t.value.clone())
            .collect::<Vec<_>>(),
        vec!["kubernetes".to_string()]
    );
}

#[test]
fn the_canvas_query_parses() {
    let parsed = q("from:lena after:aug1 has:attach");
    assert_eq!(
        parsed
            .filters()
            .map(|c| c.filter.clone())
            .collect::<Vec<_>>(),
        vec![
            Filter::From("lena".into()),
            Filter::After(date(2026, 8, 1)),
            Filter::HasAttachment,
        ]
    );
    assert!(parsed.text_terms().next().is_none());
    assert!(parsed.partials().next().is_none());
}

#[test]
fn free_text_can_surround_operators() {
    let parsed = q("kubernetes from:alice migration");
    assert_eq!(
        parsed
            .text_terms()
            .map(|t| t.value.clone())
            .collect::<Vec<_>>(),
        vec!["kubernetes".to_string(), "migration".to_string()]
    );
    assert_eq!(filters("kubernetes from:alice migration").len(), 1);
}

#[test]
fn repeated_operators_are_all_kept() {
    assert_eq!(
        filters("from:alice from:bob"),
        vec![Filter::From("alice".into()), Filter::From("bob".into())]
    );
}

#[test]
fn extra_whitespace_is_irrelevant() {
    let parsed = q("   from:alice \t  has:attach   kubernetes  ");
    assert_eq!(parsed.filters().count(), 2);
    assert_eq!(parsed.text_terms().count(), 1);
}

// ---------------------------------------------------------------------------
// Quoted phrases
// ---------------------------------------------------------------------------

#[test]
fn quoted_operator_value() {
    assert_eq!(
        filters(r#"subject:"quarterly report""#),
        vec![Filter::Subject("quarterly report".into())]
    );
}

#[test]
fn quoted_free_text_is_one_term() {
    assert_eq!(
        text(r#""quarterly report" kubernetes"#),
        vec!["quarterly report".to_string(), "kubernetes".to_string()]
    );
}

#[test]
fn an_unterminated_quote_is_still_a_value() {
    // Half-typed phrase: the user is mid-keystroke, this must not error.
    assert_eq!(
        filters(r#"subject:"quarterly rep"#),
        vec![Filter::Subject("quarterly rep".into())]
    );
    assert_eq!(
        text(r#"hello "world"#),
        vec!["hello".to_string(), "world".to_string()]
    );
}

#[test]
fn a_lone_quote_is_not_a_term() {
    let parsed = q(r#"from:alice ""#);
    assert_eq!(parsed.filters().count(), 1);
    assert_eq!(parsed.text_terms().count(), 0);
}

#[test]
fn quoted_value_may_contain_a_colon() {
    assert_eq!(
        filters(r#"subject:"re: invoice""#),
        vec![Filter::Subject("re: invoice".into())]
    );
}

// ---------------------------------------------------------------------------
// Negation
// ---------------------------------------------------------------------------

#[test]
fn negated_operator() {
    let parsed = q("-from:bob");
    let clause = parsed.filters().next().unwrap();
    assert!(clause.negated);
    assert_eq!(clause.filter, Filter::From("bob".into()));
}

#[test]
fn negated_free_text() {
    let parsed = q("kubernetes -docker");
    let terms: Vec<_> = parsed.text_terms().collect();
    assert_eq!(terms.len(), 2);
    assert!(!terms[0].negated);
    assert!(terms[1].negated);
    assert_eq!(terms[1].value, "docker");
}

#[test]
fn not_prefix_is_also_negation() {
    let parsed = q("NOT:from:bob");
    // `NOT:` is not an operator; only a leading `-` negates. Ensure we do not
    // silently swallow it as a filter.
    assert!(parsed.filters().next().is_none());
}

#[test]
fn a_bare_dash_is_free_text_not_a_negation() {
    let parsed = q("-");
    assert_eq!(parsed.filters().count(), 0);
    assert_eq!(parsed.partials().count(), 0);
    assert_eq!(parsed.text_terms().count(), 0);
}

#[test]
fn a_dash_inside_a_word_does_not_negate() {
    assert_eq!(text("re-index"), vec!["re-index".to_string()]);
}

#[test]
fn negated_partial_is_still_partial() {
    let parsed = q("-is:");
    let partial = parsed.partials().next().unwrap();
    assert!(partial.negated);
    assert_eq!(partial.field, Field::Is);
}

// ---------------------------------------------------------------------------
// Half-typed input — the critical requirement
// ---------------------------------------------------------------------------

#[test]
fn empty_input_is_an_empty_query() {
    let parsed = q("");
    assert!(parsed.is_empty());
    assert_eq!(parsed.tokens().len(), 0);
    assert_eq!(parsed.fts_match(), None);
}

#[test]
fn whitespace_only_input_is_an_empty_query() {
    assert!(q("   \t ").is_empty());
}

#[test]
fn operator_with_no_value_yet_is_a_partial() {
    for (input, field) in [
        ("from:", Field::From),
        ("to:", Field::To),
        ("subject:", Field::Subject),
        ("has:", Field::Has),
        ("is:", Field::Is),
        ("before:", Field::Before),
        ("after:", Field::After),
        ("in:", Field::In),
        ("filename:", Field::Filename),
        ("larger:", Field::Larger),
        ("list:", Field::List),
    ] {
        let parsed = q(input);
        assert_eq!(parsed.filters().count(), 0, "{input} should not filter yet");
        let partial = parsed
            .partials()
            .next()
            .unwrap_or_else(|| panic!("{input} should be a partial"));
        assert_eq!(partial.field, field, "{input}");
        assert_eq!(partial.value, "", "{input}");
        assert_eq!(parsed.text_terms().count(), 0, "{input}");
    }
}

#[test]
fn half_typed_text_value_filters_immediately() {
    // `from:al` is a usable filter as-you-type: it is a prefix match downstream.
    assert_eq!(filters("from:al"), vec![Filter::From("al".into())]);
    assert_eq!(
        filters("subject:quar"),
        vec![Filter::Subject("quar".into())]
    );
}

#[test]
fn half_typed_enumerated_value_is_a_partial_not_an_error() {
    for input in ["is:unr", "is:", "is:x", "has:att", "has:zz"] {
        let parsed = q(input);
        assert_eq!(parsed.filters().count(), 0, "{input}");
        assert_eq!(parsed.partials().count(), 1, "{input}");
    }
    // `has:att` is a prefix of `attach`, but it is not yet one of the accepted
    // spellings, so it stays partial rather than guessing.
    let parsed = q("is:unr");
    assert_eq!(parsed.partials().next().unwrap().value, "unr");
}

#[test]
fn half_typed_date_is_a_partial_not_an_error() {
    for input in ["after:2026-", "after:20", "before:au", "after:notadate"] {
        let parsed = q(input);
        assert_eq!(parsed.filters().count(), 0, "{input} should not filter");
        assert_eq!(parsed.partials().count(), 1, "{input} should be partial");
    }
}

#[test]
fn an_impossible_date_is_a_partial_not_an_error() {
    assert_eq!(q("after:2026-13-45").partials().count(), 1);
    assert_eq!(q("after:2026-02-30").partials().count(), 1);
}

#[test]
fn half_typed_size_is_a_partial_not_an_error() {
    for input in ["larger:", "larger:big", "larger:M", "larger:1X"] {
        let parsed = q(input);
        assert_eq!(parsed.filters().count(), 0, "{input}");
        assert_eq!(parsed.partials().count(), 1, "{input}");
    }
}

#[test]
fn an_unknown_operator_is_free_text() {
    // `foo:` is not ours; treat it as something the user typed, not an error.
    assert_eq!(text("foo:bar"), vec!["foo:bar".to_string()]);
    assert_eq!(
        text("https://example.com"),
        vec!["https://example.com".to_string()]
    );
}

#[test]
fn a_lone_colon_is_free_text() {
    assert_eq!(text(":"), vec![":".to_string()]);
}

#[test]
fn every_prefix_of_a_complex_query_parses_cleanly() {
    let full = r#"-from:alice@example.com to:bob subject:"quarterly report" has:attach is:unread before:2026-02-01 after:aug1 in:archive filename:contract.pdf larger:1.5M list:lkml kubernetes -docker "release notes""#;
    for end in 0..=full.len() {
        if !full.is_char_boundary(end) {
            continue;
        }
        let prefix = &full[..end];
        let parsed = parse(prefix, today());
        // Every token must point at a valid, in-order, in-bounds slice.
        let mut cursor = 0usize;
        for token in parsed.tokens() {
            assert!(
                token.span.start >= cursor,
                "overlapping spans in {prefix:?}"
            );
            assert!(
                token.span.end <= prefix.len(),
                "span past end in {prefix:?}"
            );
            assert!(
                token.span.start <= token.span.end,
                "inverted span in {prefix:?}"
            );
            assert_eq!(
                &prefix[token.span.start..token.span.end],
                token.raw,
                "raw does not match its span in {prefix:?}"
            );
            cursor = token.span.end;
        }
        // And the match expression must always be buildable.
        let _ = parsed.fts_match();
    }
}

#[test]
fn growing_a_query_one_character_at_a_time_never_panics() {
    let inputs = [
        "from:lena after:aug1 has:attach",
        r#"subject:"re: invoice" -is:unread larger:1M"#,
        "\u{1f600} from:josé in:Travées/2026 \"naïve phrase\"",
        "::::",
        "----",
        "\"\"\"\"",
        "a:b:c:d",
        "  \t\n  ",
    ];
    for input in inputs {
        for end in 0..=input.len() {
            if input.is_char_boundary(end) {
                let _ = parse(&input[..end], today());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dates: ISO, loose and relative
// ---------------------------------------------------------------------------

#[test]
fn iso_dates() {
    let cases = [
        ("after:2026-01-01", date(2026, 1, 1)),
        ("after:2026/01/01", date(2026, 1, 1)),
        ("after:2026-1-1", date(2026, 1, 1)),
        ("after:20260101", date(2026, 1, 1)),
        ("after:2026.01.01", date(2026, 1, 1)),
    ];
    for (input, expected) in cases {
        assert_eq!(filters(input), vec![Filter::After(expected)], "{input}");
    }
}

#[test]
fn loose_month_name_dates() {
    let cases = [
        ("after:aug1", date(2026, 8, 1)),
        ("after:Aug1", date(2026, 8, 1)),
        ("after:august1", date(2026, 8, 1)),
        ("after:aug-1", date(2026, 8, 1)),
        ("after:1aug", date(2026, 8, 1)),
        ("after:aug1,2025", date(2025, 8, 1)),
        ("after:aug-1-2025", date(2025, 8, 1)),
        (r#"after:"august 1""#, date(2026, 8, 1)),
        (r#"after:"1 august 2025""#, date(2025, 8, 1)),
        ("after:sept1", date(2025, 9, 1)),
    ];
    for (input, expected) in cases {
        assert_eq!(filters(input), vec![Filter::After(expected)], "{input}");
    }
}

#[test]
fn a_bare_month_name_is_the_first_of_that_month() {
    // Most recent occurrence that is not in the future, relative to `today`.
    assert_eq!(filters("after:aug"), vec![Filter::After(date(2026, 8, 1))]);
    assert_eq!(filters("after:dec"), vec![Filter::After(date(2025, 12, 1))]);
}

#[test]
fn numeric_dates_without_a_year_infer_the_most_recent_past_year() {
    assert_eq!(filters("after:8/1"), vec![Filter::After(date(2026, 8, 1))]);
    assert_eq!(filters("after:9/1"), vec![Filter::After(date(2025, 9, 1))]);
}

#[test]
fn month_day_year_numeric_dates() {
    assert_eq!(
        filters("after:1/2/2026"),
        vec![Filter::After(date(2026, 1, 2))]
    );
}

#[test]
fn relative_dates() {
    let cases = [
        ("after:today", date(2026, 8, 22)),
        ("after:yesterday", date(2026, 8, 21)),
        ("after:last-week", date(2026, 8, 15)),
        (r#"after:"last week""#, date(2026, 8, 15)),
        ("after:lastweek", date(2026, 8, 15)),
        (r#"after:"last month""#, date(2026, 7, 22)),
        (r#"after:"last quarter""#, date(2026, 5, 22)),
        (r#"after:"last year""#, date(2025, 8, 22)),
        ("after:7d", date(2026, 8, 15)),
        ("after:2w", date(2026, 8, 8)),
        ("after:3m", date(2026, 5, 22)),
        ("after:1y", date(2025, 8, 22)),
        (r#"after:"3 days ago""#, date(2026, 8, 19)),
    ];
    for (input, expected) in cases {
        assert_eq!(filters(input), vec![Filter::After(expected)], "{input}");
    }
}

#[test]
fn relative_dates_clamp_at_month_ends() {
    // 2026-03-31 minus one month has no 31st in February.
    let parsed = parse(r#"after:"last month""#, date(2026, 3, 31));
    assert_eq!(
        parsed
            .filters()
            .map(|c| c.filter.clone())
            .collect::<Vec<_>>(),
        vec![Filter::After(date(2026, 2, 28))]
    );
}

// ---------------------------------------------------------------------------
// FTS5 MATCH expression
// ---------------------------------------------------------------------------

#[test]
fn match_expression_only_carries_free_text() {
    let parsed = q("from:alice kubernetes migration");
    assert_eq!(
        parsed.fts_match().as_deref(),
        Some(r#""kubernetes" AND "migration""#)
    );
}

#[test]
fn match_expression_is_none_when_there_is_no_free_text() {
    assert_eq!(q("from:alice is:unread").fts_match(), None);
}

#[test]
fn match_expression_quotes_phrases() {
    let parsed = q(r#""quarterly report""#);
    assert_eq!(parsed.fts_match().as_deref(), Some(r#""quarterly report""#));
}

#[test]
fn match_expression_escapes_embedded_quotes() {
    let parsed = q(r#""say ""#);
    // Whatever survives tokenization must still be a legal FTS5 string literal.
    let expr = parsed.fts_match().unwrap();
    assert!(expr.starts_with('"') && expr.ends_with('"'), "{expr}");
}

#[test]
fn match_expression_uses_not_for_negated_text() {
    let parsed = q("kubernetes -docker");
    assert_eq!(
        parsed.fts_match().as_deref(),
        Some(r#"("kubernetes") NOT ("docker")"#)
    );
}

#[test]
fn match_expression_is_none_when_only_negated_text_is_present() {
    // FTS5 has no unary NOT; the executor must handle exclusion itself.
    let parsed = q("-docker");
    assert_eq!(parsed.fts_match(), None);
    assert_eq!(parsed.text_terms().count(), 1);
}

#[test]
fn match_expression_strips_fts_syntax_from_user_text() {
    // Bare `AND`/`OR`/`*`/`(` typed by a user must not become FTS5 operators.
    let parsed = q("kubernetes AND (docker OR podman)*");
    let expr = parsed.fts_match().unwrap();
    for term in expr.split(" AND ") {
        assert!(term.starts_with('"') && term.ends_with('"'), "{expr}");
    }
}

// ---------------------------------------------------------------------------
// Token spans and the chip UI
// ---------------------------------------------------------------------------

#[test]
fn tokens_carry_their_source_span_in_order() {
    let input = "from:lena after:aug1 has:attach";
    let parsed = q(input);
    let spans: Vec<(usize, usize)> = parsed
        .tokens()
        .iter()
        .map(|t| (t.span.start, t.span.end))
        .collect();
    assert_eq!(spans, vec![(0, 9), (10, 20), (21, 31)]);
    for token in parsed.tokens() {
        assert_eq!(&input[token.span.start..token.span.end], token.raw);
    }
}

#[test]
fn a_negated_token_span_includes_the_dash() {
    let input = "-from:bob";
    let parsed = q(input);
    let token = &parsed.tokens()[0];
    assert_eq!((token.span.start, token.span.end), (0, input.len()));
    assert_eq!(token.raw, "-from:bob");
}

#[test]
fn a_quoted_token_span_includes_the_quotes() {
    let input = r#"subject:"quarterly report" x"#;
    let parsed = q(input);
    let token = &parsed.tokens()[0];
    assert_eq!(token.raw, r#"subject:"quarterly report""#);
}

#[test]
fn token_at_finds_the_token_under_the_caret() {
    let input = "from:lena after:aug1";
    let parsed = q(input);
    assert!(matches!(
        parsed.token_at(3).map(|t| &t.kind),
        Some(TokenKind::Filter(_))
    ));
    assert_eq!(parsed.token_at(0).unwrap().raw, "from:lena");
    assert_eq!(parsed.token_at(9).unwrap().raw, "from:lena");
    assert_eq!(parsed.token_at(12).unwrap().raw, "after:aug1");
    // The space between tokens belongs to no token.
    assert!(parsed.token_at(usize::MAX).is_none());
}

#[test]
fn removing_a_token_gives_back_the_remaining_query_text() {
    // This is Backspace popping a chip.
    let parsed = q("from:lena after:aug1 has:attach");
    assert_eq!(parsed.remove_token(1), "from:lena has:attach");
    assert_eq!(parsed.remove_token(0), "after:aug1 has:attach");
    assert_eq!(parsed.remove_token(2), "from:lena after:aug1");
    assert_eq!(parsed.remove_token(9), "from:lena after:aug1 has:attach");
}

#[test]
fn every_token_reports_the_field_it_belongs_to() {
    let parsed = q("from:lena is: kubernetes");
    let fields: Vec<Option<Field>> = parsed.tokens().iter().map(|t| t.field()).collect();
    assert_eq!(fields, vec![Some(Field::From), Some(Field::Is), None]);
}

#[test]
fn field_keywords_round_trip() {
    for field in Field::ALL {
        assert_eq!(Field::parse(field.keyword()), Some(*field));
    }
}

// ---------------------------------------------------------------------------
// Purity
// ---------------------------------------------------------------------------

#[test]
fn parsing_is_deterministic_for_a_given_reference_date() {
    let input = "from:alice after:yesterday has:attach kubernetes";
    let a = parse(input, today());
    let b = parse(input, today());
    assert_eq!(a, b);
}

#[test]
fn the_reference_date_is_the_only_source_of_now() {
    let input = "after:yesterday";
    assert_eq!(
        parse(input, date(2020, 1, 1))
            .filters()
            .map(|c| c.filter.clone())
            .collect::<Vec<_>>(),
        vec![Filter::After(date(2019, 12, 31))]
    );
}

// ---------------------------------------------------------------------------
// Totality: no input, however malformed, may error or panic
// ---------------------------------------------------------------------------

/// A tiny deterministic PRNG so the fuzz below is reproducible in CI.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

#[test]
fn fuzzing_the_parser_with_query_shaped_noise_never_errors() {
    // The alphabet is everything the grammar cares about plus a little Unicode.
    let alphabet: Vec<&str> = vec![
        "from:",
        "to:",
        "subject:",
        "has:",
        "is:",
        "before:",
        "after:",
        "in:",
        "filename:",
        "larger:",
        "smaller:",
        "list:",
        "-",
        "\"",
        ":",
        " ",
        "  ",
        "\t",
        "a",
        "z1",
        "aug1",
        "2026-01-01",
        "1M",
        "unread",
        "attach",
        "last week",
        "@",
        "/",
        ".",
        ",",
        "é",
        "🙂",
        "AND",
        "*",
        "(",
        ")",
        "NEAR",
        "^",
    ];
    let mut rng = Lcg(0x5eed_1234);
    for _ in 0..4000 {
        let len = (rng.next() % 12) as usize;
        let mut input = String::new();
        for _ in 0..len {
            input.push_str(alphabet[(rng.next() as usize) % alphabet.len()]);
        }
        let parsed = parse(&input, today());

        let mut cursor = 0usize;
        for token in parsed.tokens() {
            assert!(token.span.start >= cursor, "{input:?}");
            assert!(token.span.end <= input.len(), "{input:?}");
            assert_eq!(
                &input[token.span.start..token.span.end],
                token.raw,
                "{input:?}"
            );
            cursor = token.span.end;
        }
        // Filters, partials and text partition the tokens exactly.
        assert_eq!(
            parsed.filters().count() + parsed.partials().count() + parsed.text_terms().count(),
            parsed.tokens().len(),
            "{input:?}"
        );
        let _ = parsed.fts_match();
        // Popping any chip must yield text that still parses.
        for index in 0..parsed.tokens().len() {
            let _ = parse(&parsed.remove_token(index), today());
        }
    }
}

#[test]
fn popping_a_chip_removes_exactly_that_token() {
    let parsed = q(r#"from:lena "quarterly report" is:unread larger:1M"#);
    assert_eq!(parsed.tokens().len(), 4);
    for index in 0..parsed.tokens().len() {
        let popped = parsed.remove_token(index);
        let reparsed = parse(&popped, today());
        assert_eq!(reparsed.tokens().len(), 3, "{popped:?}");
        assert!(
            !reparsed
                .tokens()
                .iter()
                .any(|t| t.raw == parsed.tokens()[index].raw),
            "{popped:?} still contains the popped chip"
        );
    }
}
