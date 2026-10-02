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

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use postio_ui::observe::Observation;

use crate::check::Outcome;
use crate::run::{Delivery, Run, Settle, Status, StepOutcome};

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

/// The page, as HTML.
pub fn render(header: &Header, strips: &[Filmstrip]) -> String {
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
        section(&mut body, strip);
    }
    TEMPLATE
        .replace("{{title}}", &escape(title))
        .replace("{{body}}", &body)
}

fn section(body: &mut String, strip: &Filmstrip) {
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
        let _ = writeln!(body, "</figcaption></figure>");
    }
    let _ = writeln!(body, "</div></section>");
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
}
