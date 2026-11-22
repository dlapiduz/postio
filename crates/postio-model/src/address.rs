use std::fmt;

use serde::{Deserialize, Serialize};

/// `address` is stored as received; case-insensitive comparison goes through `normalized` or
/// `same_address`. Derived `PartialEq` is exact.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub address: String,
}

impl EmailAddress {
    pub fn new(name: Option<impl Into<String>>, address: impl Into<String>) -> Self {
        Self {
            name: name.map(Into::into),
            address: address.into(),
        }
    }

    pub fn normalized(&self) -> String {
        self.address.to_lowercase()
    }

    pub fn same_address(&self, other: &Self) -> bool {
        self.address.eq_ignore_ascii_case(&other.address)
    }

    pub fn local_part(&self) -> Option<&str> {
        self.address.rsplit_once('@').map(|(local, _)| local)
    }

    pub fn domain(&self) -> Option<&str> {
        self.address.rsplit_once('@').map(|(_, domain)| domain)
    }

    pub fn display(&self) -> &str {
        match self.name.as_deref() {
            Some(name) if !name.trim().is_empty() => name,
            _ => &self.address,
        }
    }

    /// A shape check, not validation, so the composer can mark a bad recipient while keeping the
    /// text.
    /// Not RFC 5322: quoted local parts and address literals are rarer than typos.
    pub fn is_plausible(&self) -> bool {
        let address = self.address.trim();
        if address != self.address || address.contains(char::is_whitespace) {
            return false;
        }
        let Some((local, domain)) = address.split_once('@') else {
            return false;
        };
        !local.is_empty()
            && !domain.contains('@')
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
    }
}

/// Deliberately forgiving: this runs on every keystroke of a `To` field, where text is mid-edit far
/// more often than finished. `,` and `;` separate
/// addresses unless inside a quoted name or angle brackets; a quoted display name comes back
/// unquoted; whitespace-only entries vanish.
/// Deliverability is `EmailAddress::is_plausible`'s question.
pub fn parse_list(input: &str) -> Vec<EmailAddress> {
    split_list(input)
        .into_iter()
        .filter_map(|entry| parse_one(entry.trim()))
        .collect()
}

/// Inverse of `parse_list`: `, `-separated, so a saved draft reopens with the same field text.
pub fn format_list(addresses: &[EmailAddress]) -> String {
    addresses
        .iter()
        .map(EmailAddress::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The last address in `input`, still being typed: `(start, text)`, where `start` is its byte
/// offset so a caller can replace `input[start..]`.
/// Splits as `parse_list` does; whitespace after a separator is not part of the token.
pub fn current_entry(input: &str) -> (usize, &str) {
    let last = split_list(input).pop().unwrap_or(input);
    // `last` is a subslice of `input` by construction, so pointer subtraction gives its offset.
    let start = last.as_ptr() as usize - input.as_ptr() as usize;
    let trimmed = last.trim_start();
    (start + (last.len() - trimmed.len()), trimmed)
}

fn split_list(input: &str) -> Vec<&str> {
    let mut entries = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut angled = false;
    let mut escaped = false;

    for (offset, character) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '<' if !quoted => angled = true,
            '>' if !quoted => angled = false,
            ',' | ';' if !quoted && !angled => {
                entries.push(&input[start..offset]);
                start = offset + character.len_utf8();
            }
            _ => {}
        }
    }
    entries.push(&input[start..]);
    entries
}

fn parse_one(entry: &str) -> Option<EmailAddress> {
    if entry.is_empty() {
        return None;
    }

    let Some(open) = entry.rfind('<') else {
        return Some(EmailAddress::new(None::<String>, entry));
    };
    let close = entry[open..].find('>').map(|at| open + at);

    let address = match close {
        Some(close) => &entry[open + 1..close],
        None => &entry[open + 1..],
    };
    let address = address.trim();
    let name = unquote(entry[..open].trim());

    if address.is_empty() && name.is_empty() {
        return None;
    }
    Some(EmailAddress::new(
        (!name.is_empty()).then_some(name),
        address,
    ))
}

fn unquote(name: &str) -> String {
    let Some(inner) = name
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return name.to_owned();
    };
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for character in inner.chars() {
        match character {
            '\\' if !escaped => escaped = true,
            _ => {
                escaped = false;
                out.push(character);
            }
        }
    }
    out
}

impl fmt::Display for EmailAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name.as_deref() {
            Some(name) if !name.trim().is_empty() => write!(f, "{name} <{}>", self.address),
            _ => f.write_str(&self.address),
        }
    }
}

/// For logs only: an error carrying an address is right on screen and wrong in a file pasted into
/// bug reports. The domain survives because it
/// makes the line diagnostic. Deliberately crude and over-matching: finds every `@` and walks left
/// over characters an address can contain.
pub fn redact_addresses(text: &str) -> String {
    if !text.contains('@') {
        return text.to_owned();
    }
    let bytes: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != '@' {
            out.push(bytes[index]);
            index += 1;
            continue;
        }
        let mut local = 0;
        for ch in out.chars().rev() {
            if is_local_part_char(ch) {
                local += ch.len_utf8();
            } else {
                break;
            }
        }
        if local > 0 {
            out.truncate(out.len() - local);
            out.push('\u{2026}');
        }
        out.push('@');
        index += 1;
    }
    out
}

/// RFC 5322 `atext` plus `.`; a superset is the safe direction for redaction.
fn is_local_part_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~.".contains(ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_keeps_the_domain_and_drops_who() {
        assert_eq!(
            redact_addresses("no password is stored for ada@example.com; add one"),
            "no password is stored for \u{2026}@example.com; add one"
        );
    }

    #[test]
    fn redaction_handles_several_addresses_and_leaves_prose_alone() {
        assert_eq!(
            redact_addresses("ada@example.com and grace@example.net disagreed"),
            "\u{2026}@example.com and \u{2026}@example.net disagreed"
        );
        assert_eq!(
            redact_addresses("nothing to redact here"),
            "nothing to redact here"
        );
    }

    #[test]
    fn redaction_over_matches_rather_than_missing_one() {
        assert_eq!(
            redact_addresses("<ada.b+postio@example.com>"),
            "<\u{2026}@example.com>"
        );
    }

    #[test]
    fn a_list_splits_on_the_separators_that_are_not_inside_something() {
        let parsed = parse_list(
            "Ada Lovelace <ada@example.com>, grace@example.net; \"Hopper, Grace\" <gh@example.org>",
        );
        assert_eq!(
            parsed,
            vec![
                EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
                EmailAddress::new(None::<String>, "grace@example.net"),
                EmailAddress::new(Some("Hopper, Grace"), "gh@example.org"),
            ]
        );
    }

    #[test]
    fn a_half_typed_field_still_parses_into_what_is_there() {
        assert_eq!(parse_list("").len(), 0);
        assert_eq!(parse_list("   ,  ; ").len(), 0);
        assert_eq!(
            parse_list("ada@"),
            vec![EmailAddress::new(None::<String>, "ada@")]
        );
        assert_eq!(
            parse_list("Ada <ada@example.com"),
            vec![EmailAddress::new(Some("Ada"), "ada@example.com")]
        );
        assert_eq!(
            parse_list("ada@example.com,"),
            vec![EmailAddress::new(None::<String>, "ada@example.com")]
        );
    }

    #[test]
    fn a_parsed_list_formats_back_to_the_text_it_came_from() {
        let text = "Ada Lovelace <ada@example.com>, grace@example.net";
        assert_eq!(format_list(&parse_list(text)), text);
    }

    #[test]
    fn plausibility_is_a_shape_check_over_the_addr_spec() {
        for good in ["ada@example.com", "a.b+tag@mail.example.com"] {
            let address = EmailAddress::new(None::<String>, good);
            assert!(address.is_plausible(), "{good} should be plausible");
        }
        for bad in [
            "",
            "ada",
            "ada@",
            "@example.com",
            "ada@example",
            "a b@example.com",
            "a@b@example.com",
            "ada@example.com ",
        ] {
            let address = EmailAddress::new(None::<String>, bad);
            assert!(!address.is_plausible(), "{bad:?} should not be plausible");
        }
    }

    #[test]
    fn current_entry_is_the_whole_field_with_nothing_typed_yet() {
        assert_eq!(current_entry(""), (0, ""));
        assert_eq!(current_entry("gr"), (0, "gr"));
    }

    #[test]
    fn current_entry_is_only_the_token_after_the_last_separator() {
        assert_eq!(
            current_entry("ada@example.com, gr"),
            (17, "gr"),
            "the finished first address is not part of what is being typed"
        );
        assert_eq!(
            current_entry("ada@example.com,gr"),
            (16, "gr"),
            "no space after the comma is still a separator"
        );
        assert_eq!(
            current_entry("ada@example.com, "),
            (17, ""),
            "right after a separator, nothing has been typed yet"
        );
    }

    #[test]
    fn current_entry_does_not_split_inside_a_quoted_name_or_an_address() {
        assert_eq!(
            current_entry("\"Hopper, Grace\" <g"),
            (0, "\"Hopper, Grace\" <g"),
            "a quoted comma is not a separator"
        );
        assert_eq!(
            current_entry("Ada <ada@example.com>, \"Hopper, Grace\" <g"),
            (23, "\"Hopper, Grace\" <g")
        );
    }

    #[test]
    fn current_entry_replaces_exactly_what_completion_should_and_nothing_before_it() {
        let input = "Ada Lovelace <ada@example.com>, gr";
        let (start, token) = current_entry(input);
        assert_eq!(token, "gr");
        let mut replaced = input.to_owned();
        replaced.replace_range(start.., "Grace Hopper <grace@example.com>");
        assert_eq!(
            replaced,
            "Ada Lovelace <ada@example.com>, Grace Hopper <grace@example.com>"
        );
    }
}
