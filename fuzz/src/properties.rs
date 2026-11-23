use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use postio_body::sanitize::{CID_SCHEME, RemoteImages, Sanitized, sanitize_body};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mime::{self, PREVIEW_CHARS, ParsedMessage};
use postio_model::threading::ThreadCue;

/// A fixed clock, so a find reproduces on any day.
fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

fn fixed_today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).expect("1 January 2026 is a date")
}

/// Run the ingest path on `raw`, every byte attacker-controlled. `mime::parse` is total, so any
/// panic is the find.
pub fn check_parse_message(raw: &[u8]) {
    let parsed = mime::parse(raw);
    check_parsed_invariants(&parsed, raw);

    // The initial-sync path, which buffers no bodies: different code, same hostile bytes.
    let headers_only = mime::parse_headers(raw);
    check_parsed_invariants(&headers_only, raw);

    // Threading reads sender-supplied `Message-ID` and `References`, whose ids become index keys.
    let message = parsed.into_message(AccountId::new(1), MailboxId::new(1), fixed_now());
    let cue = ThreadCue::of(&message);
    for id in postio_model::threading::claimed_ids(&cue) {
        assert!(
            !id.as_str().is_empty(),
            "an empty Message-ID would collide with every other empty one, \
             and threading keys on it"
        );
    }
}

fn check_parsed_invariants(parsed: &ParsedMessage, raw: &[u8]) {
    assert_eq!(
        parsed.size,
        raw.len() as u64,
        "size is the length of what was parsed, not of what parsed cleanly"
    );

    // The preview goes in a 40px list row; an unbounded one would let a sender decide every row's
    // memory cost.
    if let Some(preview) = &parsed.preview {
        assert!(
            preview.chars().count() <= PREVIEW_CHARS,
            "preview is {} chars, over the {PREVIEW_CHARS} cap",
            preview.chars().count()
        );
        assert!(
            !preview.contains('\n'),
            "the preview is flattened to one line; a newline in it would break \
             the row's single-line layout"
        );
    }

    // Attachment filenames are not checked for path separators here: `mime::parse` reports what the
    // sender wrote, and making it safe to write to disk is `postio_ui::reader::parts::save_name`'s
    // promise, tested there.
    let _ = &parsed.parts;
}

/// Sanitize `html` both ways round and check the result. The pane has JavaScript and network off,
/// so this is defence in depth;
/// the returned count is what the UI shows as "remote blocked".
pub fn check_sanitize_html(html: &str) {
    let blocked = sanitize_body(html, RemoteImages::Blocked);
    check_sanitized(&blocked, RemoteImages::Blocked);

    let allowed = sanitize_body(html, RemoteImages::Allowed);
    check_sanitized(&allowed, RemoteImages::Allowed);

    assert_eq!(
        allowed.remote_blocked, 0,
        "nothing is held back when remote images are allowed"
    );

    // Idempotence: the pane re-renders on "show once", so output becomes input, and a second pass
    // that differs is the shape of a filter bypass.
    let twice = sanitize_body(&blocked.html, RemoteImages::Blocked);
    assert_eq!(
        twice.html, blocked.html,
        "sanitizing is not idempotent: a second pass changed the output"
    );
    assert_eq!(
        twice.remote_blocked, 0,
        "a sanitized document has nothing left to block, so a second pass \
         must count nothing"
    );
}

fn check_sanitized(sanitized: &Sanitized, remote: RemoteImages) {
    let html = &sanitized.html;

    // Tag names only, not scheme names: ammonia decodes `&#x6a;avascript&colon;` in text to inert
    // literal `javascript:`, and a substring scan called that a bypass. A literal `<script` in the
    // output can only be an element.
    for banned in ["<script", "<iframe", "<object", "<embed"] {
        assert!(
            !html.to_ascii_lowercase().contains(banned),
            "sanitized output still contains {banned:?}"
        );
    }

    // Only attributes the renderer fetches on its own: an `<a href>` is not a leak, since the click
    // goes through `decide-policy`.
    for (attribute, value) in url_attribute_values(html) {
        if !FETCHED_ON_SIGHT.contains(&attribute) {
            continue;
        }
        let lowered = value.trim().to_ascii_lowercase();
        assert!(
            !lowered.starts_with("javascript:"),
            "a javascript: URL survived in {attribute}: {value:?}"
        );
        // `cid:` is rewritten to CID_SCHEME (`sanitize::rewrite_attribute`), so a surviving bare
        // `cid:` means the rewrite was skipped.
        assert!(
            !lowered.starts_with("cid:"),
            "a bare cid: reference survived in {attribute}; it should be {CID_SCHEME}: by now"
        );
        if remote == RemoteImages::Blocked {
            assert!(
                !is_remote(&lowered),
                "a remote reference survived a blocked render in {attribute}: {value:?}"
            );
        }
    }
}

/// Attribute names the renderer fetches without being asked; `href` is absent on purpose (see
/// `check_sanitized`).
const FETCHED_ON_SIGHT: &[&str] = &["src", "srcset", "poster", "background"];

/// Parse `input` as a query and check the result. The grammar is total, so a panic is a find, and
/// so is a span that does not describe its input: a chip is cut from the query on those offsets.
pub fn check_parse_query(input: &str) {
    let parsed = postio_search::parse(input, fixed_today());

    assert_eq!(
        parsed.input(),
        input,
        "the parsed query should carry the string it was parsed from"
    );

    let mut previous_end = 0usize;
    for (index, token) in parsed.tokens().iter().enumerate() {
        let span = token.span;
        assert!(
            span.start <= span.end,
            "token {index} has an inverted span: {span:?}"
        );
        assert!(
            span.end <= input.len(),
            "token {index} spans past the end of the input: {span:?} in {} bytes",
            input.len()
        );
        assert!(
            input.is_char_boundary(span.start) && input.is_char_boundary(span.end),
            "token {index}'s span splits a character: {span:?} -- \
             slicing the query on these offsets would panic"
        );
        assert!(
            span.start >= previous_end,
            "token {index} overlaps the one before it or is out of order"
        );
        previous_end = span.end;

        assert_eq!(
            token.raw,
            &input[span.start..span.end],
            "token {index}'s raw text is not what its span points at, so the \
             chip's label and what removing it deletes are different things"
        );
    }
}

/// Every attribute value in `html` that names a URL; a crude scan of the bytes that reach WebKit,
/// not a parse.
fn url_attribute_values(html: &str) -> Vec<(&'static str, String)> {
    let mut found = Vec::new();
    for tag in tag_interiors(html) {
        for attribute in ["src", "href", "srcset", "poster", "background"] {
            let needle = format!("{attribute}=\"");
            let mut rest = tag;
            while let Some(at) = rest.find(&needle) {
                rest = &rest[at + needle.len()..];
                match rest.find('"') {
                    Some(end) => {
                        found.push((attribute, rest[..end].to_string()));
                        rest = &rest[end..];
                    }
                    None => break,
                }
            }
        }
    }
    found
}

/// The source text inside each `<...>`, the only place an attribute can be: scanning the whole
/// document reported text as markup.
/// Splitting on `<` is sound because ammonia escapes `<` and `>` in text and attribute values, so
/// this is only correct on `sanitize_body` output.
fn tag_interiors(html: &str) -> Vec<&str> {
    let mut interiors = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        rest = &rest[open + 1..];
        match rest.find('>') {
            Some(close) => {
                interiors.push(&rest[..close]);
                rest = &rest[close + 1..];
            }
            None => break,
        }
    }
    interiors
}

/// Whether a URL value reaches off this machine; mirrors the private
/// `postio_body::sanitize::is_remote`.
fn is_remote(lowered: &str) -> bool {
    lowered.starts_with("http://")
        || lowered.starts_with("https://")
        || lowered.starts_with("//")
        || lowered.starts_with("ftp://")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every check must survive the whole `.eml` corpus; one that asserts something Postio does not
    /// promise shows up here.
    #[test]
    fn the_corpus_satisfies_every_property() {
        for fixture in postio_model::test_corpus::all() {
            check_parse_message(fixture.bytes());

            let parsed = mime::parse(fixture.bytes());
            if let Some(html) = parsed.body.html.as_deref() {
                check_sanitize_html(html);
            }
            if let Some(text) = parsed.body.text.as_deref() {
                check_parse_query(text);
            }
        }
    }

    #[test]
    fn the_edges_are_still_total() {
        for raw in [
            b"".as_slice(),
            b"\0",
            b"\r\n\r\n",
            b"Subject:",
            b"Content-Type: multipart/mixed; boundary=",
            &[0xff, 0xfe, 0xfd],
        ] {
            check_parse_message(raw);
        }
        for html in ["", "<", "<img src=", "&", "\u{feff}", "<!--"] {
            check_sanitize_html(html);
        }
        for query in ["", " ", "\"", "-", ":", "from:", "\u{1f600}", "a\u{0301}b"] {
            check_parse_query(query);
        }
    }

    // A checker that cannot fail reports "clean" forever, so each test hands it a violation of
    // exactly one property.

    fn with_html(html: &str) -> Sanitized {
        Sanitized {
            html: html.to_owned(),
            ..sanitize_body("", RemoteImages::Blocked)
        }
    }

    #[test]
    #[should_panic(expected = "still contains")]
    fn the_sanitizer_check_notices_a_surviving_script() {
        check_sanitized(
            &with_html("<p>hello</p><script>alert(1)</script>"),
            RemoteImages::Blocked,
        );
    }

    #[test]
    #[should_panic(expected = "a remote reference survived")]
    fn the_sanitizer_check_notices_a_surviving_remote_image() {
        check_sanitized(
            &with_html("<img src=\"https://tracker.example.com/pixel.gif\">"),
            RemoteImages::Blocked,
        );
    }

    #[test]
    #[should_panic(expected = "bare cid:")]
    fn the_sanitizer_check_notices_an_unrewritten_cid() {
        check_sanitized(
            &with_html("<img src=\"cid:part1@example.com\">"),
            RemoteImages::Blocked,
        );
    }

    #[test]
    fn a_remote_reference_is_fine_once_the_user_allowed_it() {
        check_sanitized(
            &with_html("<img src=\"https://cdn.example.com/logo.png\">"),
            RemoteImages::Allowed,
        );
    }

    #[test]
    fn the_url_scan_finds_what_it_is_looking_for() {
        let found = url_attribute_values(
            "<a href=\"https://example.com/a\"><img src=\"cid:x\" srcset=\"//e.example.net/b\">",
        );
        assert_eq!(
            found,
            [
                ("href", "https://example.com/a".to_string()),
                ("src", "cid:x".to_string()),
                ("srcset", "//e.example.net/b".to_string()),
            ]
        );

        assert!(url_attribute_values("<img src=\"unterminated").is_empty());
        assert!(url_attribute_values("plain src=\"https://a.example.com\"").is_empty());
    }

    /// A mutation removed the `<` before `img`, leaving escaped text in a paragraph; a
    /// document-wide scan called it a remote reference.
    #[test]
    fn markup_shaped_text_is_not_markup() {
        check_sanitize_html(
            "<p>Hel\u{fffd}\u{000e}img src=\"https://tracker.example.com/open.gif\" width=\"1\">",
        );
    }

    #[test]
    fn the_tag_scan_sees_tags_and_not_text() {
        assert_eq!(tag_interiors("<p>a</p>"), ["p", "/p"]);
        assert_eq!(tag_interiors("no tags here"), Vec::<&str>::new());
        assert_eq!(tag_interiors("<img src=\"x\">"), ["img src=\"x\""]);
        assert_eq!(tag_interiors("<img src=\"x\""), Vec::<&str>::new());
        assert_eq!(
            tag_interiors("hello img src=\"https://a.example.com\""),
            Vec::<&str>::new()
        );
    }

    /// `&#x6a;avascript&colon;` decodes to inert text `javascript:` inside a `<p>`, which is not a
    /// URL.
    #[test]
    fn a_scheme_name_in_text_is_not_a_bypass() {
        check_sanitize_html(
            "<p>&lt;script&gt;&#x6a;avascript&colon;alert(1)</p>\
             <img src=\"&#104;ttps://tracker.example.com/x.gif\">",
        );
    }

    #[test]
    #[should_panic(expected = "a javascript: URL survived")]
    fn a_scheme_name_in_a_fetched_attribute_is_a_bypass() {
        check_sanitized(
            &with_html("<img src=\"javascript:alert(1)\">"),
            RemoteImages::Blocked,
        );
    }

    /// An entity-encoded scheme in an attribute is decoded before the filter sees it, so
    /// `&#104;ttps` does not dodge blocking.
    #[test]
    fn an_entity_encoded_remote_scheme_is_still_blocked() {
        let out = postio_body::sanitize::sanitize_body(
            "<img src=\"&#104;ttps://tracker.example.com/x.gif\">",
            RemoteImages::Blocked,
        );
        assert_eq!(out.remote_blocked, 1, "{}", out.html);
        assert!(!out.html.contains("tracker.example.com"), "{}", out.html);
    }

    /// A remote link is not a remote load.
    #[test]
    fn a_remote_link_is_not_a_remote_load() {
        check_sanitized(
            &with_html("<a href=\"https://news.example.org/issues/214\">read on</a>"),
            RemoteImages::Blocked,
        );
    }

    #[test]
    fn is_remote_agrees_with_the_schemes_the_sanitizer_strips() {
        for remote in [
            "http://a.example.com",
            "https://a.example.com",
            "//a.example.com",
            "ftp://a.example.com",
        ] {
            assert!(is_remote(remote), "{remote} should count as remote");
        }
        for local in ["postio-cid:x", "data:text/plain,x", "#anchor", "/relative"] {
            assert!(!is_remote(local), "{local} should not count as remote");
        }
    }
}
