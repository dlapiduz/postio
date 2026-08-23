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

#[cfg(test)]
mod tests {
    use super::*;

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
}
