//! Digest rules, read in the one query language (spec 007 T132).
//!
//! `postio-config` keeps a rule's queries as text and does not parse them
//! (`docs/ARCHITECTURE.md`, the one query language), so it validates
//! everything about a rule but this: whether each query reads in full.
//! That check is made here, where the parser is. When a digest is due is
//! [`crate::schedule::next_due`].

use chrono::NaiveDate;
use postio_config::DigestRule;
use postio_search::query::TokenKind;

/// Each of `rule`'s queries that does not read in full, as a sentence
/// naming the rule and the query's place in it (contracts/config.md).
///
/// The language refuses nothing -- half-typed is an ordinary state in the
/// bar -- so here "does not read" is a blank query, or an operator it could
/// not read (`from:` with no address, `is:someday`, `after:2026-`). In a
/// search that constrains nothing; in a rule it would hold mail the rule
/// never meant, so a rule with one is not applied.
pub fn unreadable_queries(rule: &DigestRule, today: NaiveDate) -> Vec<String> {
    let name = rule.name.trim();
    rule.queries
        .iter()
        .enumerate()
        .filter_map(|(place, query)| {
            let place = place + 1;
            if query.trim().is_empty() {
                return Some(format!("query {place} of digest rule `{name}` is empty"));
            }
            let parsed = postio_search::parse(query, today);
            let unread: Vec<String> = parsed
                .tokens()
                .iter()
                .filter(|token| matches!(token.kind, TokenKind::Partial(_)))
                .map(|token| format!("`{}`", token.raw))
                .collect();
            (!unread.is_empty()).then(|| {
                format!(
                    "query {place} of digest rule `{name}` has what the query \
                     language cannot read: {}",
                    unread.join(", ")
                )
            })
        })
        .collect()
}

/// When a rule delivers, as the digest window's sub-row says it:
/// "Weekly, Sunday 09:00", "Daily, 08:00", "Monthly, day 1 09:00"; `None`
/// for a rule whose cadence, day or time names no time.
pub fn rule_when(rule: &DigestRule) -> Option<String> {
    use postio_config::Due;
    Some(match rule.due().ok()? {
        Due::Daily { at } => format!("Daily, {}", at.format("%H:%M")),
        Due::Weekly { day, at } => {
            let day = chrono::NaiveDate::from_isoywd_opt(2026, 1, day)?.format("%A");
            format!("Weekly, {day} {}", at.format("%H:%M"))
        }
        Due::Monthly { day, at } => format!("Monthly, day {day} {}", at.format("%H:%M")),
    })
}

/// The digest window's line under its title: "14 messages from 6 senders
/// · came due today 16:00".
pub fn window_subtitle(
    count: u32,
    senders: usize,
    at: chrono::DateTime<chrono::Local>,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    let messages = if count == 1 { "message" } else { "messages" };
    let people = if senders == 1 { "sender" } else { "senders" };
    let when = if at.date_naive() == now.date_naive() {
        format!("today {}", at.format("%H:%M"))
    } else {
        at.format("%a %-d %b %H:%M").to_string()
    };
    format!("{count} {messages} from {senders} {people} \u{b7} came due {when}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 26).expect("a real date")
    }

    fn rule(name: &str, queries: &[&str]) -> DigestRule {
        DigestRule {
            name: name.to_owned(),
            queries: queries.iter().map(|query| (*query).to_owned()).collect(),
            ..DigestRule::default()
        }
    }

    /// Contracts/config.md: a query that does not parse is reported by the
    /// rule's name and the query's place. The language never refuses a
    /// query -- half-typed is a normal state in the bar -- so here "does
    /// not parse" is an operator it could not read: in a rule, that
    /// constrains nothing, and a rule that holds everything is the worst
    /// thing a digest can do.
    #[test]
    fn an_operator_the_language_cannot_read_is_named_by_rule_and_place() {
        let found = unreadable_queries(
            &rule(
                "Newsletters",
                &["from:news@localfirst.example", "from:", "after:2026-"],
            ),
            today(),
        );
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            found[0].contains("Newsletters")
                && found[0].contains('2')
                && found[0].contains("from:"),
            "{}",
            found[0]
        );
        assert!(
            found[1].contains('3') && found[1].contains("after:2026-"),
            "{}",
            found[1]
        );
    }

    #[test]
    fn a_blank_query_reads_as_nothing() {
        let found = unreadable_queries(&rule("School", &["   "]), today());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("School"), "{}", found[0]);
    }

    #[test]
    fn queries_the_language_reads_in_full_are_not_reported() {
        let found = unreadable_queries(
            &rule(
                "Lists",
                &[
                    "from:news@localfirst.example",
                    "list:harbour-dev.lists.example.org",
                    "is:unread invoice after:yesterday",
                ],
            ),
            today(),
        );
        assert!(found.is_empty(), "{found:?}");
        assert_eq!(
            unreadable_queries(&rule("Lists", &["is:someday"]), today()).len(),
            1,
            "and one it cannot read is reported"
        );
    }

    #[test]
    fn a_rule_says_when_it_delivers() {
        let toml = |text: &str| {
            postio_config::Config::from_toml_str(text)
                .expect("a config")
                .focus
                .digests
                .remove(0)
        };
        let weekly = toml(
            "[[focus.digests]]\nname = \"N\"\nmatch = [\"from:a@b.test\"]\ncadence = \"weekly\"\nday = \"sunday\"\nat = \"09:00\"\n",
        );
        assert_eq!(rule_when(&weekly).as_deref(), Some("Weekly, Sunday 09:00"));
        let daily = toml(
            "[[focus.digests]]\nname = \"N\"\nmatch = [\"from:a@b.test\"]\ncadence = \"daily\"\nat = \"08:00\"\n",
        );
        assert_eq!(rule_when(&daily).as_deref(), Some("Daily, 08:00"));
        let monthly = toml(
            "[[focus.digests]]\nname = \"N\"\nmatch = [\"from:a@b.test\"]\ncadence = \"monthly\"\nday = 1\nat = \"09:00\"\n",
        );
        assert_eq!(rule_when(&monthly).as_deref(), Some("Monthly, day 1 09:00"));
    }

    #[test]
    fn the_window_says_how_much_from_how_many_and_when() {
        use chrono::TimeZone as _;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 9, 26, 16, 9, 0)
            .single()
            .expect("a time");
        let at = chrono::Local
            .with_ymd_and_hms(2026, 9, 26, 16, 0, 0)
            .single()
            .expect("a time");
        assert_eq!(
            window_subtitle(14, 6, at, now),
            "14 messages from 6 senders \u{b7} came due today 16:00"
        );
        assert_eq!(
            window_subtitle(1, 1, at - chrono::Duration::days(1), now),
            "1 message from 1 sender \u{b7} came due Fri 25 Sep 16:00"
        );
    }
}
