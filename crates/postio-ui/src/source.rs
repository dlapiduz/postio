use postio_model::{EmailAddress, Headers, headers};

use crate::reader::authenticity::{self, Alignment, AuthReport, Method, MethodResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Verification,
    Headers,
    Raw,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Verification, Tab::Headers, Tab::Raw];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Verification => "Verification",
            Tab::Headers => "Headers",
            Tab::Raw => "Raw",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Tab::Verification => "verification",
            Tab::Headers => "headers",
            Tab::Raw => "raw",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Provider<'a> {
    pub name: &'a str,
    pub authserv_ids: &'a [&'a str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawState {
    Here,
    Fetching,
    Offline,
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyState {
    Here,
    Partial,
}

pub struct SourceInput<'a> {
    pub subject: &'a str,
    pub from: &'a [EmailAddress],
    pub headers: Option<&'a str>,
    pub envelope: Option<&'a str>,
    pub raw: Option<Vec<u8>>,
    pub raw_state: RawState,
    pub body: BodyState,
    pub provider: Option<Provider<'a>>,
    pub sent_by_you: bool,
    pub account: &'a str,
    pub signature: Option<SignatureInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureInput {
    pub status: String,
    pub reason: Option<String>,
    pub scope: String,
    pub fingerprint: Option<String>,
    pub sources: Vec<String>,
    pub first_seen: Option<i64>,
    pub accepted_at: Option<i64>,
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceView {
    pub title: String,
    pub verification: VerificationView,
    pub headers: HeadersView,
    pub raw: RawView,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VerificationView {
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    pub title: String,
    pub intro: Option<String>,
    pub mono: Option<String>,
    pub lines: Vec<Line>,
    pub quiet: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub mono: Option<String>,
}

impl Line {
    fn text(text: impl Into<String>) -> Line {
        Line {
            text: text.into(),
            mono: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HeadersView {
    pub rows: Vec<HeaderRow>,
    pub hops: Vec<Hop>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderRow {
    pub name: String,
    pub value: String,
    pub raw: Option<String>,
    pub boundary_above: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub from: Option<String>,
    pub by: Option<String>,
    pub at: Option<String>,
    pub delay: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawView {
    pub segments: Vec<RawSegment>,
    pub state_line: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawSegment {
    Text(String),
    Fold {
        label: String,
        range: std::ops::Range<usize>,
    },
}

pub fn eml_name(subject: &str) -> String {
    let kept: String = subject
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let kept = kept
        .split_whitespace()
        .filter(|word| !word.chars().all(|c| c == '.'))
        .collect::<Vec<_>>()
        .join(" ");
    let kept: String = kept.chars().take(80).collect();
    if kept.is_empty() {
        "message.eml".to_owned()
    } else {
        format!("{kept}.eml")
    }
}

pub const FIXED_LINE: &str = "These checks are about the sending domain and the message's integrity. They don't tell you the person is who you think.";
pub const NOT_CHECKED_HERE: &str = "DKIM, SPF and DMARC were not checked on this computer. Your provider checked them when the mail arrived.";

pub fn view(mut input: SourceInput<'_>) -> SourceView {
    let raw = input.raw.take();
    let parsed = match raw.as_deref() {
        Some(raw) => Some(postio_model::mime::parse_headers(raw).headers),
        None => input.headers.map(headers::parse_block),
    };
    let trusted: &[&str] = input.provider.map_or(&[], |provider| provider.authserv_ids);
    let report = parsed
        .as_ref()
        .map(|headers| authenticity::verification(headers, input.from, trusted));
    SourceView {
        title: format!("Source · {}", input.subject),
        verification: verification(&input, report.as_ref()),
        headers: HeadersView {
            rows: match (&parsed, input.envelope) {
                (Some(headers), _) => rows(headers, input.provider, report.as_ref()),
                (None, Some(envelope)) => rows(&headers::parse_block(envelope), None, None),
                (None, None) => Vec::new(),
            },
            hops: parsed.as_ref().map(hops).unwrap_or_default(),
            note: parsed
                .is_none()
                .then(|| "More headers arrive with the body.".to_owned()),
        },
        raw: RawView {
            segments: raw.as_deref().map(segments).unwrap_or_default(),
            state_line: raw_state_line(&input, raw.is_some()),
            bytes: raw.unwrap_or_default(),
        },
    }
}

pub const FOLD_OVER: usize = 64 * 1024;

fn segments(raw: &[u8]) -> Vec<RawSegment> {
    let mut out = Vec::new();
    let mut at = 0;
    for leaf in postio_model::mime::leaf_bodies(raw) {
        if leaf.range.len() <= FOLD_OVER {
            continue;
        }
        if at < leaf.range.start {
            out.push(RawSegment::Text(
                String::from_utf8_lossy(&raw[at..leaf.range.start]).into_owned(),
            ));
        }
        let mut label = format!(
            "{} · {}",
            leaf.encoding,
            crate::format::human_size(leaf.range.len() as u64)
        );
        if let Some(name) = &leaf.name {
            label.push_str(&format!(" · {name}"));
        }
        at = leaf.range.end;
        out.push(RawSegment::Fold {
            label,
            range: leaf.range,
        });
    }
    if at < raw.len() {
        out.push(RawSegment::Text(
            String::from_utf8_lossy(&raw[at..]).into_owned(),
        ));
    }
    out
}

fn rows(
    headers: &Headers,
    provider: Option<Provider<'_>>,
    report: Option<&authenticity::Verification>,
) -> Vec<HeaderRow> {
    let trusted_at = report
        .and_then(|report| report.provider.as_ref())
        .map(|trusted| trusted.position);
    let mut results_seen = 0;
    let mut boundary_next = false;
    headers
        .iter()
        .map(|header| {
            let decoded = postio_model::mime::decode_header_text(header.value.as_bytes());
            let collapsed = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
            let raw =
                (collapsed(&decoded) != collapsed(&header.value)).then(|| header.value.clone());
            let boundary_above = std::mem::take(&mut boundary_next).then(|| {
                format!(
                    "Above: written by {}. Below: came with the message.",
                    provider.map_or("your provider", |provider| provider.name)
                )
            });
            if header.name.eq_ignore_ascii_case("Authentication-Results") {
                boundary_next = trusted_at == Some(results_seen);
                results_seen += 1;
            }
            HeaderRow {
                name: header.name.clone(),
                value: decoded,
                raw,
                boundary_above,
            }
        })
        .collect()
}

fn hops(headers: &Headers) -> Vec<Hop> {
    let timed: Vec<(Hop, Option<i64>)> = headers.get_all("Received").into_iter().map(hop).collect();
    let times: Vec<Option<i64>> = timed.iter().map(|(_, seconds)| *seconds).collect();
    timed
        .into_iter()
        .enumerate()
        .map(|(at, (hop, seconds))| Hop {
            delay: match (seconds, times.get(at + 1).copied().flatten()) {
                (Some(this), Some(before)) => Some(delay(this - before)),
                _ => None,
            },
            ..hop
        })
        .collect()
}

fn hop(value: &str) -> (Hop, Option<i64>) {
    let (route, date) = match value.rsplit_once(';') {
        Some((route, date)) => (route, Some(without_comments(date).trim().to_owned())),
        None => (value, None),
    };
    let route = without_comments(route);
    let mut words = route.split_whitespace();
    let (mut from, mut by) = (None, None);
    while let Some(word) = words.next() {
        match word.to_ascii_lowercase().as_str() {
            "from" if from.is_none() => from = words.next().map(str::to_owned),
            "by" if by.is_none() => by = words.next().map(str::to_owned),
            _ => {}
        }
    }
    let at = date.filter(|date| !date.is_empty());
    let seconds = at
        .as_deref()
        .and_then(|date| chrono::DateTime::parse_from_rfc2822(date).ok())
        .map(|time| time.timestamp());
    (
        Hop {
            from,
            by,
            at,
            delay: None,
        },
        seconds,
    )
}

fn delay(seconds: i64) -> String {
    let sign = if seconds < 0 { "−" } else { "" };
    let seconds = seconds.unsigned_abs();
    let text = match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{} min {} s", seconds / 60, seconds % 60),
        3600..86400 => format!("{} h {} min", seconds / 3600, seconds % 3600 / 60),
        _ => format!("{} days", seconds / 86400),
    };
    format!("{sign}{text}")
}

fn without_comments(value: &str) -> String {
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

fn raw_state_line(input: &SourceInput<'_>, here: bool) -> Option<String> {
    if here {
        return None;
    }
    match input.raw_state {
        RawState::Here => None,
        RawState::Fetching => Some(format!("Fetching the message from {}…", input.account)),
        RawState::Offline => Some(format!(
            "This message isn't on this computer yet. Postio will show it when {} is back online.",
            input.account
        )),
        RawState::Unreachable => Some(format!(
            "This message isn't on this computer, and {} isn't syncing, so Postio can't fetch it.",
            input.account
        )),
    }
}

fn verification(
    input: &SourceInput<'_>,
    report: Option<&authenticity::Verification>,
) -> VerificationView {
    let limits = Section {
        title: "What this does not tell you".to_owned(),
        lines: vec![Line::text(FIXED_LINE)],
        ..Section::default()
    };
    if input.sent_by_you {
        return VerificationView {
            sections: vec![
                Section {
                    title: "You sent this".to_owned(),
                    intro: Some("Postio doesn't judge mail you sent.".to_owned()),
                    ..Section::default()
                },
                limits,
            ],
        };
    }
    let mut sections = Vec::new();
    let Some(report) = report.filter(|_| input.body == BodyState::Here) else {
        let account = input.account;
        let intro = match (input.body, input.raw_state) {
            (BodyState::Partial, _) => {
                "The headers that carry these checks arrive with the message body.".to_owned()
            }
            (BodyState::Here, RawState::Fetching) => {
                format!("The headers that carry these checks are on their way from {account}.")
            }
            (BodyState::Here, RawState::Unreachable) => format!(
                "The headers that carry these checks aren't on this computer, and {account} isn't syncing."
            ),
            (BodyState::Here, _) => format!(
                "The headers that carry these checks aren't on this computer yet. Postio will read them when {account} is back online."
            ),
        };
        let state = match input.raw_state {
            RawState::Fetching => format!("{account} is sending them now."),
            RawState::Offline => format!("{account} is offline."),
            RawState::Unreachable => format!("{account} isn't syncing."),
            RawState::Here => String::new(),
        };
        sections.push(Section {
            title: "What your provider checked".to_owned(),
            intro: Some(intro),
            lines: (input.body == BodyState::Partial && !state.is_empty())
                .then(|| Line::text(state))
                .into_iter()
                .collect(),
            ..Section::default()
        });
        if let Some(signature) = &input.signature {
            sections.push(signature_section(signature));
        }
        sections.push(local(input));
        sections.push(limits);
        return VerificationView { sections };
    };
    sections.push(provider_section(input, report));
    if let Some(signature) = &input.signature {
        sections.push(signature_section(signature));
    }
    if !report.not_believed.is_empty() {
        sections.push(Section {
            title: "Headers Postio does not believe".to_owned(),
            intro: Some(if report.provider.is_some() {
                "Written before your provider received the message. Anyone could have written these."
                    .to_owned()
            } else {
                "Postio can't tell which of these your provider wrote, so it believes none of them."
                    .to_owned()
            }),
            lines: report.not_believed.iter().map(doubted).collect(),
            quiet: true,
            ..Section::default()
        });
    }
    sections.push(local(input));
    sections.push(limits);
    VerificationView { sections }
}

fn provider_section(input: &SourceInput<'_>, report: &authenticity::Verification) -> Section {
    let title = "What your provider checked".to_owned();
    let (Some(provider), Some(trusted)) = (input.provider, report.provider.as_ref()) else {
        return Section {
            title,
            intro: Some("Your provider didn't add a result Postio can trust.".to_owned()),
            ..Section::default()
        };
    };
    let mut intro = format!("Checked by {} when the message arrived.", provider.name);
    if report.provider_headers > 1 {
        intro.push_str(&format!(
            " It added {} results; Postio believes the topmost.",
            report.provider_headers
        ));
    }
    Section {
        title,
        intro: Some(intro),
        mono: Some(trusted.authserv_id.clone()),
        lines: provider_lines(trusted, &report.arc),
        ..Section::default()
    }
}

fn provider_lines(report: &AuthReport, arc: &[authenticity::ArcSet]) -> Vec<Line> {
    let mut lines = Vec::new();
    let dkim: Vec<&MethodResult> = report
        .results
        .iter()
        .filter(|r| r.method == Method::Dkim)
        .collect();
    let mut dkim_said = false;
    for result in &report.results {
        let line = match &result.method {
            Method::Dkim if dkim.len() > 1 => {
                if dkim_said {
                    continue;
                }
                dkim_said = true;
                let each: Vec<String> = dkim
                    .iter()
                    .map(|r| {
                        format!(
                            "{} {}{}",
                            r.domain.as_deref().unwrap_or("A signature"),
                            verb(&r.result),
                            match r.aligned {
                                Alignment::Aligned => " and matches the From domain",
                                Alignment::NotAligned => " and is not the From domain",
                                Alignment::Unknown => "",
                            }
                        )
                    })
                    .collect();
                format!("DKIM: {} signatures. {}.", dkim.len(), each.join(". "))
            }
            Method::Dkim => match &result.domain {
                Some(domain) => format!(
                    "DKIM {} for {domain}{}.",
                    verb(&result.result),
                    match result.aligned {
                        Alignment::Aligned => ", which matches the From domain",
                        Alignment::NotAligned => ", which is not the From domain",
                        Alignment::Unknown => "",
                    }
                ),
                None => format!("DKIM {}.", verb(&result.result)),
            },
            Method::Spf => match &result.domain {
                Some(domain) => format!(
                    "SPF {} for {domain}{}.",
                    verb(&result.result),
                    clause(result.aligned)
                ),
                None => format!("SPF {}.", verb(&result.result)),
            },
            Method::Dmarc => {
                let mut line = format!("DMARC {}", verb(&result.result));
                if let Some(domain) = &result.domain {
                    line.push_str(&format!(" for {domain}"));
                }
                if let Some(policy) = &result.policy {
                    line.push_str(&format!(" (policy: {policy})"));
                }
                if result.aligned == Alignment::NotAligned {
                    line.push_str(", which is not the From domain");
                }
                line.push('.');
                line
            }
            Method::Arc => arc_line(&result.result, arc),
            Method::Other(name) => format!("{name}: {}.", result.result),
        };
        lines.push(Line::text(line));
    }
    lines
}

fn clause(aligned: Alignment) -> &'static str {
    match aligned {
        Alignment::Aligned => ", which is the From domain",
        Alignment::NotAligned => ", which is not the From domain",
        Alignment::Unknown => "",
    }
}

fn arc_line(result: &str, arc: &[authenticity::ArcSet]) -> String {
    let chain = match result {
        "pass" => "Your provider says the chain is intact.".to_owned(),
        "fail" => "Your provider says the chain is broken.".to_owned(),
        other => format!("Your provider says {other}."),
    };
    let sealers: Vec<&str> = arc.iter().map(|set| set.sealer.as_str()).collect();
    match sealers.as_slice() {
        [] => format!("ARC: {chain}"),
        [one] => format!("ARC: a forwarder, {one}, sealed the results it saw. {chain}"),
        many => format!(
            "ARC: {} forwarders, {}, sealed the results they saw. {chain}",
            many.len(),
            many.join(", ")
        ),
    }
}

fn verb(result: &str) -> String {
    match result {
        "pass" => "passed".to_owned(),
        "fail" => "failed".to_owned(),
        "softfail" => "soft-failed".to_owned(),
        "neutral" => "was neutral".to_owned(),
        "none" => "found nothing to check".to_owned(),
        "temperror" => "couldn't be checked (a temporary error)".to_owned(),
        "permerror" => "couldn't be checked (a permanent error)".to_owned(),
        "policy" => "was overridden by the provider's policy".to_owned(),
        other => format!("said {other}"),
    }
}

fn doubted(report: &AuthReport) -> Line {
    let said: Vec<String> = report
        .results
        .iter()
        .map(|result| {
            let name = match &result.method {
                Method::Spf => "SPF".to_owned(),
                Method::Dkim => "DKIM".to_owned(),
                Method::Dmarc => "DMARC".to_owned(),
                Method::Arc => "ARC".to_owned(),
                Method::Other(name) => name.clone(),
            };
            let aligned = match (&result.method, result.aligned) {
                (Method::Dmarc | Method::Arc, _) | (_, Alignment::Unknown) => "",
                (_, Alignment::Aligned) => ", the From domain",
                (_, Alignment::NotAligned) => ", not the From domain",
            };
            match &result.domain {
                Some(domain) => format!("{name} {} for {domain}{aligned}.", verb(&result.result)),
                None => format!("{name} {}.", verb(&result.result)),
            }
        })
        .collect();
    let said = if said.is_empty() {
        "No results.".to_owned()
    } else {
        said.join(" ")
    };
    Line {
        text: match report.arc_instance {
            Some(instance) => format!("ARC set {instance}: {said}"),
            None => said,
        },
        mono: Some(report.authserv_id.clone()),
    }
}

fn signature_section(signature: &SignatureInput) -> Section {
    let address = signature.address.as_str();
    let source = signature
        .sources
        .iter()
        .map(|source| match source.as_str() {
            "autocrypt" => format!("{address}'s Autocrypt header"),
            "gnupg" => "your GnuPG keyring".to_owned(),
            "file" => "a file you imported".to_owned(),
            "attachment" => "a key attached to a message".to_owned(),
            other => other.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" and ");
    let seen = signature
        .first_seen
        .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
        .map(|at| format!(", first seen {}", at.format("%-d %B %Y")))
        .unwrap_or_default();
    let intro = match (signature.status.as_str(), signature.scope.as_str()) {
        ("good", "whole") => format!("Good signature by a key from {source}{seen}."),
        ("good", _) => "Part of this message is signed: a list or a forward added text the signature doesn't cover.".to_owned(),
        ("bad", _) => "Bad signature: the message changed after it was signed.".to_owned(),
        ("revoked", _) => {
            "Revoked key: the key that made this signature has been revoked.".to_owned()
        }
        ("not_here_yet", _) => "The signed message isn't fully on this computer yet; Postio will check the signature when it is.".to_owned(),
        _ => match signature.reason.as_deref() {
            Some("no_key") => {
                "Can't check: no key for this signature is on this computer.".to_owned()
            }
            Some(reason) if reason.starts_with("weak_hash:") => format!(
                "Can't check: this signature uses {}, which is no longer safe.",
                reason
                    .trim_start_matches("weak_hash:")
                    .to_ascii_uppercase()
                    .replace("SHA", "SHA-")
            ),
            Some("weak_key") => "Can't check: the key is too short to be safe.".to_owned(),
            Some("expired") => "Bad signature: it was made after its key expired.".to_owned(),
            Some("not_yet_valid") => {
                "Bad signature: it was made before its key existed.".to_owned()
            }
            Some("unsupported") => "Can't check: Postio can't check this kind of message yet.".to_owned(),
            _ => "Can't check: the signature couldn't be read.".to_owned(),
        },
    };
    let mut lines = Vec::new();
    if signature.status == "good" {
        lines.push(Line::text(
            "Signed means this message hasn't changed since this key signed it. It doesn't tell you who holds that key.",
        ));
        match signature
            .accepted_at
            .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
        {
            Some(at) => lines.push(Line::text(format!(
                "You accepted this key for {address} on {}.",
                at.format("%-d %B %Y")
            ))),
            None => lines.push(Line::text(format!("Accept this key for {address}"))),
        }
    }
    Section {
        title: "Signature".to_owned(),
        intro: Some(intro),
        mono: signature.fingerprint.as_deref().map(grouped),
        lines,
        ..Section::default()
    }
}

fn grouped(fingerprint: &str) -> String {
    fingerprint
        .as_bytes()
        .chunks(4)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn local(input: &SourceInput<'_>) -> Section {
    let name = match (authenticity::impersonation(input.from), input.from.first()) {
        (Some(claimed), Some(sender)) => format!(
            "The name says {claimed}, but it was sent from {}.",
            sender.address
        ),
        _ => "The display name is not another address or domain.".to_owned(),
    };
    Section {
        title: "What Postio checked on this computer".to_owned(),
        lines: [Line::text(name)]
            .into_iter()
            .chain(
                input
                    .signature
                    .as_ref()
                    .map(|_| Line::text("The signature was checked on this computer.")),
            )
            .chain([Line::text(NOT_CHECKED_HERE)])
            .collect(),
        ..Section::default()
    }
}

pub fn text(view: &SourceView, tab: Tab) -> String {
    let mut out = String::new();
    match tab {
        Tab::Verification => {
            for section in &view.verification.sections {
                out.push_str(&section.title);
                out.push('\n');
                if let Some(intro) = &section.intro {
                    out.push_str(&format!("  {intro}\n"));
                }
                if let Some(mono) = &section.mono {
                    out.push_str(&format!("  {mono}\n"));
                }
                for line in &section.lines {
                    out.push_str(&format!("  {}\n", line.text));
                    if let Some(mono) = &line.mono {
                        out.push_str(&format!("    {mono}\n"));
                    }
                }
                out.push('\n');
            }
        }
        Tab::Headers => {
            if let Some(note) = &view.headers.note {
                out.push_str(&format!("{note}\n\n"));
            }
            if !view.headers.hops.is_empty() {
                out.push_str("Received, newest first\n");
                for hop in &view.headers.hops {
                    let mut line = String::from(" ");
                    if let Some(from) = &hop.from {
                        line.push_str(&format!(" from {from}"));
                    }
                    if let Some(by) = &hop.by {
                        line.push_str(&format!(" by {by}"));
                    }
                    if let Some(at) = &hop.at {
                        line.push_str(&format!(" · {at}"));
                    }
                    if let Some(delay) = &hop.delay {
                        line.push_str(&format!(" · {delay} after the hop before"));
                    }
                    out.push_str(&line);
                    out.push('\n');
                }
                out.push('\n');
            }
            for row in &view.headers.rows {
                if let Some(boundary) = &row.boundary_above {
                    out.push_str(&format!("──── {boundary}\n"));
                }
                out.push_str(&format!("{}: {}\n", row.name, row.value));
                if let Some(raw) = &row.raw {
                    out.push_str(&format!("    as written: {raw}\n"));
                }
            }
        }
        Tab::Raw => match &view.raw.state_line {
            Some(state) => out.push_str(state),
            None => {
                for segment in &view.raw.segments {
                    match segment {
                        RawSegment::Text(text) => out.push_str(text),
                        RawSegment::Fold { label, .. } => out.push_str(&format!("[{label}]")),
                    }
                }
            }
        },
    }
    out
}

pub fn footer(keymap: &postio_core::Keymap) -> crate::scope::FooterView {
    use postio_core::CommandId;
    let keys = [
        (CommandId::SourceTabVerification, "verification"),
        (CommandId::SourceTabHeaders, "headers"),
        (CommandId::SourceTabRaw, "raw"),
        (CommandId::CopySource, "copy all"),
        (CommandId::SaveSource, "save as .eml"),
        (CommandId::Back, "close"),
    ]
    .into_iter()
    .filter_map(|(command, label)| crate::hints::hint(keymap, command, label))
    .map(|hint| (crate::hints::short(&hint.key), hint.label))
    .collect();
    crate::scope::FooterView {
        title: "Source and verification".to_owned(),
        keys,
        note: None,
    }
}
