//! Whether a message is who it says it is (#1928). `verdict` trusts only the topmost
//! `Authentication-Results` whose authserv-id belongs to the account's provider; none is `Unknown`,
//! which shows nothing. `impersonation` catches a display name that is itself an address or another
//! domain. Headers come from the downloaded body, not the envelope fetch.

use postio_model::{EmailAddress, Headers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Verified,
    Unverified,
    Failed,
    Unknown,
}

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

/// A name that merely mentions the sender's own domain, or a subdomain or parent of it, is not a
/// hit.
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
        } else if is_host(token) && has_public_suffix(token) && !related(token, &actual_domain) {
            return Some(token.to_owned());
        }
    }
    None
}

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

fn related(value: &str, domain: &str) -> bool {
    let host = value
        .rsplit('@')
        .next()
        .unwrap_or(value)
        .trim_end_matches('.')
        .to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}")) || domain.ends_with(&format!(".{host}"))
}

fn has_public_suffix(host: &str) -> bool {
    psl::suffix(host.to_ascii_lowercase().as_bytes()).is_some_and(|suffix| suffix.is_known())
}

/// Deliberately plain: `Dr.`, `J.R.` and `v2.1` are not hosts.
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
            impersonation(&named("bank.com Support", "x@other.example")),
            Some("bank.com".into())
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
            ("Node.js Weekly", "news@example.org"),
        ] {
            assert_eq!(impersonation(&named(name, address)), None, "{name}");
        }
    }

    fn from(address: &str) -> Vec<EmailAddress> {
        vec![EmailAddress::new(None::<String>, address)]
    }

    fn report(values: &[(&str, &str)], sender: &str) -> Verification {
        let mut headers = Headers::new();
        for (name, value) in values {
            headers.push(*name, *value);
        }
        verification(&headers, &from(sender), &["mx.example.net"])
    }

    const TWO_DKIM: &str = "mx.example.net; dkim=pass header.d=example.com header.s=s2026 header.b=b3du; \
        dkim=pass header.d=esp.example header.s=esp1; spf=pass smtp.mailfrom=bounce-7731@bounce.esp.example; \
        dmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=example.com";

    #[test]
    fn each_method_names_its_domain_selector_and_policy() {
        let seen = report(
            &[("Authentication-Results", TWO_DKIM)],
            "billing@example.com",
        );
        let provider = seen.provider.expect("the provider's header");
        assert_eq!(provider.authserv_id, "mx.example.net");
        assert!(provider.trusted);
        let rows: Vec<(Method, &str, Option<&str>, Option<&str>, Option<&str>)> = provider
            .results
            .iter()
            .map(|r| {
                (
                    r.method.clone(),
                    r.result.as_str(),
                    r.domain.as_deref(),
                    r.selector.as_deref(),
                    r.policy.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (
                    Method::Dkim,
                    "pass",
                    Some("example.com"),
                    Some("s2026"),
                    None
                ),
                (
                    Method::Dkim,
                    "pass",
                    Some("esp.example"),
                    Some("esp1"),
                    None
                ),
                (Method::Spf, "pass", Some("bounce.esp.example"), None, None),
                (
                    Method::Dmarc,
                    "pass",
                    Some("example.com"),
                    None,
                    Some("reject")
                ),
            ]
        );
    }

    #[test]
    fn alignment_is_relaxed_to_the_organisational_domain() {
        let value = "mx.example.net; dkim=pass header.d=mail.example.com; dkim=pass header.d=esp.example; \
            spf=pass smtp.helo=out.example.com; dkim=neutral";
        let seen = report(&[("Authentication-Results", value)], "ada@news.example.com");
        let aligned: Vec<Alignment> = seen
            .provider
            .expect("trusted")
            .results
            .iter()
            .map(|r| r.aligned)
            .collect();
        assert_eq!(
            aligned,
            [
                Alignment::Aligned,
                Alignment::NotAligned,
                Alignment::Aligned,
                Alignment::Unknown
            ]
        );
    }

    #[test]
    fn arc_sets_are_named_by_instance_and_never_believed() {
        let seen = report(
            &[
                (
                    "Authentication-Results",
                    "mx.example.net; arc=pass (i=2) header.oldest-pass=1",
                ),
                (
                    "ARC-Seal",
                    "i=2; a=rsa-sha256; cv=pass; d=relay.example.org; s=a; b=eA==",
                ),
                (
                    "ARC-Authentication-Results",
                    "i=2; mx.relay.example.org; dmarc=pass header.from=example.com",
                ),
                (
                    "ARC-Seal",
                    "i=1; a=rsa-sha256; t=1; cv=none; d=lists.example.org; s=arc; b=eA==",
                ),
                (
                    "ARC-Authentication-Results",
                    "i=1; mx.lists.example.org; dkim=pass header.d=example.com",
                ),
            ],
            "ada@example.com",
        );
        assert_eq!(
            seen.arc,
            [
                ArcSet {
                    instance: 1,
                    sealer: "lists.example.org".into(),
                    cv: Some("none".into())
                },
                ArcSet {
                    instance: 2,
                    sealer: "relay.example.org".into(),
                    cv: Some("pass".into())
                },
            ]
        );
        let doubted: Vec<(&str, Option<u32>)> = seen
            .not_believed
            .iter()
            .map(|r| (r.authserv_id.as_str(), r.arc_instance))
            .collect();
        assert_eq!(
            doubted,
            [
                ("mx.relay.example.org", Some(2)),
                ("mx.lists.example.org", Some(1))
            ]
        );
        let arc = &seen.provider.expect("trusted").results[0];
        assert_eq!(
            (arc.method.clone(), arc.result.as_str()),
            (Method::Arc, "pass")
        );
    }

    #[test]
    fn the_trusted_header_is_the_topmost_of_the_providers_and_the_rest_are_not_believed() {
        let fail = "mx.example.net; dmarc=fail header.from=example.org";
        let forged = "mx.example.net; dmarc=pass header.from=example.org";
        let other = "relay.example.com; dmarc=pass header.from=example.org";
        let seen = report(
            &[
                ("Authentication-Results", other),
                ("Authentication-Results", fail),
                ("Authentication-Results", forged),
            ],
            "ada@example.org",
        );
        let provider = seen.provider.expect("trusted");
        assert_eq!(
            (provider.position, provider.results[0].result.as_str()),
            (1, "fail")
        );
        assert_eq!(seen.provider_headers, 2, "the provider stamped twice");
        let doubted: Vec<(usize, bool)> = seen
            .not_believed
            .iter()
            .map(|r| (r.position, r.trusted))
            .collect();
        assert_eq!(doubted, [(0, false), (2, false)]);
        assert_eq!(
            verdict(
                &headers(&[other, fail, forged]),
                &from("ada@example.org"),
                &["mx.example.net"]
            ),
            Verdict::Failed,
            "the verdict reads the same header the report trusts"
        );
    }

    #[test]
    fn no_provider_header_trusts_nothing_and_doubts_everything() {
        let seen = report(
            &[("Authentication-Results", TWO_DKIM)],
            "billing@example.com",
        );
        assert!(seen.provider.is_some());
        let mut headers = Headers::new();
        headers.push("Authentication-Results", TWO_DKIM);
        let none: [&str; 0] = [];
        let seen = verification(&headers, &from("billing@example.com"), &none);
        assert!(seen.provider.is_none());
        assert_eq!(seen.not_believed.len(), 1);
    }
}
