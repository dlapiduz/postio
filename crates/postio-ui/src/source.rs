//! The Source surface's view model: the Verification, Headers and Raw tabs as text, and every
//! string they say. Nothing here is rendered or resolved: a URL is characters (spec 016 D3).

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

    /// What storyboards and tests observe.
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
    /// The preset's display name.
    pub name: &'a str,
    pub authserv_ids: &'a [&'a str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawState {
    Here,
    Fetching,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyState {
    Here,
    Partial,
}

pub struct SourceInput<'a> {
    pub subject: &'a str,
    pub from: &'a [EmailAddress],
    /// The stored header block, as `postio_model::headers::block_of` keeps it.
    pub headers: Option<&'a str>,
    pub raw: Option<&'a [u8]>,
    pub raw_state: RawState,
    pub body: BodyState,
    pub provider: Option<Provider<'a>>,
    pub sent_by_you: bool,
    /// The account's name, for the fetch and offline lines.
    pub account: &'a str,
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
    /// Drawn in mono after the intro: an authserv-id, a fingerprint.
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
    /// Unfolded, with encoded words decoded.
    pub value: String,
    /// As written, when decoding changed it.
    pub raw: Option<String>,
    /// The trust boundary is drawn above this row: rows above it the provider wrote.
    pub boundary_above: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub from: Option<String>,
    pub by: Option<String>,
    pub at: Option<String>,
    /// Since the hop before it, which is the next one down.
    pub delay: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawView {
    pub segments: Vec<RawSegment>,
    pub state_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawSegment {
    Text(String),
    /// A part body over `FOLD_OVER`, drawn as one line; `range` indexes the raw bytes.
    Fold {
        label: String,
        range: std::ops::Range<usize>,
    },
}

pub const FIXED_LINE: &str = "These checks are about the sending domain and the message's integrity. They don't tell you the person is who you think.";
pub const NOT_CHECKED_HERE: &str = "DKIM, SPF and DMARC were not checked on this computer. Your provider checked them when the mail arrived.";

pub fn view(input: &SourceInput<'_>) -> SourceView {
    let block = input.headers.map(str::to_owned).or_else(|| {
        input
            .raw
            .and_then(headers::block_of)
            .map(|block| block.text)
    });
    let parsed = block.as_deref().map(headers::parse_block);
    SourceView {
        title: format!("Source · {}", input.subject),
        verification: verification(input, parsed.as_ref()),
        headers: HeadersView {
            note: (input.body == BodyState::Partial)
                .then(|| "More headers arrive with the body.".to_owned()),
            ..HeadersView::default()
        },
        raw: RawView {
            segments: Vec::new(),
            state_line: raw_state_line(input),
        },
    }
}

fn raw_state_line(input: &SourceInput<'_>) -> Option<String> {
    if input.raw.is_some() {
        return None;
    }
    match input.raw_state {
        RawState::Here => None,
        RawState::Fetching => Some(format!("Fetching the message from {}…", input.account)),
        RawState::Offline => Some(format!(
            "This message isn't on this computer yet. Postio will show it when {} is back online.",
            input.account
        )),
    }
}

fn verification(input: &SourceInput<'_>, headers: Option<&Headers>) -> VerificationView {
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
    let Some(headers) = headers.filter(|_| input.body == BodyState::Here) else {
        sections.push(Section {
            title: "What your provider checked".to_owned(),
            intro: Some(
                "The headers that carry these checks arrive with the message body.".to_owned(),
            ),
            ..Section::default()
        });
        sections.push(local(input));
        sections.push(limits);
        return VerificationView { sections };
    };
    let trusted: &[&str] = input.provider.map_or(&[], |provider| provider.authserv_ids);
    let report = authenticity::verification(headers, input.from, trusted);
    sections.push(provider_section(input, &report));
    if !report.not_believed.is_empty() {
        sections.push(Section {
            title: "Headers Postio does not believe".to_owned(),
            intro: Some(
                "Written before your provider received the message. Anyone could have written these."
                    .to_owned(),
            ),
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
            match &result.domain {
                Some(domain) => format!("{name} {} for {domain}.", verb(&result.result)),
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
        lines: vec![Line::text(name), Line::text(NOT_CHECKED_HERE)],
        ..Section::default()
    }
}
