use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

impl Header {
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.push(Header::new(name, value));
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }

    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
            .collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Header> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

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

/// Longest normalized value stored or matched; the index holds a row per header occurrence (ADR
/// 0025). A correctness hazard, not a cost knob: the index holds the prefix, so both sides must
/// pass `normalize_value` or disagree on longer headers.
pub const VALUE_LIMIT: usize = 512;

/// Lowercased, because the name is matched exactly (`header:x-mail` must not find `X-Mailer`) and
/// needs one spelling on both sides.
pub fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// Both sides of a `header:` match must run this. In order: unfold and decode RFC 2047 words
/// (`decode_header_text`, which catches parser panics because senders choose the bytes, #277);
/// collapse whitespace; truncate to `VALUE_LIMIT` on a char boundary, since a byte slice would
/// panic inside the indexing pass.
pub fn normalize_value(value: &str) -> String {
    let decoded = crate::mime::decode_header_text(value.as_bytes());
    let mut collapsed = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() > VALUE_LIMIT {
        // Walk back to a char boundary: a character can be four bytes.
        let mut end = VALUE_LIMIT;
        while end > 0 && !collapsed.is_char_boundary(end) {
            end -= 1;
        }
        collapsed.truncate(end);
    }
    collapsed
}

impl Headers {
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
        // RFC 5322 §2.2.3: the fold is not part of the value, so a folded `Received` chain must
        // match what a person typed.
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
        assert_eq!(
            normalize_value("=?utf-8?q?caf=C3=A9?="),
            "café",
            "an encoded word has to be readable before it can be searched"
        );
    }

    #[test]
    fn runs_of_whitespace_collapse_to_one_space() {
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
        // `é` is two bytes, so a prefix of `VALUE_LIMIT` bytes lands mid-character.
        let long = "é".repeat(VALUE_LIMIT);
        let value = normalize_value(&long);
        assert!(value.len() <= VALUE_LIMIT);
        assert!(
            value.len() > VALUE_LIMIT - 4,
            "it should cut as close to the limit as a character boundary allows"
        );
        assert!(value.chars().all(|c| c == 'é'));
    }

    #[test]
    fn duplicate_fields_survive_normalization_in_wire_order() {
        // A normalization that deduplicated by name would take the `Received` hop chain with it.
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

/// The pathological case only. ADR 0025 stores the block whole so the indexing policy can change by
/// local reindex;
/// this bound keeps one message from deciding the store's size.
pub const BLOCK_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub text: String,
    /// Carried to the row because a truncated block cannot answer "has no such header", only "the
    /// kept part has none".
    pub truncated: bool,
}

/// Everything before the first empty line (RFC 5322 §2.1), CRLF or bare LF, since mail through a
/// Unix mailbox loses its carriage returns. `None` when there is no separator: bytes that are not a
/// message must not become index content.
pub fn block_of(raw: &[u8]) -> Option<Block> {
    let block = match find_separator(raw) {
        Some(end) => &raw[..end],
        // No separator: either the whole message is headers (a real shape) or these are not
        // headers;
        // `looks_like_a_field` tells them apart.
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

fn find_separator(raw: &[u8]) -> Option<usize> {
    if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some(at);
    }
    raw.windows(2).position(|w| w == b"\n\n")
}

/// Deliberately weak: it only separates an empty-body message from not-a-message;
/// `mime::parse_headers` validates.
fn looks_like_a_field(raw: &[u8]) -> bool {
    let first = raw.split(|byte| *byte == b'\n').next().unwrap_or(raw);
    match first.iter().position(|byte| *byte == b':') {
        Some(colon) => colon > 0 && !first[..colon].contains(&b' '),
        None => false,
    }
}

fn trim_line_break(raw: &[u8]) -> &[u8] {
    let raw = raw.strip_suffix(b"\n").unwrap_or(raw);
    raw.strip_suffix(b"\r").unwrap_or(raw)
}

/// Cut on a line break, not at the byte: half a field looks whole to every reader, which is worse
/// than absence.
/// It also never splits a UTF-8 character.
fn cut_to_limit(block: &[u8]) -> Option<&[u8]> {
    if block.len() <= BLOCK_LIMIT {
        return None;
    }
    let boundary = block[..=BLOCK_LIMIT]
        .iter()
        .rposition(|byte| *byte == b'\n')?;
    Some(trim_line_break(&block[..boundary]))
}

/// The other half of `block_of`. The block is stored without its terminating blank line, and a
/// parser handed an unterminated block drops the last field, so this appends it. Uses
/// `parse_headers`, the hardened entry point (#277).
pub fn parse_block(block: &str) -> Headers {
    let mut terminated = block.to_owned().into_bytes();
    terminated.extend_from_slice(b"\r\n\r\n");
    crate::mime::parse_headers(&terminated).headers
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
        let unix = b"From: ada@example.com\nSubject: hello\n\nthe body\n";
        let block = block_of(unix).expect("bare LF is still a separator");
        assert!(block.text.contains("Subject: hello"));
        assert!(!block.text.contains("the body"));
    }

    #[test]
    fn bytes_with_no_separator_have_no_block_rather_than_a_made_up_one() {
        assert_eq!(block_of(b"this is not a message at all"), None);
    }

    #[test]
    fn a_block_past_the_limit_is_cut_and_says_so() {
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
    fn a_stored_block_parses_back_to_the_fields_it_was_made_of() {
        let block = block_of(MESSAGE).expect("a block");
        let headers = parse_block(&block.text);

        assert_eq!(headers.get("From"), Some("ada@example.com"));
        assert_eq!(
            headers.get("X-Mailer"),
            Some("mutt 1.5.24"),
            "the last field in a stored block is a field, not a fold"
        );
    }

    #[test]
    fn every_fixture_in_the_corpus_round_trips_through_a_stored_block() {
        for fixture in crate::test_corpus::all() {
            let block = block_of(fixture.bytes())
                .unwrap_or_else(|| panic!("{} has no header block", fixture.name()));
            let parsed = parse_block(&block.text);
            let direct = fixture.parse().headers;
            assert_eq!(
                parsed.len(),
                direct.len(),
                "{} lost or gained a field on the way through storage",
                fixture.name()
            );
        }
    }

    #[test]
    fn every_fixture_in_the_corpus_yields_a_block() {
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
