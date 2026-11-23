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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub status: String,
    pub scope: String,
    pub signer: Option<String>,
    pub accepted: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mark {
    Signed,
    SignedAccepted,
}

impl Mark {
    pub fn label(self) -> &'static str {
        match self {
            Mark::Signed => "Signed",
            Mark::SignedAccepted => "Signed · accepted",
        }
    }

    pub fn accessible_name(self, address: &str) -> String {
        match self {
            Mark::Signed => "Signed. Open verification".to_owned(),
            Mark::SignedAccepted => {
                format!("Signed with the key you accepted for {address}. Open verification")
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HeaderAuthenticity {
    pub warning: Option<String>,
    pub mark: Option<Mark>,
}

pub fn header<T: AsRef<str>>(
    headers: &Headers,
    from: &[EmailAddress],
    trusted: &[T],
    signature: Option<&Signature>,
) -> HeaderAuthenticity {
    let mut lines: Vec<String> = warning(headers, from, trusted).into_iter().collect();
    let mut mark = None;
    if let Some(signature) = signature {
        let address = from.first().map_or("", |sender| sender.address.as_str());
        match (signature.status.as_str(), signature.scope.as_str()) {
            ("bad", _) => {
                lines.push("The signature on this message does not match its content.".to_owned());
            }
            ("revoked", _) => {
                lines.push("This message was signed with a key that has been revoked.".to_owned())
            }
            ("good", "whole") => match &signature.accepted {
                Some(accepted) if Some(accepted) == signature.signer.as_ref() => {
                    mark = Some(Mark::SignedAccepted);
                }
                Some(_) => lines.push(format!(
                    "Signed with a different key than the one you accepted for {address}."
                )),
                None => mark = Some(Mark::Signed),
            },
            _ => {}
        }
    }
    HeaderAuthenticity {
        warning: (!lines.is_empty()).then(|| lines.join(" ")),
        mark,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    Spf,
    Dkim,
    Dmarc,
    Arc,
    Other(String),
}

impl Method {
    fn named(name: &str) -> Method {
        match name {
            "spf" => Method::Spf,
            "dkim" => Method::Dkim,
            "dmarc" => Method::Dmarc,
            "arc" => Method::Arc,
            other => Method::Other(other.to_owned()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Aligned,
    NotAligned,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodResult {
    pub method: Method,
    pub result: String,
    pub domain: Option<String>,
    pub selector: Option<String>,
    pub policy: Option<String>,
    pub aligned: Alignment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthReport {
    pub authserv_id: String,
    pub trusted: bool,
    pub position: usize,
    pub arc_instance: Option<u32>,
    pub results: Vec<MethodResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArcSet {
    pub instance: u32,
    pub sealer: String,
    pub cv: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Verification {
    pub provider: Option<AuthReport>,
    pub provider_headers: usize,
    pub not_believed: Vec<AuthReport>,
    pub arc: Vec<ArcSet>,
}

pub fn verification<T: AsRef<str>>(
    headers: &Headers,
    from: &[EmailAddress],
    trusted: &[T],
) -> Verification {
    let from_domain = from
        .first()
        .and_then(EmailAddress::domain)
        .map(str::to_ascii_lowercase);
    let from_domain = from_domain.as_deref();
    let mut out = Verification::default();
    for (position, value) in headers
        .get_all("Authentication-Results")
        .into_iter()
        .enumerate()
    {
        let Some(parsed) = parse(value) else { continue };
        let ours = trusts(trusted, &parsed.authserv_id);
        out.provider_headers += usize::from(ours);
        let believed = ours && out.provider.is_none();
        let report = parsed.report(position, believed, None, from_domain);
        if believed {
            out.provider = Some(report);
        } else {
            out.not_believed.push(report);
        }
    }
    for (position, value) in headers
        .get_all("ARC-Authentication-Results")
        .into_iter()
        .enumerate()
    {
        let (instance, rest) = arc_instance(value);
        if let Some(parsed) = parse(rest) {
            out.not_believed
                .push(parsed.report(position, false, instance, from_domain));
        }
    }
    out.arc = headers
        .get_all("ARC-Seal")
        .into_iter()
        .filter_map(|value| {
            let tags = tags(value);
            Some(ArcSet {
                instance: tags.get("i")?.parse().ok()?,
                sealer: tags.get("d")?.to_ascii_lowercase(),
                cv: tags.get("cv").map(|cv| cv.to_ascii_lowercase()),
            })
        })
        .collect();
    out.arc.sort_by_key(|set| set.instance);
    out
}

fn arc_instance(value: &str) -> (Option<u32>, &str) {
    match value.split_once(';') {
        Some((head, rest)) => match head.trim().strip_prefix("i=") {
            Some(n) => (n.trim().parse().ok(), rest),
            None => (None, value),
        },
        None => (None, value),
    }
}

fn tags(value: &str) -> std::collections::BTreeMap<String, String> {
    value
        .split(';')
        .filter_map(|tag| tag.split_once('='))
        .map(|(key, value)| {
            (
                key.trim().to_ascii_lowercase(),
                value.split_whitespace().collect::<String>(),
            )
        })
        .collect()
}

fn aligned(domain: Option<&str>, from: Option<&str>) -> Alignment {
    match (domain, from) {
        (Some(domain), Some(from)) => {
            if organisational(domain) == organisational(from) {
                Alignment::Aligned
            } else {
                Alignment::NotAligned
            }
        }
        _ => Alignment::Unknown,
    }
}

fn organisational(host: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    psl::domain(host.as_bytes())
        .and_then(|domain| std::str::from_utf8(domain.as_bytes()).ok())
        .map(str::to_owned)
        .unwrap_or(host)
}

#[derive(Debug)]
struct Result {
    method: String,
    result: String,
    properties: Vec<(String, String)>,
    comment: String,
}

impl Result {
    fn report(&self, from: Option<&str>) -> MethodResult {
        let method = Method::named(&self.method);
        let domain = match method {
            Method::Dkim => self.property("header.d").or(self.property("header.i")),
            Method::Dmarc => self.property("header.from"),
            Method::Spf => self
                .property("smtp.mailfrom")
                .or(self.property("smtp.helo")),
            _ => None,
        }
        .map(|value| value.rsplit('@').next().unwrap_or(value).to_owned());
        let selector = (method == Method::Dkim)
            .then(|| self.property("header.s").map(str::to_owned))
            .flatten();
        let policy = (method == Method::Dmarc)
            .then(|| {
                self.properties
                    .iter()
                    .find(|(key, _)| key.starts_with("policy."))
                    .map(|(_, value)| value.clone())
                    .or_else(|| {
                        self.comment
                            .split(|c: char| c.is_whitespace() || c == ',')
                            .find_map(|word| word.strip_prefix("p="))
                            .map(str::to_ascii_lowercase)
                    })
            })
            .flatten();
        MethodResult {
            aligned: aligned(domain.as_deref(), from),
            method,
            result: self.result.clone(),
            domain,
            selector,
            policy,
        }
    }

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

impl Parsed {
    fn report(
        &self,
        position: usize,
        trusted: bool,
        arc_instance: Option<u32>,
        from: Option<&str>,
    ) -> AuthReport {
        AuthReport {
            authserv_id: self.authserv_id.clone(),
            trusted,
            position,
            arc_instance,
            results: self.results.iter().map(|r| r.report(from)).collect(),
        }
    }
}

fn parse(value: &str) -> Option<Parsed> {
    let mut sections = sections(value).into_iter();
    let (head, _) = sections.next()?;
    let authserv_id = head
        .split_whitespace()
        .next()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let results = sections
        .filter_map(|(section, comment)| {
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
                comment,
            })
        })
        .collect();
    Some(Parsed {
        authserv_id,
        results,
    })
}

fn sections(value: &str) -> Vec<(String, String)> {
    let mut depth = 0usize;
    let mut out = vec![(String::new(), String::new())];
    for c in value.chars() {
        let (text, comment) = out.last_mut().expect("never empty");
        match c {
            '(' => {
                if depth > 0 {
                    comment.push(c);
                }
                depth += 1;
            }
            ')' => {
                depth = depth.saturating_sub(1);
                if depth > 0 {
                    comment.push(c);
                } else {
                    comment.push(' ');
                }
            }
            _ if depth > 0 => comment.push(c),
            ';' => out.push((String::new(), String::new())),
            _ => text.push(c),
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
        type Row<'a> = (
            Method,
            &'a str,
            Option<&'a str>,
            Option<&'a str>,
            Option<&'a str>,
        );
        let rows: Vec<Row> = provider
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

    fn signed(status: &str, scope: &str) -> Signature {
        Signature {
            status: status.to_owned(),
            scope: scope.to_owned(),
            signer: Some("AAAA".to_owned()),
            accepted: None,
        }
    }

    fn mark_of(signature: Option<Signature>) -> HeaderAuthenticity {
        header(
            &Headers::new(),
            &from("ada@example.com"),
            &["mx.example.net"],
            signature.as_ref(),
        )
    }

    #[test]
    fn the_header_table_one_row_at_a_time() {
        let different =
            "Signed with a different key than the one you accepted for ada@example.com.";
        let accepted = |key: &str| Signature {
            accepted: Some(key.to_owned()),
            ..signed("good", "whole")
        };
        for (signature, mark, warning) in [
            (None, None, None),
            (Some(signed("good", "whole")), Some(Mark::Signed), None),
            (Some(accepted("AAAA")), Some(Mark::SignedAccepted), None),
            (Some(accepted("BBBB")), None, Some(different)),
            (Some(signed("good", "part")), None, None),
            (Some(signed("good", "inline")), None, None),
            (
                Some(signed("bad", "whole")),
                None,
                Some("The signature on this message does not match its content."),
            ),
            (
                Some(signed("revoked", "whole")),
                None,
                Some("This message was signed with a key that has been revoked."),
            ),
            (Some(signed("cant_check", "whole")), None, None),
            (Some(signed("not_here_yet", "whole")), None, None),
        ] {
            let said = mark_of(signature.clone());
            assert_eq!(
                (said.mark, said.warning.as_deref()),
                (mark, warning),
                "{signature:?}"
            );
        }
    }

    #[test]
    fn a_domain_warning_comes_first_and_the_mark_stands_beside_it() {
        let mut headers = Headers::new();
        headers.push(
            "Authentication-Results",
            "mx.example.net; dmarc=fail header.from=example.com",
        );
        let said = header(
            &headers,
            &from("ada@example.com"),
            &["mx.example.net"],
            Some(&signed("bad", "whole")),
        );
        assert_eq!(
            said.warning.as_deref(),
            Some(
                "This message failed sender verification: it may not be from example.com. \
                 The signature on this message does not match its content."
            )
        );
        let good = header(
            &headers,
            &from("ada@example.com"),
            &["mx.example.net"],
            Some(&signed("good", "whole")),
        );
        assert_eq!(
            good.mark,
            Some(Mark::Signed),
            "the mark is about the content, the line about the domain"
        );
        assert!(good.warning.is_some());
    }

    #[test]
    fn the_mark_says_its_words_and_its_accessible_name() {
        assert_eq!(Mark::Signed.label(), "Signed");
        assert_eq!(Mark::SignedAccepted.label(), "Signed · accepted");
        assert_eq!(
            Mark::Signed.accessible_name("ada@example.com"),
            "Signed. Open verification"
        );
        assert_eq!(
            Mark::SignedAccepted.accessible_name("ada@example.com"),
            "Signed with the key you accepted for ada@example.com. Open verification"
        );
    }
}
