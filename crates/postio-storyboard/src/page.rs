//! The page a person reads: every run as a filmstrip, with the keyboard's
//! region outlined on every frame, what the app said it saw, and what each
//! check came to (data-model § Review page).
//!
//! Self-contained HTML: one inline stylesheet that follows the reader's
//! light or dark preference, images by relative path, nothing fetched. It is
//! opened from disk, and it may be published as a private page when the
//! maintainer is away from the workstation, so it can rely on nothing beside
//! itself and its frames.
//!
//! A run that was not covered or not applicable is shown and counted, never
//! folded into "passed": a page that hid what it could not check would read
//! as though everything had been.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use postio_ui::observe::Observation;

use crate::bundle::{Class, Manifest};
use crate::check::Outcome;
use crate::run::{Delivery, Run, Settle, Status, StepOutcome};
use crate::verdicts::{self, Finding, Kind, Resolution, Review, Verdict};

const TEMPLATE: &str = include_str!("../templates/page.html");

/// One run, and where its files are relative to the page.
#[derive(Debug, Clone)]
pub struct Filmstrip {
    /// The run's directory relative to the page, with `/` separators, such
    /// as `runs/classic/archive-walks-down/default`.
    pub dir: String,
    /// The run.
    pub run: Run,
}

/// What the page says about itself at the top.
#[derive(Debug, Clone, Default)]
pub struct Header {
    /// The page's title: the branch, usually.
    pub title: String,
    /// The review key of the tree the runs came from.
    pub tree_key: Option<String>,
}

/// Every `run.json` under `root`, as filmstrips whose `dir` starts with
/// `prefix` -- the path from the page to `root`. Sorted by app, storyboard
/// and variant, so two builds of the page list runs in the same order.
pub fn collect(root: &Path, prefix: &str) -> io::Result<Vec<Filmstrip>> {
    let mut found = Vec::new();
    walk(root, &mut found)?;
    let mut strips = Vec::new();
    for path in found {
        let text = std::fs::read_to_string(&path)?;
        let run: Run = serde_json::from_str(&text)
            .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))?;
        let dir = path
            .parent()
            .and_then(|parent| parent.strip_prefix(root).ok())
            .map(|relative| {
                relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default();
        let dir = if prefix.is_empty() {
            dir
        } else {
            format!("{}/{dir}", prefix.trim_end_matches('/'))
        };
        strips.push(Filmstrip { dir, run });
    }
    strips.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(strips)
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found)?;
        } else if path.file_name().is_some_and(|name| name == "run.json") {
            found.push(path);
        }
    }
    Ok(())
}

/// What the review of a bundle came to.
#[derive(Debug, Clone)]
pub enum ReviewState {
    /// The bundle has no `verdicts.json`.
    NoReview,
    /// `verdicts check` rejected it; these are the problems.
    Incomplete(Vec<String>),
    /// It passed, with contests attached.
    Complete(Review),
}

/// A bundle as the page and the summary read it.
#[derive(Debug, Clone)]
pub struct Reviewed {
    /// What was to be reviewed.
    pub manifest: Manifest,
    /// What came back.
    pub state: ReviewState,
    /// Where the bundle is relative to the page, for frame links, such as
    /// `bundle`.
    pub frame_prefix: String,
}

/// Reads a bundle's manifest and review. A missing `verdicts.json` is not an
/// error: it is the state "no review ran".
pub fn load_review(bundle: &Path, frame_prefix: &str) -> Result<Reviewed, String> {
    let manifest = crate::prompt::load_manifest(bundle)?;
    let state = if !bundle.join("verdicts.json").exists() {
        ReviewState::NoReview
    } else {
        match verdicts::check(bundle) {
            Ok(review) => ReviewState::Complete(review),
            Err(rejections) => {
                ReviewState::Incomplete(rejections.iter().map(ToString::to_string).collect())
            }
        }
    };
    Ok(Reviewed {
        manifest,
        state,
        frame_prefix: frame_prefix.trim_end_matches('/').to_owned(),
    })
}

/// The page with a review's verdicts, as HTML.
pub fn render_reviewed(header: &Header, strips: &[Filmstrip], reviewed: &Reviewed) -> String {
    render_with(header, strips, Some(reviewed))
}

/// The summary a pull request carries, as text with no images
/// (contracts/review.md § What reaches the maintainer). Its first line is
/// `storyboards-key: <key>`, which is how a landing tells whether it is
/// current.
pub fn summary(header: &Header, strips: &[Filmstrip], reviewed: &Reviewed) -> String {
    let key = header
        .tree_key
        .as_deref()
        .unwrap_or(&reviewed.manifest.tree_key);
    let mut out = format!("storyboards-key: {key}\n\n");
    let title = if header.title.is_empty() {
        "Storyboards"
    } else {
        header.title.as_str()
    };
    let _ = writeln!(out, "# {title}\n");
    let review = match &reviewed.state {
        ReviewState::NoReview => None,
        ReviewState::Incomplete(problems) => {
            let _ = writeln!(out, "review incomplete: {} problem(s)", problems.len());
            for problem in problems {
                let _ = writeln!(out, "- {problem}");
            }
            None
        }
        ReviewState::Complete(review) => {
            let _ = writeln!(out, "review complete");
            Some(review)
        }
    };
    if matches!(reviewed.state, ReviewState::NoReview) {
        let _ = writeln!(out, "no review ran");
    }

    let _ = writeln!(out, "\n## Needs you\n");
    let asks = review.map(needs_you).unwrap_or_default();
    if asks.is_empty() {
        let _ = writeln!(out, "nothing");
    }
    for ask in &asks {
        let _ = write!(out, "- {} {}: {}", ask.kind, ask.citation, ask.says);
        if let Some(reason) = ask.reason {
            let _ = write!(out, " (contested: {reason})");
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "\n## Changed and new storyboards\n");
    // Each storyboard once, with the strongest class its runs have.
    let mut listed: BTreeMap<&str, Class> = BTreeMap::new();
    for run in reviewed.manifest.batches.iter().flat_map(|b| &b.runs) {
        let class = listed.entry(&run.storyboard).or_insert(run.class);
        if run.class == Class::Changed {
            *class = Class::Changed;
        }
    }
    if listed.is_empty() {
        let _ = writeln!(out, "none");
    }
    for (name, class) in listed {
        let class = match class {
            Class::Changed => "changed",
            _ => "new",
        };
        match review {
            Some(review) => {
                let of = |kind| {
                    review
                        .verdicts
                        .iter()
                        .filter(|v| v.storyboard == name && v.verdict == kind)
                        .count()
                };
                let findings = review
                    .findings
                    .iter()
                    .filter(|f| f.storyboard == name)
                    .count();
                let _ = writeln!(
                    out,
                    "- {name} ({class}): {} pass, {} fail, {} question; {findings} finding(s)",
                    of(Kind::Pass),
                    of(Kind::Fail),
                    of(Kind::Question),
                );
            }
            None => {
                let _ = writeln!(out, "- {name} ({class}): not reviewed");
            }
        }
    }

    let _ = writeln!(out, "\n## Coverage\n");
    let count = |wanted: &str| {
        strips
            .iter()
            .filter(|s| status_class(&s.run.status) == wanted)
            .count()
    };
    let _ = writeln!(out, "- unchanged: {}", reviewed.manifest.unchanged);
    let _ = writeln!(out, "- not covered: {}", count("not_covered"));
    let _ = writeln!(out, "- not applicable: {}", count("not_applicable"));
    out
}

/// The page, as HTML.
pub fn render(header: &Header, strips: &[Filmstrip]) -> String {
    render_with(header, strips, None)
}

fn render_with(header: &Header, strips: &[Filmstrip], reviewed: Option<&Reviewed>) -> String {
    let mut body = String::new();
    let title = if header.title.is_empty() {
        "Storyboards"
    } else {
        header.title.as_str()
    };
    let _ = writeln!(body, "<h1>{}</h1>", escape(title));

    let mut deliveries: Vec<&str> = strips.iter().map(|s| delivery(s.run.delivery)).collect();
    deliveries.sort_unstable();
    deliveries.dedup();
    let mut renderers: Vec<&str> = strips.iter().map(|s| s.run.renderer.as_str()).collect();
    renderers.sort_unstable();
    renderers.dedup();
    let _ = writeln!(
        body,
        "<p class=\"meta\">delivery: {} · renderer: {}{}</p>",
        escape(&join_or(&deliveries, "none")),
        escape(&join_or(&renderers, "none")),
        header
            .tree_key
            .as_deref()
            .map(|key| format!(" · key {}", escape(key)))
            .unwrap_or_default(),
    );

    if let Some(reviewed) = reviewed {
        review_header(&mut body, reviewed);
    }

    let count = |wanted: &str| {
        strips
            .iter()
            .filter(|s| status_class(&s.run.status) == wanted)
            .count()
    };
    let _ = writeln!(body, "<div class=\"counts\">");
    for (class, word) in [
        ("passed", "passed"),
        ("failed", "failed"),
        ("not_covered", "not covered"),
        ("not_applicable", "not applicable"),
        ("unavailable", "unavailable"),
        ("error", "error"),
    ] {
        let n = count(class);
        if n > 0 || class == "passed" || class == "failed" {
            let _ = writeln!(body, "<span class=\"badge {class}\">{n} {word}</span>");
        }
    }
    let _ = writeln!(body, "</div>");

    for strip in strips {
        section(&mut body, strip, reviewed);
    }
    TEMPLATE
        .replace("{{title}}", &escape(title))
        .replace("{{body}}", &body)
}

fn section(body: &mut String, strip: &Filmstrip, reviewed: Option<&Reviewed>) {
    let run = &strip.run;
    let class = status_class(&run.status);
    let variant = crate::run::variant_key(&run.variant);
    let _ = writeln!(
        body,
        "<section class=\"run\" id=\"{}\"><header><h2>{}</h2>\
         <span class=\"muted\">{} · {} · seed {}</span>\
         <span class=\"badge {class}\">{}</span></header>",
        escape(&strip.dir),
        escape(&run.storyboard.name),
        escape(app(run)),
        escape(&variant),
        escape(&run.seed),
        escape(&status_word(&run.status)),
    );
    if let Some(reason) = status_reason(&run.status) {
        let _ = writeln!(body, "<p class=\"reason\">{}</p>", escape(reason));
    }
    if !run.ignored_axes.is_empty() {
        let _ = writeln!(
            body,
            "<p class=\"reason\">ignored here: {}</p>",
            escape(&run.ignored_axes.join(", "))
        );
    }
    let _ = writeln!(body, "<div class=\"steps\">");
    for step in &run.steps {
        let _ = writeln!(body, "<figure class=\"step\">");
        if let Some(outlined) = &step.outlined {
            let src = format!("{}/{}", strip.dir, outlined);
            let _ = writeln!(
                body,
                "<a href=\"{0}\"><img src=\"{0}\" alt=\"step {1}\" loading=\"lazy\"></a>",
                escape(&src),
                step.step
            );
        }
        let input = step
            .input
            .as_ref()
            .map(|input| match &input.chord {
                Some(chord) => format!("{} → {chord}", input.step),
                None => input.step.clone(),
            })
            .unwrap_or_else(|| "starting state".to_owned());
        let _ = writeln!(
            body,
            "<figcaption><strong>{}</strong> <span class=\"input\">{}</span> {} {}",
            step.step,
            escape(&input),
            outcome_badge(&step.outcome),
            settle_badge(&step.settle),
        );
        if let Some(expect) = &step.expect {
            let _ = writeln!(body, "<div class=\"muted\">{}</div>", escape(expect));
        }
        if !step.checks.is_empty() {
            let _ = writeln!(body, "<ul class=\"checks\">");
            for check in &step.checks {
                let (class, mark) = match check.outcome {
                    Outcome::Pass => ("pass", "✓"),
                    Outcome::Fail => ("fail", "✗"),
                    Outcome::NotApplicable => ("not_applicable", "–"),
                };
                let observed = check
                    .observed
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "absent".to_owned());
                let _ = writeln!(
                    body,
                    "<li class=\"{class}\">{mark} {} {} (saw {})</li>",
                    escape(&check.path),
                    escape(&check.expected),
                    escape(&observed),
                );
            }
            let _ = writeln!(body, "</ul>");
        }
        let _ = writeln!(
            body,
            "<details><summary>where things were</summary>{}</details>",
            observation_table(&step.observation)
        );
        if let Some(Reviewed {
            state: ReviewState::Complete(review),
            frame_prefix,
            ..
        }) = reviewed
        {
            let label = crate::bundle::step_label(step);
            let cited = |sb: &str, st: &str, app_: &str, var: &str| {
                sb == run.storyboard.name && st == label && app_ == app(run) && var == variant
            };
            for v in &review.verdicts {
                if cited(&v.storyboard, &v.step, &v.app, &v.variant) {
                    let _ = writeln!(body, "{}", verdict_html(v, frame_prefix));
                }
            }
            for f in &review.findings {
                if cited(&f.storyboard, &f.step, &f.app, &f.variant) {
                    let _ = writeln!(body, "{}", finding_html(f, frame_prefix));
                }
            }
        }
        let _ = writeln!(body, "</figcaption></figure>");
    }
    let _ = writeln!(body, "</div></section>");
}

fn frame_link(frame: &str, prefix: &str) -> String {
    let href = if prefix.is_empty() {
        frame.to_owned()
    } else {
        format!("{prefix}/{frame}")
    };
    format!("<a href=\"{}\">frame</a>", escape(&href))
}

fn severity_word(severity: Option<verdicts::Severity>) -> &'static str {
    match severity {
        Some(verdicts::Severity::Blocker) => "blocker",
        Some(verdicts::Severity::Wrong) => "wrong",
        Some(verdicts::Severity::Polish) => "polish",
        None => "",
    }
}

fn resolution_html(resolution: &Resolution) -> String {
    match resolution {
        Resolution::Open => String::new(),
        Resolution::Fixed { rerun } => {
            format!(
                " <span class=\"badge pass\">fixed: {}</span>",
                escape(rerun)
            )
        }
        Resolution::Contested { reason } => format!(
            " <div class=\"contest\"><strong>contested:</strong> {}</div>",
            escape(reason)
        ),
    }
}

fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Pass => "pass",
        Kind::Fail => "fail",
        Kind::Question => "question",
    }
}

fn verdict_html(v: &Verdict, prefix: &str) -> String {
    let kind = kind_word(v.verdict);
    format!(
        "<div class=\"verdict {kind}\"><strong>{kind}</strong> \
         <span class=\"badge\">{}</span> {} <span class=\"muted\">{} · {}</span>{}</div>",
        severity_word(v.severity),
        escape(&v.says),
        escape(&v.rule),
        frame_link(&v.frame, prefix),
        resolution_html(&v.resolution),
    )
}

fn finding_html(f: &Finding, prefix: &str) -> String {
    format!(
        "<div class=\"verdict finding\"><strong>finding</strong> \
         <span class=\"badge\">{}</span> {} <span class=\"muted\">{} · {}</span>{}</div>",
        severity_word(f.severity),
        escape(&f.says),
        escape(&f.rule),
        frame_link(&f.frame, prefix),
        resolution_html(&f.resolution),
    )
}

/// One thing for the maintainer: a contest or a question.
struct Ask<'a> {
    citation: String,
    kind: &'static str,
    says: &'a str,
    reason: Option<&'a str>,
    frame: &'a str,
}

/// The reason a verdict or finding is contested, if it is.
fn contested(resolution: &Resolution) -> Option<&str> {
    match resolution {
        Resolution::Contested { reason } => Some(reason.as_str()),
        _ => None,
    }
}

/// Exactly the contested verdicts and findings and the questions: what the
/// reviewer and the implementer could not settle between them.
fn needs_you(review: &Review) -> Vec<Ask<'_>> {
    let mut asks = Vec::new();
    for v in &review.verdicts {
        let reason = contested(&v.resolution);
        if reason.is_some() || v.verdict == Kind::Question {
            asks.push(Ask {
                citation: verdicts::reference(&v.storyboard, &v.step, &v.app, &v.variant),
                kind: if reason.is_some() {
                    "contested"
                } else {
                    "question"
                },
                says: &v.says,
                reason,
                frame: &v.frame,
            });
        }
    }
    for f in &review.findings {
        if let Some(reason) = contested(&f.resolution) {
            asks.push(Ask {
                citation: verdicts::reference(&f.storyboard, &f.step, &f.app, &f.variant),
                kind: "contested finding",
                says: &f.says,
                reason: Some(reason),
                frame: &f.frame,
            });
        }
    }
    asks
}

/// The review's status line, and **Needs you**.
fn review_header(body: &mut String, reviewed: &Reviewed) {
    match &reviewed.state {
        ReviewState::NoReview => {
            let _ = writeln!(body, "<p class=\"review none\">no review ran</p>");
        }
        ReviewState::Incomplete(problems) => {
            let _ = writeln!(
                body,
                "<p class=\"review incomplete\">review incomplete: {} problem(s)</p><ul>",
                problems.len()
            );
            for problem in problems {
                let _ = writeln!(body, "<li>{}</li>", escape(problem));
            }
            let _ = writeln!(body, "</ul>");
        }
        ReviewState::Complete(review) => {
            let _ = writeln!(body, "<p class=\"review complete\">review complete</p>");
            let _ = writeln!(body, "<section id=\"needs-you\"><h2>Needs you</h2>");
            let asks = needs_you(review);
            if asks.is_empty() {
                let _ = writeln!(
                    body,
                    "<p class=\"muted\">nothing: no contests and no questions</p>"
                );
            }
            for ask in asks {
                let _ = writeln!(
                    body,
                    "<div class=\"verdict {}\"><strong>{}</strong> {} {} <span class=\"muted\">{}</span>{}</div>",
                    if ask.kind == "question" {
                        "question"
                    } else {
                        "fail"
                    },
                    escape(ask.kind),
                    escape(&ask.citation),
                    escape(ask.says),
                    frame_link(ask.frame, &reviewed.frame_prefix),
                    ask.reason
                        .map(|r| format!(
                            " <div class=\"contest\"><strong>reason:</strong> {}</div>",
                            escape(r)
                        ))
                        .unwrap_or_default(),
                );
            }
            let _ = writeln!(body, "</section>");
        }
    }
}

/// The observation as rows of `path value`, flattened from its JSON, with
/// empty fields left out so the table shows what was there.
fn observation_table(observation: &Observation) -> String {
    let mut rows = Vec::new();
    if let Ok(value) = serde_json::to_value(observation) {
        flatten("", &value, &mut rows);
    }
    let mut html = String::from("<table class=\"obs\">");
    for (path, value) in rows {
        let _ = write!(
            html,
            "<tr><td>{}</td><td>{}</td></tr>",
            escape(&path),
            escape(&value)
        );
    }
    html.push_str("</table>");
    html
}

fn flatten(prefix: &str, value: &serde_json::Value, rows: &mut Vec<(String, String)>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, inner) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&path, inner, rows);
            }
        }
        serde_json::Value::Null => {}
        serde_json::Value::String(text) => rows.push((prefix.to_owned(), text.clone())),
        other => rows.push((prefix.to_owned(), other.to_string())),
    }
}

fn outcome_badge(outcome: &StepOutcome) -> String {
    let (class, word) = match outcome {
        StepOutcome::Delivered => return String::new(),
        StepOutcome::Unbound { context } => ("fail", format!("unbound in {context}")),
        StepOutcome::Dropped => ("fail", "key dropped".to_owned()),
        StepOutcome::NothingToTypeInto => ("fail", "nothing to type into".to_owned()),
        StepOutcome::Skipped { reason } => ("not_applicable", format!("skipped: {reason}")),
        StepOutcome::NotCovered { delivery: mode } => {
            ("not_covered", format!("not covered ({})", delivery(*mode)))
        }
    };
    format!("<span class=\"badge {class}\">{}</span>", escape(&word))
}

fn settle_badge(settle: &Settle) -> String {
    let (class, word) = match settle {
        Settle::Settled { .. } | Settle::NotSampled => return String::new(),
        Settle::Jumped { .. } => ("jumped", "jumped".to_owned()),
        Settle::Blanked { .. } => ("blanked", "blank frame".to_owned()),
        Settle::Unsettled { ms } => ("unsettled", format!("unsettled after {ms} ms")),
    };
    format!("<span class=\"badge {class}\">{word}</span>")
}

fn status_class(status: &Status) -> &'static str {
    match status {
        Status::Passed => "passed",
        Status::Failed => "failed",
        Status::NotApplicable { .. } => "not_applicable",
        Status::NotCovered { .. } => "not_covered",
        Status::Unavailable { .. } => "unavailable",
        Status::Error { .. } => "error",
    }
}

fn status_word(status: &Status) -> String {
    status_class(status).replace('_', " ")
}

fn status_reason(status: &Status) -> Option<&str> {
    match status {
        Status::NotApplicable { reason }
        | Status::NotCovered { reason }
        | Status::Unavailable { reason } => Some(reason),
        Status::Error { message } => Some(message),
        Status::Passed | Status::Failed => None,
    }
}

fn delivery(mode: Delivery) -> &'static str {
    match mode {
        Delivery::Direct => "direct",
        Delivery::Chain => "chain",
        Delivery::Real => "real",
    }
}

fn app(run: &Run) -> &'static str {
    match run.app {
        crate::apply::App::Classic => "classic",
        crate::apply::App::Focus => "focus",
        crate::apply::App::Terminal => "terminal",
        crate::apply::App::Macos => "macos",
    }
}

fn join_or(items: &[&str], empty: &str) -> String {
    if items.is_empty() {
        empty.to_owned()
    } else {
        items.join(", ")
    }
}

/// HTML-escapes text for element content and attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::App;
    use crate::check::CheckResult;
    use crate::run::{Delivered, Frame, Played, RunWriter, StepRun};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn observation(region: &str) -> Observation {
        serde_json::from_value(json!({
            "window": "open",
            "view": "list",
            "scope": "Inbox",
            "keyboard": { "region": region, "field": null, "typing": false, "reachable": true, "widget": "w" },
            "cursor": { "index": 1, "id": "7", "subject": "Quarterly <draft>" },
            "rows": { "first_visible": 0, "count": 11 },
            "selection": { "count": 0 },
            "overlay": { "kind": "none", "mode": null },
            "notice": { "text": null, "tone": null, "undo": false },
            "banner": { "title": null },
            "reading": { "id": null, "focused": null, "scroll": null },
            "composer": { "open": false, "detached": false },
            "back_depth": null
        }))
        .expect("fixture")
    }

    fn run(name: &str, status: Status, steps: Vec<StepRun>) -> Run {
        Run {
            storyboard: Played {
                name: name.into(),
                hash: "h".into(),
            },
            app: App::Classic,
            variant: BTreeMap::new(),
            ignored_axes: vec![],
            tree_key: "key".into(),
            commit: "c".into(),
            delivery: Delivery::Chain,
            renderer: "cairo".into(),
            stride: 2,
            seed: "small".into(),
            preset: None,
            steps,
            status,
        }
    }

    fn step(n: usize, settle: Settle, checks: Vec<CheckResult>) -> StepRun {
        StepRun {
            step: n,
            id: None,
            input: (n > 0).then(|| Delivered {
                step: "command archive".into(),
                chord: Some("a".into()),
                context: Some("list".into()),
            }),
            expect: (n > 0).then(|| "The row below takes its place <not the top>".into()),
            outcome: StepOutcome::Delivered,
            observation: observation("list"),
            checks,
            settle,
            frame: Some(Frame {
                path: RunWriter::frame(n),
                hash: "h".into(),
            }),
            outlined: Some(RunWriter::outlined(n)),
        }
    }

    fn tree() -> (tempfile::TempDir, Vec<Filmstrip>) {
        let out = tempfile::tempdir().expect("temp");
        let root = out.path().join("runs");
        let passed = run(
            "archive-walks-down",
            Status::Passed,
            vec![
                step(0, Settle::Settled { ms: 100 }, vec![]),
                step(
                    1,
                    Settle::Jumped {
                        frames: vec!["01.s1.png".into()],
                    },
                    vec![CheckResult {
                        path: "cursor.index".into(),
                        expected: "= 1".into(),
                        observed: Some(json!(1)),
                        outcome: Outcome::Pass,
                    }],
                ),
            ],
        );
        let uncovered = run(
            "tab-cycles-panes",
            Status::NotCovered {
                reason: "step 1 needs real input".into(),
            },
            vec![step(0, Settle::Settled { ms: 100 }, vec![])],
        );
        for r in [&passed, &uncovered] {
            RunWriter::new(&root, App::Classic, &r.storyboard.name, &BTreeMap::new())
                .expect("writer")
                .write(r)
                .expect("written");
        }
        let strips = collect(&root, "runs").expect("collected");
        (out, strips)
    }

    #[test]
    fn collect_finds_every_run_with_its_directory_relative_to_the_page() {
        let (_out, strips) = tree();
        let dirs: Vec<_> = strips.iter().map(|s| s.dir.as_str()).collect();
        assert_eq!(
            dirs,
            [
                "runs/classic/archive-walks-down/default",
                "runs/classic/tab-cycles-panes/default"
            ]
        );
    }

    #[test]
    fn every_step_shows_its_outlined_frame_by_relative_path() {
        let (_out, strips) = tree();
        let html = render(&Header::default(), &strips);
        assert!(
            html.contains("<img src=\"runs/classic/archive-walks-down/default/00.outlined.png\"")
        );
        assert!(
            html.contains("<img src=\"runs/classic/archive-walks-down/default/01.outlined.png\"")
        );
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "nothing is fetched"
        );
    }

    #[test]
    fn the_observation_checks_and_settle_flags_are_shown() {
        let (_out, strips) = tree();
        let html = render(&Header::default(), &strips);
        assert!(html.contains("<td>keyboard.region</td><td>list</td>"));
        assert!(html.contains("cursor.index = 1 (saw 1)"));
        assert!(html.contains(">jumped<"));
        assert!(html.contains("command archive → a"));
    }

    #[test]
    fn the_header_names_the_delivery_mode_and_renderer() {
        let (_out, strips) = tree();
        let html = render(
            &Header {
                title: "feature/storyboards".into(),
                tree_key: Some("abc123".into()),
            },
            &strips,
        );
        assert!(html.contains("delivery: chain"));
        assert!(html.contains("renderer: cairo"));
        assert!(html.contains("key abc123"));
    }

    #[test]
    fn not_covered_runs_are_shown_and_counted_never_as_passed() {
        let (_out, strips) = tree();
        let html = render(&Header::default(), &strips);
        assert!(html.contains(">1 passed<"), "only one run passed");
        assert!(html.contains(">1 not covered<"));
        assert!(html.contains("step 1 needs real input"));
    }

    #[test]
    fn text_from_runs_is_escaped() {
        let (_out, strips) = tree();
        let html = render(&Header::default(), &strips);
        assert!(html.contains("&lt;not the top&gt;"));
        assert!(html.contains("Quarterly &lt;draft&gt;"));
        assert!(!html.contains("<not the top>"));
    }

    // ---- the review sections (T055) ----

    mod reviewed {
        use super::*;
        use crate::bundle::{self, Classification, Inputs};
        use crate::fixtures;

        fn verdict(
            step: &str,
            kind: &str,
            severity: Option<&str>,
            says: &str,
        ) -> serde_json::Value {
            json!({
                "storyboard": "archive-walks-down", "step": step, "app": "classic",
                "variant": "default",
                "frame": format!("runs/classic/archive-walks-down/default/0{step}.outlined.png"),
                "verdict": kind, "severity": severity, "says": says, "rule": "ux-architect §2"
            })
        }

        fn verdicts_json() -> serde_json::Value {
            json!({
                "bundle": { "tree_key": "treekey", "base": null },
                "reviewer": { "agent": "ux-reviewer", "model": "m", "template": "t" },
                "verdicts": [
                    verdict("0", "pass", None, "says-pass"),
                    verdict("1", "fail", Some("wrong"), "says-contested"),
                    verdict("2", "question", None, "says-question"),
                    verdict("3", "fail", Some("blocker"), "says-openfail"),
                ],
                "findings": [{
                    "storyboard": "archive-walks-down", "step": "0", "app": "classic",
                    "variant": "default",
                    "frame": "runs/classic/archive-walks-down/default/00.outlined.png",
                    "severity": "polish", "says": "says-polish", "rule": "canvas 01"
                }]
            })
        }

        /// Two runs; the second is unchanged and not covered.
        fn bundled(
            verdicts: Option<serde_json::Value>,
            contests: bool,
        ) -> (tempfile::TempDir, Vec<Filmstrip>, Reviewed) {
            let dir = tempfile::tempdir().expect("temp");
            let runs = dir.path().join("runs");
            fixtures::write(
                &runs,
                &fixtures::run("archive-walks-down", App::Classic, &[], 4),
            );
            let mut quiet = fixtures::run("tab-cycles-panes", App::Classic, &[], 1);
            quiet.status = Status::NotCovered {
                reason: "step 1 needs real input".into(),
            };
            fixtures::write(&runs, &quiet);
            let catalogue = dir.path().join("storyboards/list");
            std::fs::create_dir_all(&catalogue).expect("dir");
            for name in ["archive-walks-down", "tab-cycles-panes"] {
                std::fs::write(
                    catalogue.join(format!("{name}.toml")),
                    fixtures::storyboard(None),
                )
                .expect("board");
            }
            std::fs::write(dir.path().join("acc.md"), "acceptance").expect("acc");
            let bundle_dir = dir.path().join("bundle");
            let classify = |strip: &Filmstrip| Classification {
                class: if strip.run.storyboard.name == "tab-cycles-panes" {
                    Class::Unchanged
                } else {
                    Class::New
                },
                changed_steps: None,
            };
            bundle::build(
                &Inputs {
                    runs: &runs,
                    base: None,
                    base_sha: None,
                    acceptance: &dir.path().join("acc.md"),
                    catalogue: &dir.path().join("storyboards"),
                    design_dirs: &[],
                    out: &bundle_dir,
                },
                &classify,
            )
            .expect("bundle");
            if let Some(verdicts) = verdicts {
                std::fs::write(bundle_dir.join("verdicts.json"), verdicts.to_string())
                    .expect("verdicts");
            }
            if contests {
                std::fs::write(
                    bundle_dir.join("contests.toml"),
                    "[[contest]]\nref = \"archive-walks-down/1/classic/default\"\nreason = \"reason-contest\"\n",
                )
                .expect("contests");
            }
            let strips = collect(&runs, "runs").expect("collected");
            let reviewed = load_review(&bundle_dir, "bundle").expect("loaded");
            (dir, strips, reviewed)
        }

        fn needs_you(html: &str) -> &str {
            let start = html
                .find("<section id=\"needs-you\"")
                .expect("a Needs you section");
            let end = start + html[start..].find("</section>").expect("closed");
            &html[start..end]
        }

        #[test]
        fn verdicts_render_beside_their_frames_and_link_to_them() {
            let (_dir, strips, reviewed) = bundled(Some(verdicts_json()), true);
            let html = render_reviewed(&Header::default(), &strips, &reviewed);
            let section = html
                .split("<figure class=\"step\">")
                .find(|figure| figure.contains("says-pass"))
                .expect("the pass verdict is inside a step's figure");
            assert!(section.contains("00.outlined.png"), "beside step 0's frame");
            assert!(
                html.contains(
                    "href=\"bundle/runs/classic/archive-walks-down/default/00.outlined.png\""
                ),
                "each verdict links to its frame"
            );
            assert!(html.contains("says-polish"), "findings are shown too");
            assert!(html.contains("ux-architect §2"), "with the rule");
        }

        #[test]
        fn needs_you_holds_exactly_the_contests_and_the_questions() {
            let (_dir, strips, reviewed) = bundled(Some(verdicts_json()), true);
            let html = render_reviewed(&Header::default(), &strips, &reviewed);
            let needs = needs_you(&html);
            assert!(needs.contains("says-contested") && needs.contains("reason-contest"));
            assert!(needs.contains("says-question"));
            for left_out in ["says-pass", "says-openfail", "says-polish"] {
                assert!(
                    !needs.contains(left_out),
                    "{left_out} is not for the maintainer yet"
                );
            }
            let first = html.find("id=\"needs-you\"").expect("section");
            let first_run = html.find("class=\"run\"").expect("a run");
            assert!(first < first_run, "Needs you comes before the runs");
        }

        #[test]
        fn with_nothing_for_the_maintainer_needs_you_says_so() {
            let mut value = verdicts_json();
            value["verdicts"][2]["verdict"] = json!("pass");
            let (_dir, strips, reviewed) = bundled(Some(value), false);
            let html = render_reviewed(&Header::default(), &strips, &reviewed);
            assert!(needs_you(&html).contains("nothing"), "{}", needs_you(&html));
        }

        #[test]
        fn a_bundle_with_no_verdicts_says_no_review_ran() {
            let (_dir, strips, reviewed) = bundled(None, false);
            assert!(matches!(reviewed.state, ReviewState::NoReview));
            let html = render_reviewed(&Header::default(), &strips, &reviewed);
            assert!(html.contains("no review ran"));
            assert!(!html.contains("review complete"));
            assert!(summary(&Header::default(), &strips, &reviewed).contains("no review ran"));
        }

        #[test]
        fn a_review_that_fails_its_check_says_incomplete_and_lists_why() {
            let mut value = verdicts_json();
            value["verdicts"][0]["says"] = json!("");
            let (_dir, strips, reviewed) = bundled(Some(value), false);
            assert!(matches!(reviewed.state, ReviewState::Incomplete(_)));
            let html = render_reviewed(&Header::default(), &strips, &reviewed);
            assert!(html.contains("review incomplete"));
            assert!(html.contains("`says` is empty"), "the reason is shown");
            assert!(
                !html.contains("says-pass"),
                "an unchecked review is not shown"
            );
            assert!(summary(&Header::default(), &strips, &reviewed).contains("review incomplete"));
        }

        #[test]
        fn a_complete_review_says_so() {
            let (_dir, strips, reviewed) = bundled(Some(verdicts_json()), true);
            let html = render_reviewed(&Header::default(), &strips, &reviewed);
            assert!(html.contains("review complete"));
        }

        #[test]
        fn the_summary_leads_with_the_key_and_holds_no_image() {
            let (_dir, strips, reviewed) = bundled(Some(verdicts_json()), true);
            let header = Header {
                title: "branch".into(),
                tree_key: Some("treekey".into()),
            };
            let text = summary(&header, &strips, &reviewed);
            assert_eq!(text.lines().next(), Some("storyboards-key: treekey"));
            for image in ["![", "<img", ".png", ".jpg"] {
                assert!(!text.contains(image), "{image} in\n{text}");
            }
            assert!(text.contains("review complete"));
        }

        #[test]
        fn the_summary_names_what_needs_the_maintainer_and_counts_the_rest() {
            let (_dir, strips, reviewed) = bundled(Some(verdicts_json()), true);
            let text = summary(&Header::default(), &strips, &reviewed);
            let needs = text
                .split("## Needs you")
                .nth(1)
                .and_then(|rest| rest.split("\n## ").next())
                .expect("a Needs you section");
            assert!(needs.contains("archive-walks-down/1/classic/default"));
            assert!(needs.contains("reason-contest"));
            assert!(needs.contains("archive-walks-down/2/classic/default"));
            assert!(!needs.contains("says-openfail"));
            assert!(
                text.contains("archive-walks-down (new): 1 pass, 2 fail, 1 question"),
                "{text}"
            );
            assert!(text.contains("unchanged: 1"), "{text}");
            assert!(text.contains("not covered: 1"), "{text}");
            assert!(text.contains("not applicable: 0"), "{text}");
        }

        #[test]
        fn the_summary_takes_the_key_from_the_bundle_when_the_header_has_none() {
            let (_dir, strips, reviewed) = bundled(None, false);
            let text = summary(&Header::default(), &strips, &reviewed);
            assert_eq!(text.lines().next(), Some("storyboards-key: treekey"));
        }
    }
}
