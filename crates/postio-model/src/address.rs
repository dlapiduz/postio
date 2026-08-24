//! Mail participants as they appear in address headers.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A single RFC 5322 address: an optional display name plus an addr-spec.
///
/// The `address` is stored exactly as it was received. Comparisons that should
/// ignore case go through [`EmailAddress::normalized`] or
/// [`EmailAddress::same_address`]; derived `PartialEq` is exact, because two
/// headers that differ only in display name are genuinely different headers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct EmailAddress {
    /// The display name, if the header carried one.
    pub name: Option<String>,
    /// The addr-spec, e.g. `alice@example.com`, verbatim.
    pub address: String,
}

impl EmailAddress {
    /// Builds an address from an optional display name and an addr-spec.
    pub fn new(name: Option<impl Into<String>>, address: impl Into<String>) -> Self {
        Self {
            name: name.map(Into::into),
            address: address.into(),
        }
    }

    /// The lowercased addr-spec, for indexing and equality.
    pub fn normalized(&self) -> String {
        self.address.to_lowercase()
    }

    /// Whether both values name the same mailbox, ignoring display name and case.
    pub fn same_address(&self, other: &Self) -> bool {
        self.address.eq_ignore_ascii_case(&other.address)
    }

    /// The part before the last `@`.
    pub fn local_part(&self) -> Option<&str> {
        self.address.rsplit_once('@').map(|(local, _)| local)
    }

    /// The part after the last `@`.
    pub fn domain(&self) -> Option<&str> {
        self.address.rsplit_once('@').map(|(_, domain)| domain)
    }

    /// What to show in the UI: the display name if there is one, else the address.
    pub fn display(&self) -> &str {
        match self.name.as_deref() {
            Some(name) if !name.trim().is_empty() => name,
            _ => &self.address,
        }
    }

    /// Whether the addr-spec could plausibly be delivered to.
    ///
    /// A shape check, not a validation: exactly one `@`, something either
    /// side of it, a dot in the domain, and no whitespace. It exists so the
    /// composer can *mark* a recipient that will not work while still holding
    /// on to it — the alternative, refusing to accept the text, loses what the
    /// user typed and tells them nothing about why.
    ///
    /// Deliberately not RFC 5322: a quoted local part and an address literal
    /// are both legal and both vanishingly rare next to a typo, and the only
    /// authority on whether an address exists is the receiving server.
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

/// Parses a header-style address list as a person would type it into a
/// composer field.
///
/// Deliberately forgiving. This runs on every keystroke of a `To` field, where
/// the text is *mid-edit* far more often than it is finished, so a strict RFC
/// 5322 parser would spend most of its life rejecting input that is merely
/// unfinished. What it does guarantee:
///
/// * `,` and `;` separate addresses, unless they are inside a quoted display
///   name or inside the angle brackets of an addr-spec.
/// * `Ada Lovelace <ada@example.com>` splits into name and address; a bare
///   `ada@example.com` is all address.
/// * A quoted display name comes back unquoted, so what the field shows is
///   what the header will carry.
/// * Whitespace-only entries vanish rather than becoming empty addresses.
///
/// Whether an entry is *deliverable* is a different question, answered by
/// [`EmailAddress::is_plausible`] — the composer wants to show a half-typed
/// address as a recipient and merely mark it, not drop it.
///
/// ```
/// use postio_model::address::parse_list;
///
/// let parsed = parse_list("Ada Lovelace <ada@example.com>, grace@example.net");
/// assert_eq!(parsed.len(), 2);
/// assert_eq!(parsed[0].name.as_deref(), Some("Ada Lovelace"));
/// assert_eq!(parsed[1].address, "grace@example.net");
/// ```
pub fn parse_list(input: &str) -> Vec<EmailAddress> {
    split_list(input)
        .into_iter()
        .filter_map(|entry| parse_one(entry.trim()))
        .collect()
}

/// Renders addresses back into what a composer field should show.
///
/// The inverse of [`parse_list`] for anything [`parse_list`] produced: the
/// separator is `, ` and each address is its [`Display`](fmt::Display) form,
/// so a draft that is saved and reopened puts the same text back in the field.
pub fn format_list(addresses: &[EmailAddress]) -> String {
    addresses
        .iter()
        .map(EmailAddress::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The last address in `input`, still being typed — what recipient
/// completion should search contacts for and, if the user picks one, replace.
///
/// Returns `(start, text)`: `text` is the token to search for, and `start` is
/// its byte offset in `input`, so a caller can replace exactly
/// `input[start..]` with whatever was chosen and leave every address typed
/// before it untouched. Splits on the same rules as [`parse_list`] — a comma
/// or semicolon inside a quoted name or an angle-bracketed address is not a
/// separator — and the whitespace after a separator is not part of the token:
/// completing `"Ada <ada@example.com>, gr"` searches for `"gr"` starting where
/// the letter is, not at the space right after the comma.
///
/// ```
/// use postio_model::address::current_entry;
/// assert_eq!(current_entry("grace"), (0, "grace"));
/// assert_eq!(current_entry("ada@example.com, gr"), (17, "gr"));
/// assert_eq!(current_entry("ada@example.com, "), (17, ""));
/// ```
pub fn current_entry(input: &str) -> (usize, &str) {
    let last = split_list(input).pop().unwrap_or(input);
    // `last` is a subslice of `input` by construction, so this offset is
    // sound and is the only way to recover it without `split_list` keeping
    // more than the borrowed text itself.
    let start = last.as_ptr() as usize - input.as_ptr() as usize;
    let trimmed = last.trim_start();
    (start + (last.len() - trimmed.len()), trimmed)
}

/// Splits on the separators that are not inside quotes or angle brackets.
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

/// One entry, already trimmed. `None` for an entry that held nothing.
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
        // `Ada <ada@example.com` — the bracket the user has not typed yet.
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

/// Strips the quotes around a display name, and the backslashes inside them.
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

/// Replace the local part of every address in `text` with `…`.
///
/// For **logs**, and only for logs. An error carrying an address is right on
/// screen — it is how the user knows which account to fix — and wrong in a
/// file people paste into bug reports. The domain survives because it is what
/// makes the line diagnostic at all: telling an iCloud failure from a Fastmail
/// one is most of the triage, and it identifies nobody.
///
/// Deliberately crude, and deliberately over-matching. It finds every `@` and
/// walks left over the characters an address can contain; a log line is not
/// the place to be precise about RFC 5322 at the cost of missing one.
///
/// ```
/// use postio_model::address::redact_addresses;
///
/// assert_eq!(
///     redact_addresses("no password is stored for ada@example.com; add one"),
///     "no password is stored for \u{2026}@example.com; add one"
/// );
/// ```
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
        // Walk back over the local part already written out and drop it.
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

/// The characters an address's local part is allowed to use, per RFC 5322's
/// `atext` plus `.` — a superset is the safe direction here.
fn is_local_part_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~.".contains(ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_keeps_the_domain_and_drops_who() {
        // The domain is what makes a log line diagnostic; the local part is
        // what identifies a person.
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
        // A dotted local part, a plus tag, and an address flush against
        // punctuation all have to lose their local part.
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
        // Every one of these is a state a To field passes through while it is
        // being typed, and none of them may lose a character of it.
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
