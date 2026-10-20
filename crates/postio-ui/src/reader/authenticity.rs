//! Whether a message is who it says it is (#1928): the rule, with no toolkit
//! in it.
//!
//! Two independent signals, and the reader draws one quiet line for either
//! and nothing at all for ordinary mail:
//!
//! * [`verdict`] reads the `Authentication-Results` header (RFC 8601) the
//!   *receiving server* added. A sender controls every header it sends, so a
//!   header is trusted only when its authserv-id belongs to the account's
//!   provider (`authserv_ids` in `providers.toml`) and only the topmost such
//!   header counts: the server prepends its own, so anything the sender
//!   wrote sits below it. No trusted header is [`Verdict::Unknown`], which
//!   shows nothing, because no header is also what mail from a server that
//!   does not check looks like.
//! * [`impersonation`] catches a display name that is itself an address or a
//!   domain other than the one the message came from.
//!
//! The headers come from the body once it is downloaded
//! (`Message.headers`), not from the envelope fetch: the warning is only
//! needed with the message open, and asking for it earlier would cost wire
//! bytes on every message in the mailbox.

use postio_model::{EmailAddress, Headers};

/// What the receiving server said about the message's sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// DMARC passed, or SPF or DKIM passed for the From domain.
    Verified,
    /// The server checked and nothing passed, but nothing failed either.
    Unverified,
    /// DMARC failed, or SPF and DKIM both failed for the From domain.
    Failed,
    /// No header from the account's provider: nothing is known.
    Unknown,
}

/// The verdict for a message with these `headers` and `from`, trusting only
/// an `Authentication-Results` whose authserv-id is one of `trusted` (a
/// hostname, matched whole or as a label-bounded suffix).
pub fn verdict<T: AsRef<str>>(headers: &Headers, from: &[EmailAddress], trusted: &[T]) -> Verdict {
    let Some(domain) = from.first().and_then(EmailAddress::domain) else {
        return Verdict::Unknown;
    };
    let domain = domain.to_ascii_lowercase();
    let Some(results) = headers
        .get_all("Authentication-Results")
        .into_iter()
        .filter_map(parse)
        .find(|parsed| trusts(trusted, &parsed.authserv_id))
    else {
        return Verdict::Unknown;
    };

    let of = |method: &str| -> Vec<&Result> {
        results
            .results
            .iter()
            .filter(|r| r.method == method)
            .collect()
    };
    let dmarc = of("dmarc");
    if dmarc.iter().any(|r| r.result == "fail") {
        return Verdict::Failed;
    }
    if dmarc.iter().any(|r| r.result == "pass") {
        return Verdict::Verified;
    }
    // Only a result about the From domain says anything about the From
    // address: a forwarder's own signature failing is not this sender's.
    let about_sender = |r: &&Result, keys: &[&str]| {
        let named: Vec<&str> = keys.iter().filter_map(|key| r.property(key)).collect();
        named.is_empty() || named.iter().any(|value| related(value, &domain))
    };
    let dkim: Vec<&Result> = of("dkim")
        .into_iter()
        .filter(|r| about_sender(r, &["header.d", "header.i"]))
        .collect();
    let spf: Vec<&Result> = of("spf")
        .into_iter()
        .filter(|r| about_sender(r, &["smtp.mailfrom", "smtp.helo"]))
        .collect();
    let passed = |list: &[&Result]| list.iter().any(|r| r.result == "pass");
    let failed = |list: &[&Result]| !passed(list) && list.iter().any(|r| r.result == "fail");
    if passed(&dkim) || passed(&spf) {
        Verdict::Verified
    } else if failed(&dkim) && failed(&spf) {
        Verdict::Failed
    } else {
        Verdict::Unverified
    }
}

/// The address or domain a display name claims, when it is not the sender's.
///
/// `Some("billing@bank.example")` for `"billing@bank.example"
/// <x@other.example>`. A name that merely mentions the sender's own domain,
/// or a subdomain or parent of it, is not a hit.
pub fn impersonation(from: &[EmailAddress]) -> Option<String> {
    let sender = from.first()?;
    let name = sender.name.as_deref()?;
    let actual = sender.address.to_ascii_lowercase();
    let actual_domain = sender.domain()?.to_ascii_lowercase();
    for token in name.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '<' | '>' | '(' | ')' | '[' | ']' | '"' | '\'' | ',' | ';'
            )
    }) {
        let token = token.trim_matches(|c: char| !c.is_alphanumeric());
        if let Some((local, host)) = token.split_once('@') {
            if !local.is_empty() && is_host(host) && !token.eq_ignore_ascii_case(&actual) {
                return Some(token.to_owned());
            }
        } else if is_host(token) && !related(token, &actual_domain) {
            return Some(token.to_owned());
        }
    }
    None
}

/// The one warning line for a message, or `None` when there is nothing to
/// say. A failed verdict and an impersonation both speak, in one line.
pub fn warning<T: AsRef<str>>(
    headers: &Headers,
    from: &[EmailAddress],
    trusted: &[T],
) -> Option<String> {
    let failed = verdict(headers, from, trusted) == Verdict::Failed;
    let claimed = impersonation(from);
    let sender = from.first()?;
    let mut parts = Vec::new();
    if failed {
        parts.push(format!(
            "This message failed sender verification: it may not be from {}.",
            sender.domain().unwrap_or(&sender.address)
        ));
    }
    if let Some(claimed) = claimed {
        parts.push(format!(
            "The name says {claimed}, but it was sent from {}.",
            sender.address
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// One `method=result` from an `Authentication-Results` header, with its
/// `ptype.property=value` pairs.
#[derive(Debug)]
struct Result {
    method: String,
    result: String,
    properties: Vec<(String, String)>,
}

impl Result {
    fn property(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug)]
struct Parsed {
    authserv_id: String,
    results: Vec<Result>,
}

/// RFC 8601 §2.2: `authserv-id [version] ; resinfo ; resinfo ...`.
fn parse(value: &str) -> Option<Parsed> {
    let value = strip_comments(value);
    let mut sections = value.split(';');
    let authserv_id = sections
        .next()?
        .split_whitespace()
        .next()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let results = sections
        .filter_map(|section| {
            let mut words = section.split_whitespace();
            let (method, result) = words.next()?.split_once('=')?;
            // `dkim/1` carries a method version.
            let method = method.split('/').next()?.to_ascii_lowercase();
            let properties = words
                .filter_map(|word| word.split_once('='))
                .map(|(key, value)| {
                    (
                        key.to_ascii_lowercase(),
                        value.trim_matches('"').to_ascii_lowercase(),
                    )
                })
                .collect();
            Some(Result {
                method,
                result: result.to_ascii_lowercase(),
                properties,
            })
        })
        .collect();
    Some(Parsed {
        authserv_id,
        results,
    })
}

/// Drops `( ... )` comments, which nest.
fn strip_comments(value: &str) -> String {
    let mut depth = 0usize;
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Whether `id` is one of `trusted`, whole or as a label-bounded suffix.
fn trusts<T: AsRef<str>>(trusted: &[T], id: &str) -> bool {
    trusted.iter().any(|entry| {
        let entry = entry
            .as_ref()
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        !entry.is_empty() && (id == entry || id.ends_with(&format!(".{entry}")))
    })
}

/// A property value (`bob@example.org`, `@example.org`, `example.org`) and
/// `domain` are the same domain or one is a subdomain of the other.
fn related(value: &str, domain: &str) -> bool {
    let host = value
        .rsplit('@')
        .next()
        .unwrap_or(value)
        .trim_end_matches('.')
        .to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}")) || domain.ends_with(&format!(".{host}"))
}

/// A plausible hostname: dotted labels, a letters-only last one of two or
/// more characters. Deliberately plain: `Dr.`, `J.R.` and `v2.1` are not.
fn is_host(text: &str) -> bool {
    let labels: Vec<&str> = text.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && labels
            .last()
            .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(values: &[&str]) -> Headers {
        let mut headers = Headers::new();
        for value in values {
            headers.push("Authentication-Results", *value);
        }
        headers
    }

    fn ada() -> Vec<EmailAddress> {
        vec![EmailAddress::new(None::<String>, "ada@example.org")]
    }

    fn judge(values: &[&str]) -> Verdict {
        verdict(&headers(values), &ada(), &["mx.example.net"])
    }

    #[test]
    fn dmarc_decides_before_spf_and_dkim() {
        assert_eq!(
            judge(&[
                "mx.example.net; dmarc=fail header.from=example.org; spf=pass smtp.mailfrom=example.org"
            ]),
            Verdict::Failed
        );
        assert_eq!(
            judge(&[
                "mx.example.net; dmarc=pass header.from=example.org; dkim=fail header.d=example.org"
            ]),
            Verdict::Verified
        );
    }

    #[test]
    fn spf_and_dkim_must_both_fail_for_the_from_domain() {
        let both = "mx.example.net; dkim=fail header.d=example.org; spf=fail smtp.mailfrom=ada@example.org";
        assert_eq!(judge(&[both]), Verdict::Failed);
        let one = "mx.example.net; dkim=fail header.d=example.org; spf=pass smtp.mailfrom=ada@example.org";
        assert_eq!(judge(&[one]), Verdict::Verified);
        let only_dkim = "mx.example.net; dkim=fail header.d=example.org";
        assert_eq!(judge(&[only_dkim]), Verdict::Unverified);
    }

    #[test]
    fn a_result_about_another_domain_is_not_about_the_sender() {
        let forwarder = "mx.example.net; dkim=fail header.d=lists.example.com; spf=fail smtp.mailfrom=bounce@lists.example.com";
        assert_eq!(judge(&[forwarder]), Verdict::Unverified);
    }

    #[test]
    fn comments_and_versions_do_not_confuse_the_parse() {
        let value = "mx.example.net 1; spf=pass (domain of ada@example.org (really) designates 192.0.2.1) smtp.mailfrom=ada@example.org";
        assert_eq!(judge(&[value]), Verdict::Verified);
    }

    #[test]
    fn nothing_checked_is_unverified_and_no_trusted_header_is_unknown() {
        assert_eq!(judge(&["mx.example.net; none"]), Verdict::Unverified);
        assert_eq!(judge(&[]), Verdict::Unknown);
        assert_eq!(judge(&["mx.example.com; dmarc=fail"]), Verdict::Unknown);
    }

    #[test]
    fn only_the_topmost_trusted_header_counts() {
        let fail = "mx.example.net; dmarc=fail header.from=example.org";
        let pass = "mx.example.net; dmarc=pass header.from=example.org";
        assert_eq!(judge(&[fail, pass]), Verdict::Failed, "a forged pass below");
        assert_eq!(judge(&[pass, fail]), Verdict::Verified);
        // A header from another server above the provider's is skipped, not
        // believed and not allowed to hide the provider's.
        let other = "relay.example.com; dmarc=pass header.from=example.org";
        assert_eq!(judge(&[other, fail]), Verdict::Failed);
    }

    #[test]
    fn an_authserv_id_matches_whole_or_on_a_label_boundary() {
        let fail = "mx1.example.net; dmarc=fail";
        assert_eq!(
            verdict(&headers(&[fail]), &ada(), &["example.net"]),
            Verdict::Failed
        );
        assert_eq!(
            verdict(
                &headers(&["evilexample.net; dmarc=fail"]),
                &ada(),
                &["example.net"]
            ),
            Verdict::Unknown
        );
    }

    fn named(name: &str, address: &str) -> Vec<EmailAddress> {
        vec![EmailAddress::new(Some(name), address)]
    }

    #[test]
    fn a_name_that_is_an_address_other_than_the_senders_is_a_hit() {
        assert_eq!(
            impersonation(&named("billing@bank.example", "x@other.example")),
            Some("billing@bank.example".into())
        );
        assert_eq!(
            impersonation(&named("Bank <support@bank.example>", "x@other.example")),
            Some("support@bank.example".into())
        );
    }

    #[test]
    fn a_name_that_is_a_domain_other_than_the_senders_is_a_hit() {
        assert_eq!(
            impersonation(&named("bank.example Support", "x@other.example")),
            Some("bank.example".into())
        );
    }

    #[test]
    fn ordinary_names_are_not_hits() {
        for (name, address) in [
            ("Ada Example", "ada@example.org"),
            ("Dr. J.R. Smith", "smith@example.org"),
            ("ada@example.org", "ada@example.org"),
            ("Ada@Example.org", "ada@example.org"),
            ("example.org Billing", "billing@mail.example.org"),
            ("mail.example.org", "billing@example.org"),
            ("v2.1 release", "ada@example.org"),
        ] {
            assert_eq!(impersonation(&named(name, address)), None, "{name}");
        }
    }
}
