//! Raw header storage.

use serde::{Deserialize, Serialize};

/// One header field, name and unfolded value, as it appeared on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// The field name, original case preserved.
    pub name: String,
    /// The unfolded field value.
    pub value: String,
}

impl Header {
    /// Builds a header field.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// A message's headers.
///
/// Order and duplicates are preserved — `Received` chains and multiple
/// `References` lines matter — while lookup is case-insensitive per RFC 5322.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Headers(Vec<Header>);

impl Headers {
    /// An empty header block.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a header field, keeping any existing field of the same name.
    pub fn push(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.push(Header::new(name, value));
    }

    /// The first value for `name`, matched case-insensitively.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }

    /// Every value for `name`, in wire order.
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
            .collect()
    }

    /// Whether a field with this name is present.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Iterates the fields in wire order.
    pub fn iter(&self) -> std::slice::Iter<'_, Header> {
        self.0.iter()
    }

    /// The number of header fields, counting duplicates.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no header fields.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<Header> for Headers {
    fn from_iter<I: IntoIterator<Item = Header>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<N: Into<String>, V: Into<String>> FromIterator<(N, V)> for Headers {
    fn from_iter<I: IntoIterator<Item = (N, V)>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .map(|(name, value)| Header::new(name, value))
                .collect(),
        )
    }
}

impl<'a> IntoIterator for &'a Headers {
    type Item = &'a Header;
    type IntoIter = std::slice::Iter<'a, Header>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// The longest normalized header value that is stored or matched against.
///
/// A cap rather than a preference: ADR 0025 indexes one row per header
/// occurrence, and an unbounded value would let a single `Received` chain or a
/// DKIM signature decide the size of the index. 512 bytes holds every header a
/// person searches for with room to spare.
///
/// **It is a correctness hazard, not a cost knob.** The index holds the
/// prefix; an in-memory matcher holds whatever it was handed. The two disagree
/// about any longer header unless both sides pass through
/// [`normalize_value`] — which is the entire reason that function is here, in
/// the crate `postio-index` and `postio-search` both already depend on,
/// instead of once in each evaluator.
pub const VALUE_LIMIT: usize = 512;

/// A header name as it is stored and matched: lowercased.
///
/// RFC 5322 field names are case-insensitive, so the stored form picks one.
/// Lowercase rather than the wire's own case because the name is matched
/// **exactly** — `header:x-mail` must not find `X-Mailer` — and an exact match
/// against a column needs one spelling on both sides.
pub fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// A header value as it is stored and matched.
///
/// Four steps, in this order, and the order is the argument:
///
/// 1. **Unfold and decode encoded words.** RFC 5322 §2.2.3 splits a long field
///    across lines and the fold is not part of the value; RFC 2047 encodes
///    non-ASCII as `=?utf-8?q?...?=`. Both are the wire's business, not the
///    reader's — nobody searches for the base64 of a word. [`decode_header_text`]
///    does both, and is already the hardened path for this: it is
///    `mail_parser` behind a `catch_unwind`, because these bytes are chosen by
///    whoever sent the mail (#277).
/// 2. **Collapse whitespace.** Two spaces after a colon, a tab inside a
///    `Received` chain, the space a fold leaves behind — formatting rather
///    than value, and none of it should decide whether a query matches. This
///    also trims, and it is the belt to the unfold's braces: any `\r` or `\n`
///    that survived step 1 is whitespace and goes here.
/// 3. **Truncate to [`VALUE_LIMIT`]**, on a character boundary. The cap is in
///    bytes and the values are UTF-8, so the obvious slice panics — and it
///    would panic inside the indexing pass, over somebody's mail, which is the
///    least recoverable place available.
///
/// Both sides of a `header:` match must run this. See [`VALUE_LIMIT`].
pub fn normalize_value(value: &str) -> String {
    let decoded = crate::mime::decode_header_text(value.as_bytes());
    let mut collapsed = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() > VALUE_LIMIT {
        // Walk back to a boundary rather than rounding down by a fixed amount:
        // one character can be four bytes, and the cut has to land as close to
        // the limit as the encoding allows.
        let mut end = VALUE_LIMIT;
        while end > 0 && !collapsed.is_char_boundary(end) {
            end -= 1;
        }
        collapsed.truncate(end);
    }
    collapsed
}

impl Headers {
    /// Every field, normalized, in wire order.
    pub fn normalized(&self) -> Vec<Header> {
        self.0
            .iter()
            .map(|header| Header::new(normalize_name(&header.name), normalize_value(&header.value)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_matched_in_one_case_whatever_the_wire_used() {
        assert_eq!(normalize_name("X-Mailer"), "x-mailer");
        assert_eq!(normalize_name("MIME-Version"), "mime-version");
        assert_eq!(normalize_name("  Received  "), "received");
    }

    #[test]
    fn a_folded_value_becomes_one_line() {
        // RFC 5322 §2.2.3: a long field is split across lines at a fold, and
        // the fold is not part of the value. A `Received` chain arrives folded
        // almost every time, so a matcher that saw the CRLF would fail to
        // match the string a person typed.
        let folded = "from mail.example.com (mail.example.com [192.0.2.1])\r\n\tby \
                      mx.example.net with ESMTPS id abc123";
        let value = normalize_value(folded);
        assert!(!value.contains('\r'), "got: {value:?}");
        assert!(!value.contains('\n'), "got: {value:?}");
        assert!(value.contains("mail.example.com (mail.example.com [192.0.2.1])"));
        assert!(
            value.contains("by mx.example.net"),
            "the fold has to close up into a single space, got: {value:?}"
        );
    }

    #[test]
    fn an_encoded_word_is_decoded_before_it_is_matched() {
        // RFC 2047. Somebody searching for a word does not type its base64,
        // and the corpus is full of subjects and display names that arrive
        // this way.
        assert_eq!(
            normalize_value("=?utf-8?q?caf=C3=A9?="),
            "café",
            "an encoded word has to be readable before it can be searched"
        );
    }

    #[test]
    fn runs_of_whitespace_collapse_to_one_space() {
        // Two spaces after a colon, a tab in a `Received` chain, the space a
        // fold leaves behind: all of them are the wire's formatting rather
        // than the value, and none of them should decide whether a query
        // matches.
        assert_eq!(normalize_value("mutt   1.5.24"), "mutt 1.5.24");
        assert_eq!(normalize_value("mutt\t1.5.24"), "mutt 1.5.24");
        assert_eq!(normalize_value("  mutt 1.5.24  "), "mutt 1.5.24");
    }

    #[test]
    fn a_value_past_the_limit_is_cut_to_it() {
        let long = "a".repeat(VALUE_LIMIT * 2);
        let value = normalize_value(&long);
        assert_eq!(value.len(), VALUE_LIMIT);
    }

    #[test]
    fn truncation_never_cuts_through_a_character() {
        // The cap is in bytes and the values are UTF-8, so a naive slice
        // panics -- and it would do so on somebody's mail, in the indexing
        // pass, where it is least recoverable. `é` is two bytes, so a prefix
        // of `VALUE_LIMIT` bytes lands mid-character by construction.
        let long = "é".repeat(VALUE_LIMIT);
        let value = normalize_value(&long);
        assert!(value.len() <= VALUE_LIMIT);
        assert!(
            value.len() > VALUE_LIMIT - 4,
            "it should cut as close to the limit as a character boundary allows"
        );
        // The real assertion: this is a `String`, so it built one without
        // slicing through a character.
        assert!(value.chars().all(|c| c == 'é'));
    }

    #[test]
    fn duplicate_fields_survive_normalization_in_wire_order() {
        // `Received` chains are the reason `Headers` keeps duplicates at all,
        // and ADR 0025 indexes one row per occurrence. A normalization that
        // deduplicated by name would take the hop chain with it.
        let headers: Headers = [
            ("Received", "from a.example.com"),
            ("X-Mailer", "mutt"),
            ("Received", "from b.example.com"),
        ]
        .into_iter()
        .collect();

        let normalized = headers.normalized();

        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[0].name, "received");
        assert_eq!(normalized[0].value, "from a.example.com");
        assert_eq!(normalized[2].name, "received");
        assert_eq!(
            normalized[2].value, "from b.example.com",
            "the second occurrence is a different hop, not a repeat"
        );
    }

    #[test]
    fn every_header_in_the_corpus_normalizes_within_the_limit() {
        // The corpus is the closest thing here to real mail, and this pass is
        // about to run over every message in somebody's store. What it must
        // not do is panic or hand the index a value it cannot hold.
        let mut seen = 0;
        for fixture in crate::test_corpus::all() {
            for header in fixture.parse().headers.iter() {
                let value = normalize_value(&header.value);
                assert!(
                    value.len() <= VALUE_LIMIT,
                    "{} exceeded the limit in {}",
                    header.name,
                    fixture.name()
                );
                assert!(
                    !value.contains('\n'),
                    "a fold survived in {}",
                    fixture.name()
                );
                assert!(
                    !value.contains('\r'),
                    "a fold survived in {}",
                    fixture.name()
                );
                assert_eq!(normalize_name(&header.name), header.name.to_lowercase());
                seen += 1;
            }
        }
        assert!(
            seen > 0,
            "the corpus produced no headers to normalize at all"
        );
    }
}

/// The longest header block that is stored, in bytes.
///
/// The pathological case only — a mailing-list message that has been through
/// twenty hops with a DKIM signature at each one, or a spam report with a
/// kilobyte of scoring. ADR 0025 stores the block **whole** precisely so the
/// indexing policy stays revisable by a local reindex rather than a
/// re-download, and a bound this generous keeps that true for every message
/// anyone will actually meet while refusing to let one message decide the size
/// of the store.
pub const BLOCK_LIMIT: usize = 256 * 1024;

/// A message's header block as it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The block text, headers only, no trailing blank line.
    pub text: String,
    /// Whether [`BLOCK_LIMIT`] cut it short.
    ///
    /// Carried out to the row rather than left implicit, because a truncated
    /// block cannot answer "this message has no such header" — only "the part
    /// of it that was kept has none". An evaluator that could not tell the two
    /// apart would report absence with the same confidence either way.
    pub truncated: bool,
}

/// The header block of a raw RFC 5322 message.
///
/// Everything before the first empty line, which is what separates headers
/// from body in RFC 5322 §2.1 — accepting either CRLF or bare LF, because
/// mail that has been through a Unix mailbox has usually lost its carriage
/// returns and refusing it would mean refusing the block on real messages.
///
/// `None` when there is no separator at all: that is not a message with no
/// body, it is bytes that are not a message, and inventing a block from them
/// would put nonsense into the index.
pub fn block_of(raw: &[u8]) -> Option<Block> {
    let block = match find_separator(raw) {
        Some(end) => &raw[..end],
        // No separator. Either the whole message is headers -- which is a real
        // shape, and in the corpus: a nightly-build notice that is a subject
        // and nothing else -- or these are not headers at all. `looks_like_a_field`
        // is what tells the two apart, because inventing a block from arbitrary
        // bytes puts nonsense in the index and the index cannot tell nonsense
        // from a header nobody has heard of.
        None if looks_like_a_field(raw) => trim_line_break(raw),
        None => return Option::None,
    };

    let (block, truncated) = match cut_to_limit(block) {
        Some(cut) => (cut, true),
        None => (block, false),
    };
    Some(Block {
        text: String::from_utf8_lossy(block).into_owned(),
        truncated,
    })
}

/// The offset of the first empty line, or `None` if there is not one.
fn find_separator(raw: &[u8]) -> Option<usize> {
    if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some(at);
    }
    raw.windows(2).position(|w| w == b"\n\n")
}

/// Whether the first line reads as an RFC 5322 field: a name, then a colon.
///
/// Deliberately weak. It is not validating the block — `mime::parse_headers`
/// does that, and this crate's rule is to show what arrived rather than to
/// refuse it. All this has to separate is "a message whose body is empty" from
/// "not a message", and a colon before the first space does that without
/// having an opinion about anybody's header names.
fn looks_like_a_field(raw: &[u8]) -> bool {
    let first = raw.split(|byte| *byte == b'\n').next().unwrap_or(raw);
    match first.iter().position(|byte| *byte == b':') {
        Some(colon) => colon > 0 && !first[..colon].contains(&b' '),
        None => false,
    }
}

/// `raw` without one trailing line break, if it has one.
fn trim_line_break(raw: &[u8]) -> &[u8] {
    let raw = raw.strip_suffix(b"\n").unwrap_or(raw);
    raw.strip_suffix(b"\r").unwrap_or(raw)
}

/// `block` cut to [`BLOCK_LIMIT`] at a field boundary, or `None` if it fits.
///
/// The cut lands on a line break rather than at the byte: what is kept has to
/// parse, and half a field looks exactly like a whole one to every reader
/// downstream — which is worse than the field being absent, because absence is
/// at least honest. Cutting on a line boundary also means never splitting a
/// UTF-8 character, since a line break cannot be inside one.
fn cut_to_limit(block: &[u8]) -> Option<&[u8]> {
    if block.len() <= BLOCK_LIMIT {
        return None;
    }
    let boundary = block[..=BLOCK_LIMIT]
        .iter()
        .rposition(|byte| *byte == b'\n')?;
    Some(trim_line_break(&block[..boundary]))
}

#[cfg(test)]
mod block_tests {
    use super::*;

    const MESSAGE: &[u8] = b"From: ada@example.com\r\n\
                             Subject: hello\r\n\
                             X-Mailer: mutt 1.5.24\r\n\
                             \r\n\
                             the body, which is not a header\r\n";

    #[test]
    fn the_block_is_everything_before_the_blank_line() {
        let block = block_of(MESSAGE).expect("a message has a header block");
        assert!(block.text.contains("X-Mailer: mutt 1.5.24"));
        assert!(
            !block.text.contains("the body"),
            "the body is not a header and must not reach the index: {:?}",
            block.text
        );
        assert!(!block.truncated);
    }

    #[test]
    fn a_message_that_lost_its_carriage_returns_still_has_a_block() {
        // Mail that has been through a Unix mailbox, or a fixture saved by
        // hand. Refusing it would mean refusing the block on real messages.
        let unix = b"From: ada@example.com\nSubject: hello\n\nthe body\n";
        let block = block_of(unix).expect("bare LF is still a separator");
        assert!(block.text.contains("Subject: hello"));
        assert!(!block.text.contains("the body"));
    }

    #[test]
    fn bytes_with_no_separator_have_no_block_rather_than_a_made_up_one() {
        // Not "a message with no body": bytes that are not a message. A block
        // invented from them is nonsense in the index, and the index cannot
        // tell nonsense from a header nobody has heard of.
        assert_eq!(block_of(b"this is not a message at all"), None);
    }

    #[test]
    fn a_block_past_the_limit_is_cut_and_says_so() {
        // Twenty hops, each with its own DKIM signature. The cap exists so one
        // message cannot decide the size of the store.
        let mut raw = Vec::new();
        while raw.len() <= BLOCK_LIMIT {
            raw.extend_from_slice(b"Received: from relay.example.com by mx.example.net\r\n");
        }
        raw.extend_from_slice(b"\r\nthe body\r\n");

        let block = block_of(&raw).expect("a block");

        assert!(
            block.truncated,
            "a block over the cap has to say it was cut"
        );
        assert!(block.text.len() <= BLOCK_LIMIT);
    }

    #[test]
    fn a_cut_block_still_ends_on_a_whole_field() {
        // What is kept has to parse. Cutting mid-field would leave a partial
        // value that looks like a real one to every reader downstream, which
        // is worse than the field being absent.
        let mut raw = Vec::new();
        while raw.len() <= BLOCK_LIMIT {
            raw.extend_from_slice(b"Received: from relay.example.com by mx.example.net\r\n");
        }
        raw.extend_from_slice(b"\r\nthe body\r\n");

        let block = block_of(&raw).expect("a block");

        assert!(
            block.text.ends_with("mx.example.net"),
            "the cut landed inside a field: {:?}",
            &block.text[block.text.len().saturating_sub(60)..]
        );
    }

    #[test]
    fn every_fixture_in_the_corpus_yields_a_block() {
        // The corpus is real mail. A parser that found no header block in one
        // of these would be finding none in somebody's mailbox either.
        for fixture in crate::test_corpus::all() {
            let block = block_of(fixture.bytes())
                .unwrap_or_else(|| panic!("{} has no header block", fixture.name()));
            assert!(!block.truncated, "{} is not pathological", fixture.name());
            assert!(
                !block.text.is_empty(),
                "{} produced an empty block",
                fixture.name()
            );
        }
    }
}
