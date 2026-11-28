use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use postio_model::test_corpus;

use crate::{EventTime, Invitation, parse};

pub(crate) fn calendar_part(name: &str) -> Vec<u8> {
    let parsed = postio_model::mime::parse(test_corpus::load(name).bytes());
    parsed
        .parts
        .iter()
        .find(|part| part.attachment.mime_type == "text/calendar")
        .map(|part| part.content.clone())
        .unwrap_or_else(|| panic!("{name}: no text/calendar part"))
}

pub(crate) fn invitation(name: &str) -> Invitation {
    parse(&calendar_part(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

pub(crate) fn ics(lines: &[&str]) -> Vec<u8> {
    let mut text = lines.join("\r\n");
    text.push_str("\r\n");
    text.into_bytes()
}

pub(crate) fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 instant in the test")
        .with_timezone(&Utc)
}

pub(crate) fn at(text: &str) -> EventTime {
    EventTime::At(utc(text))
}

pub(crate) fn floating(text: &str) -> EventTime {
    EventTime::Floating(
        NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").expect("a local time in the test"),
    )
}

pub(crate) fn day(text: &str) -> EventTime {
    EventTime::Day(NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("a date in the test"))
}

pub(crate) fn unfolded(ics: &[u8]) -> String {
    String::from_utf8(ics.to_vec())
        .expect("a reply is UTF-8")
        .replace("\r\n ", "")
}
