//! The Contacts screen's rules, without a toolkit (specs/005-contacts).
//!
//! What a row says, what "show mail" searches, and which command means
//! something on which kind of row -- decided here, where a test proves them
//! in milliseconds, so the widget only draws the answers.

use chrono::{DateTime, Local, Utc};
use postio_core::CommandId;
use postio_model::{ContactListRow, ContactSource};

/// The query "show mail" writes for a person: every address they own, in one
/// `with:` field (research R6). Addresses, not the person, so a search pinned
/// from it keeps meaning the addresses it named.
pub fn show_mail_query(addresses: &[String]) -> String {
    let value = addresses.join(",");
    // Quoted only when the value could not survive being typed back in: the
    // parser splits tokens on whitespace, and a quote inside would end it.
    if value.contains(char::is_whitespace) {
        format!("with:\"{}\"", value.replace('"', ""))
    } else {
        format!("with:{value}")
    }
}

/// Whether a row carries the mark for a person the user made or imported
/// (FR-005): the list distinguishes the address book from the mail.
pub fn is_made(source: ContactSource) -> bool {
    !matches!(source, ContactSource::Mail)
}

/// The line under a person's name: how many addresses, and when they were
/// last in touch -- `2 addresses · Thu`, `1 address`.
pub fn secondary_line(row: &ContactListRow, now: DateTime<Local>) -> String {
    let count = match row.address_count {
        1 => "1 address".to_owned(),
        n => format!("{n} addresses"),
    };
    match last_in_touch(row.last_seen_at, now) {
        Some(when) => format!("{count} · {when}"),
        None => count,
    }
}

/// The kind of row the keyboard is on. The Contacts screen is one context,
/// and a command acts on the focused row's kind (contracts/commands.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A person in the list.
    Person,
    /// One of a person's addresses, in the detail.
    Address,
    /// A group.
    Group,
    /// A suggestion that two people may be one.
    Suggestion,
}

/// Why a command did nothing on the focused row: the one-line hint the
/// screen shows instead of failing silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint(pub &'static str);

/// Whether `command` means something on a row of `kind`, and the hint to show
/// when it does not.
pub fn applies(command: CommandId, kind: RowKind) -> Result<(), Hint> {
    use CommandId as C;
    use RowKind as K;
    match (command, kind) {
        // A suggestion is two people; showing mail or writing to it would
        // have to guess which one.
        (C::ContactShowMail | C::ContactCompose, K::Suggestion) => {
            Err(Hint("Choose one of the two people first"))
        }
        // Everything else either names a person, an address or a group --
        // each of which it means something on -- or is the screen's own.
        _ => Ok(()),
    }
}

/// When a person was last in touch, for a row: [`crate::row::timestamp`]'s
/// words, or nothing for someone no mail has involved.
fn last_in_touch(at: Option<DateTime<Utc>>, now: DateTime<Local>) -> Option<String> {
    at.map(|at| crate::row::timestamp(at, now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use postio_model::{ContactId, ContactState};

    fn row(addresses: u32, last: Option<DateTime<Utc>>, source: ContactSource) -> ContactListRow {
        ContactListRow {
            id: ContactId::new(1),
            name: "Ada Lovelace".into(),
            preferred: Some("ada@example.com".into()),
            address_count: addresses,
            last_seen_at: last,
            source,
            state: ContactState::Live,
            sort_key: "ada lovelace".into(),
        }
    }

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 24, 12, 0, 0).unwrap()
    }

    #[test]
    fn show_mail_names_every_address_in_one_with() {
        assert_eq!(
            show_mail_query(&["ada@work.example".into(), "ada@home.example".into()]),
            "with:ada@work.example,ada@home.example"
        );
        assert_eq!(
            show_mail_query(&["grace@example.org".into()]),
            "with:grace@example.org"
        );
    }

    #[test]
    fn show_mail_query_is_read_back_as_the_filter_it_means() {
        let query = show_mail_query(&["ada@work.example".into(), "ada@home.example".into()]);
        let parsed = postio_search::parse(&query, now().date_naive());
        let filters: Vec<_> = parsed.filters().map(|c| c.filter.clone()).collect();
        assert_eq!(
            filters,
            vec![postio_search::query::Filter::With(vec![
                "ada@work.example".into(),
                "ada@home.example".into()
            ])]
        );
    }

    #[test]
    fn an_address_that_could_not_be_typed_back_is_quoted() {
        let query = show_mail_query(&["odd one@example.com".into()]);
        let parsed = postio_search::parse(&query, now().date_naive());
        let filters: Vec<_> = parsed.filters().map(|c| c.filter.clone()).collect();
        assert_eq!(
            filters,
            vec![postio_search::query::Filter::With(vec![
                "odd one@example.com".into()
            ])]
        );
    }

    #[test]
    fn the_address_book_is_marked_and_the_mail_is_not() {
        assert!(is_made(ContactSource::User));
        assert!(is_made(ContactSource::Import));
        assert!(!is_made(ContactSource::Mail));
    }

    #[test]
    fn the_secondary_line_counts_addresses_and_says_when() {
        let today = Utc.with_ymd_and_hms(2026, 9, 24, 9, 14, 0).unwrap();
        assert!(
            secondary_line(&row(2, Some(today), ContactSource::Mail), now())
                .starts_with("2 addresses · "),
        );
        assert_eq!(
            secondary_line(&row(1, None, ContactSource::User), now()),
            "1 address",
            "no mail, no date -- never a date nobody saw"
        );
    }

    #[test]
    fn a_command_acts_on_the_row_kind_it_names_and_hints_elsewhere() {
        assert!(applies(CommandId::ContactShowMail, RowKind::Person).is_ok());
        assert!(applies(CommandId::ContactCompose, RowKind::Person).is_ok());
        assert!(
            applies(CommandId::ContactsFilter, RowKind::Group).is_ok(),
            "the filter is the screen's"
        );
        let hint = applies(CommandId::ContactCompose, RowKind::Suggestion)
            .expect_err("a suggestion is two people; which one would it write to?");
        assert!(!hint.0.is_empty(), "a refusal says what it wants");
    }

    #[test]
    fn last_in_touch_is_nothing_for_someone_no_mail_involved() {
        assert_eq!(last_in_touch(None, now()), None);
    }
}
