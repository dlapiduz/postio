//! The checks themselves. See the crate docs for the two kinds.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use postio_body::sanitize::{CID_SCHEME, RemoteImages, Sanitized, sanitize_body};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mime::{self, PREVIEW_CHARS, ParsedMessage};
use postio_model::threading::ThreadCue;

/// A fixed clock, so a find reproduces tomorrow.
///
/// `received_at` and the query parser's notion of "today" both feed dates into
/// the code under test. Reading the real clock would make a crash depend on
/// when it was found, and a crash artifact that only reproduces on the day it
/// was discovered is not a bug report.
fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

/// The same date, for [`postio_search::parse`].
fn fixed_today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).expect("1 January 2026 is a date")
}

/// Run the ingest path on `raw` — every byte of it attacker-controlled.
///
/// This is what a fetched message goes through before anything renders it:
/// header and address parsing, MIME walking, charset and transfer-encoding
/// decoding, preview flattening, and the threading cue that decides which
/// conversation it joins. `mime::parse` is documented as total, so *any* panic
/// here is the find — there is no input this is allowed to reject.
pub fn check_parse_message(raw: &[u8]) {
    let parsed = mime::parse(raw);
    check_parsed_invariants(&parsed, raw);

    // The initial-sync path, which buffers no bodies. Different code, same
    // hostile bytes: it is the one that runs first and on every message.
    let headers_only = mime::parse_headers(raw);
    check_parsed_invariants(&headers_only, raw);

    // Threading reads sender-supplied `Message-ID` and `References`, and the
    // ids it pulls out of them are used as index keys. A cue is cheap to build
    // and is the last step before the message reaches storage.
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

/// What must hold of any [`ParsedMessage`], however malformed its source.
fn check_parsed_invariants(parsed: &ParsedMessage, raw: &[u8]) {
    assert_eq!(
        parsed.size,
        raw.len() as u64,
        "size is the length of what was parsed, not of what parsed cleanly"
    );

    // The preview goes in a 40px list row. A message that could make it
    // unbounded would let a sender decide how much memory every row costs, and
    // the row is drawn for every message in the mailbox.
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

    // Attachment metadata is deliberately *not* checked for path separators
    // here, and the first run of this target is why. It found
    // `filename="=?utf-8Qa/b.txt"` -- a malformed RFC 2047 word that does not
    // decode, leaving the slash in place -- and that is the parser behaving
    // correctly. `mime::parse` reports the filename the sender wrote; making
    // it safe to write to disk is `postio_gtk::parts::save_name`'s promise,
    // and that is the layer whose tests assert it. Asserting it here would
    // demand that the model launder data it is supposed to report faithfully.
    //
    // The find was still worth having: it is what sent someone to look at
    // `save_name`, which turned out to strip separators and dots but not
    // control characters, and a NUL reaches a filename by the same route.
    let _ = &parsed.parts;
}

/// Sanitize `html` both ways round and check what comes out.
///
/// This is the boundary between attacker-controlled markup and the reading
/// pane's `WebView`. The pane has JavaScript and network access off, so this
/// is defence in depth rather than the only defence — but it is the layer that
/// decides what the user sees, and the count it returns is what the UI shows
/// as "remote blocked".
pub fn check_sanitize_html(html: &str) {
    let blocked = sanitize_body(html, RemoteImages::Blocked);
    check_sanitized(&blocked, RemoteImages::Blocked);

    let allowed = sanitize_body(html, RemoteImages::Allowed);
    check_sanitized(&allowed, RemoteImages::Allowed);

    // Blocking can only ever remove references, never invent them.
    assert_eq!(
        allowed.remote_blocked, 0,
        "nothing is held back when remote images are allowed"
    );

    // Idempotence. The reading pane re-renders on "show once" and on "always
    // allow", so output becomes input again in normal use. A sanitizer whose
    // second pass differs from its first has a construct it does not have a
    // fixed point for -- which is the shape of a filter bypass, because the
    // markup the user sees is whichever pass ran last.
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

/// What must hold of sanitized markup.
fn check_sanitized(sanitized: &Sanitized, remote: RemoteImages) {
    let html = &sanitized.html;

    // The headline promise. `ammonia` is what enforces it; this is the
    // assertion that says so out loud, and that would notice the day an
    // `attribute_filter` change let something back through.
    //
    // Tag names only. A scheme name is deliberately *not* checked here, and
    // the first run of this target is why: ammonia decodes character
    // references in text, so `&#x6a;avascript&colon;` inside a `<p>` comes out
    // as the literal text `javascript:` -- visible, inert, and not a URL. A
    // document-wide substring scan called that a sanitizer bypass. A tag name
    // has no such ambiguity: `<` is escaped to `&lt;` everywhere in text, so a
    // literal `<script` in the output can only be an element.
    for banned in ["<script", "<iframe", "<object", "<embed"] {
        assert!(
            !html.to_ascii_lowercase().contains(banned),
            "sanitized output still contains {banned:?}"
        );
    }

    // The rule applies to attributes the renderer *fetches on its own*, and
    // only to those. An `<a href>` to a remote page is not a leak: nothing
    // loads until the user clicks it, and the click is handled by the
    // reader's `decide-policy` handler rather than by the pane. Asserting on
    // href here failed on the newsletter fixture, which is a real message
    // behaving correctly -- the corpus test caught exactly that.
    for (attribute, value) in url_attribute_values(html) {
        if !FETCHED_ON_SIGHT.contains(&attribute) {
            continue;
        }
        let lowered = value.trim().to_ascii_lowercase();
        // Where a scheme name actually means something. In an attribute the
        // renderer fetches, this would execute; in text it is just text.
        assert!(
            !lowered.starts_with("javascript:"),
            "a javascript: URL survived in {attribute}: {value:?}"
        );
        // `cid:` is rewritten to CID_SCHEME on the way through -- see
        // `sanitize::rewrite_attribute` -- so a surviving bare `cid:` in a
        // fetched attribute means the rewrite was skipped.
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

/// Attribute names whose value the renderer fetches without being asked.
///
/// These are the ones remote-image blocking is about: a `src` loads when the
/// document does, which is what makes a tracking pixel work. `href` is
/// deliberately absent -- see [`check_sanitized`].
const FETCHED_ON_SIGHT: &[&str] = &["src", "srcset", "poster", "background"];

/// Parse `input` as a search query and check what comes back.
///
/// The grammar is documented as total — every string is a query, because
/// results update on every keystroke and half-typed input is the normal case.
/// So a panic is a find, and so is a span that does not describe the input it
/// claims to: `ParsedQuery::remove_token` slices the query on those offsets,
/// and a span landing inside a multi-byte character panics there rather than
/// here.
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
             ParsedQuery::remove_token slices on these offsets and would panic"
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

        // The round trip the search bar performs when a chip is dismissed.
        // Whatever comes back has to be a query too -- it is handed straight
        // back to the parser on the next keystroke.
        let without = parsed.remove_token(index);
        let _ = postio_search::parse(&without, fixed_today());
    }
}

/// Every attribute value in `html` that names a URL.
///
/// A deliberately crude scan rather than a parse: the point is to look at the
/// bytes that will reach WebKit, not to agree with the parser about what they
/// mean. Something this misses is a gap in the check; something it invents is
/// at worst a false positive that a person reads.
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

/// The source text inside each `<...>`, which is the only place an attribute
/// can be.
///
/// Scanning the whole document instead reports text as markup, and this target
/// found that twice in its first six minutes: an input whose `<` had been
/// mutated away left `img src="https://..."` sitting in a paragraph as visible,
/// inert text, and a document-wide scan called it a surviving remote
/// reference.
///
/// Splitting on `<` is sound *because* of what the sanitizer guarantees:
/// ammonia escapes `<` to `&lt;` and `>` to `&gt;` everywhere in text and in
/// attribute values, so in sanitized output an unescaped `<` starts a tag and
/// the next `>` ends it. This function is therefore only correct on output
/// that has already been through `sanitize_body` -- which is the only thing
/// it is ever called on.
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
            // An unterminated tag: nothing after it can be an attribute
            // either, so stop rather than treating the remainder as one.
            None => break,
        }
    }
    interiors
}

/// Whether a URL value reaches off this machine. Mirrors
/// `postio_body::sanitize::is_remote`, which is private.
fn is_remote(lowered: &str) -> bool {
    lowered.starts_with("http://")
        || lowered.starts_with("https://")
        || lowered.starts_with("//")
        || lowered.starts_with("ftp://")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every check must survive the whole `.eml` corpus. If one of these
    /// asserts something Postio does not actually promise, this is where it
    /// shows up — before a scheduled job reports it as a security find at
    /// three in the morning.
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

    /// Degenerate inputs, which is where a total function stops being total.
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

    // ── the checks can fail ──────────────────────────────────────────────
    //
    // A property checker that cannot fail is worse than none: it reports
    // "clean" forever and the scheduled job that runs it is theatre. Each of
    // these hands the checker something that violates exactly one property and
    // asserts it notices. They are the reason to believe the three above.

    #[test]
    #[should_panic(expected = "still contains")]
    fn the_sanitizer_check_notices_a_surviving_script() {
        check_sanitized(
            &Sanitized {
                html: "<p>hello</p><script>alert(1)</script>".to_string(),
                remote_blocked: 0,
            },
            RemoteImages::Blocked,
        );
    }

    #[test]
    #[should_panic(expected = "a remote reference survived")]
    fn the_sanitizer_check_notices_a_surviving_remote_image() {
        check_sanitized(
            &Sanitized {
                html: "<img src=\"https://tracker.example.com/pixel.gif\">".to_string(),
                remote_blocked: 0,
            },
            RemoteImages::Blocked,
        );
    }

    #[test]
    #[should_panic(expected = "bare cid:")]
    fn the_sanitizer_check_notices_an_unrewritten_cid() {
        check_sanitized(
            &Sanitized {
                html: "<img src=\"cid:part1@example.com\">".to_string(),
                remote_blocked: 0,
            },
            RemoteImages::Blocked,
        );
    }

    /// The same value under `RemoteImages::Allowed` is not a find — that is
    /// the user having asked for it — so the check has to tell the two apart.
    #[test]
    fn a_remote_reference_is_fine_once_the_user_allowed_it() {
        check_sanitized(
            &Sanitized {
                html: "<img src=\"https://cdn.example.com/logo.png\">".to_string(),
                remote_blocked: 0,
            },
            RemoteImages::Allowed,
        );
    }

    #[test]
    fn the_url_scan_finds_what_it_is_looking_for() {
        let found = url_attribute_values(
            "<a href=\"https://example.com/a\"><img src=\"cid:x\" srcset=\"//e.example.net/b\">",
        );
        // Tag by tag, then attribute within the tag -- so the anchor's href
        // comes before the img's src, which is source order rather than the
        // order the attribute names are listed in.
        assert_eq!(
            found,
            [
                ("href", "https://example.com/a".to_string()),
                ("src", "cid:x".to_string()),
                ("srcset", "//e.example.net/b".to_string()),
            ]
        );

        // An unterminated attribute must end the scan rather than loop or
        // slice past the end.
        assert!(url_attribute_values("<img src=\"unterminated").is_empty());
        // And an attribute that is only text is not an attribute.
        assert!(url_attribute_values("plain src=\"https://a.example.com\"").is_empty());
    }

    /// The second find this target produced. The mutation removed the `<`
    /// before `img`, so `img src="https://..."` was escaped *text* in a
    /// paragraph -- visible, inert, and fetched by nothing. A document-wide
    /// scan called it a surviving remote reference.
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
        // Unterminated: stop, rather than read the rest of the document as an
        // attribute list.
        assert_eq!(tag_interiors("<img src=\"x\""), Vec::<&str>::new());
        // The shape the fuzzer found: text that looks like a tag but has no
        // opening angle bracket.
        assert_eq!(
            tag_interiors("hello img src=\"https://a.example.com\""),
            Vec::<&str>::new()
        );
    }

    /// The first find this target produced, kept as a test so the distinction
    /// cannot be un-learned. `&#x6a;avascript&colon;` decodes to the literal
    /// text `javascript:` inside a `<p>` -- inert, and not a URL. A
    /// document-wide substring scan reported it as a sanitizer bypass.
    #[test]
    fn a_scheme_name_in_text_is_not_a_bypass() {
        check_sanitize_html(
            "<p>&lt;script&gt;&#x6a;avascript&colon;alert(1)</p>\
             <img src=\"&#104;ttps://tracker.example.com/x.gif\">",
        );
    }

    /// The same string where it *would* execute is still a find.
    #[test]
    #[should_panic(expected = "a javascript: URL survived")]
    fn a_scheme_name_in_a_fetched_attribute_is_a_bypass() {
        check_sanitized(
            &Sanitized {
                html: "<img src=\"javascript:alert(1)\">".to_string(),
                remote_blocked: 0,
            },
            RemoteImages::Blocked,
        );
    }

    /// The reassuring half of the same find: an entity-encoded scheme in an
    /// attribute is decoded before the filter sees it, so blocking is not
    /// dodged by spelling `https` as `&#104;ttps`.
    #[test]
    fn an_entity_encoded_remote_scheme_is_still_blocked() {
        let out = postio_body::sanitize::sanitize_body(
            "<img src=\"&#104;ttps://tracker.example.com/x.gif\">",
            RemoteImages::Blocked,
        );
        assert_eq!(out.remote_blocked, 1, "{}", out.html);
        assert!(!out.html.contains("tracker.example.com"), "{}", out.html);
    }

    /// The distinction the corpus taught us: a remote *link* is not a remote
    /// *load*, and a check that conflates them fails on real mail.
    #[test]
    fn a_remote_link_is_not_a_remote_load() {
        check_sanitized(
            &Sanitized {
                html: "<a href=\"https://news.example.org/issues/214\">read on</a>".to_string(),
                remote_blocked: 0,
            },
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
