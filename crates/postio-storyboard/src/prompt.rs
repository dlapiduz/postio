//! The reviewer's prompt (`contracts/review.md` § The prompt template).
//!
//! The template is fixed and hashed into every review, and the prompt is
//! the template with the manifest filled in. It reads nothing else: no
//! acceptance, no expectation, no hint from the session that asks. The
//! reviewer reads those from the bundle, which is the point of a fresh pair
//! of eyes (FR-018).

use std::path::Path;

use crate::bundle::{Batch, Manifest};

const TEMPLATE: &str = include_str!("../templates/reviewer-prompt.md");

/// The template's blake3, which a verdicts file carries as
/// `reviewer.template`.
pub fn template_hash() -> String {
    blake3::hash(TEMPLATE.as_bytes()).to_hex().to_string()
}

/// How many prompts a manifest makes: one per batch.
pub fn count(manifest: &Manifest) -> usize {
    manifest.batches.len()
}

/// The manifest of a bundle directory.
pub fn load_manifest(bundle: &Path) -> Result<Manifest, String> {
    let path = bundle.join("manifest.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The prompt for the 1-based batch `n`, or `None` when there is none.
pub fn render(manifest: &Manifest, n: usize, bundle: &Path) -> Option<String> {
    let batch: &Batch = manifest.batches.get(n.checked_sub(1)?)?;
    let bullets = |items: Vec<String>| {
        if items.is_empty() {
            "  (none)".to_owned()
        } else {
            items.join("\n")
        }
    };
    let design = bullets(
        batch
            .design
            .iter()
            .filter(|name| manifest.design.contains(name))
            .map(|name| format!("  - `design/{name}.png`"))
            .collect(),
    );
    let runs = bullets(
        batch
            .runs
            .iter()
            .map(|run| {
                format!(
                    "  - {} ({}, {:?}): `{}/run.json`",
                    run.storyboard, run.variant, run.class, run.dir
                )
            })
            .collect(),
    );
    let steps = bullets(
        batch
            .steps
            .iter()
            .map(|s| {
                format!(
                    "  - {} / {} / {} / {} -> `{}`",
                    s.storyboard, s.step, s.app, s.variant, s.frame
                )
            })
            .collect(),
    );
    let output = if manifest.batches.len() == 1 {
        "verdicts.json".to_owned()
    } else {
        format!("verdicts.{n}.json")
    };
    // Absolute, so a reviewer started anywhere can open every file it is
    // told to read: the bundle's own paths are relative to it, and the
    // project's guidance lives in the repository the bundle sits in.
    let absolute = bundle
        .canonicalize()
        .unwrap_or_else(|_| bundle.to_path_buf());
    let repo = absolute
        .ancestors()
        .find(|dir| dir.join(".claude").is_dir())
        .map_or_else(|| ".".to_owned(), |dir| dir.display().to_string());
    let values = [
        ("batch", n.to_string()),
        ("batches", manifest.batches.len().to_string()),
        ("hash", template_hash()),
        ("bundle", absolute.display().to_string()),
        ("repo", repo),
        ("tree_key", manifest.tree_key.clone()),
        (
            "base",
            manifest.base.clone().unwrap_or_else(|| "none".into()),
        ),
        ("app", batch.app.to_string()),
        ("surface", batch.surface.clone()),
        ("design", design),
        ("runs", runs),
        ("step_count", batch.steps.len().to_string()),
        ("steps", steps),
        ("output", output),
    ];
    // One pass over the template, so a value that happens to contain
    // `{{x}}` is never expanded again.
    let mut out = String::with_capacity(TEMPLATE.len());
    let mut rest = TEMPLATE;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        match after.find("}}") {
            Some(close) => {
                let name = &after[..close];
                match values.iter().find(|(key, _)| *key == name) {
                    Some((_, value)) => out.push_str(value),
                    None => out.push_str(&rest[open..open + 2 + close + 2]),
                }
                rest = &after[close + 2..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::App;
    use crate::bundle::{self, Inputs, all_new};
    use crate::fixtures::{run, storyboard, write};
    use std::path::PathBuf;

    const SENTINEL: &str = "SENTINEL-FROM-OUTSIDE";

    fn bundled(designs: usize) -> (tempfile::TempDir, Manifest) {
        let dir = tempfile::tempdir().expect("temp");
        let runs = dir.path().join("runs");
        write(&runs, &run("archive-walks-down", App::Focus, &[], 3));
        write(&runs, &run("open-settings", App::Terminal, &[], 2));
        let catalogue = dir.path().join("storyboards");
        for (surface, name) in [("list", "archive-walks-down"), ("screens", "open-settings")] {
            std::fs::create_dir_all(catalogue.join(surface)).expect("dir");
            std::fs::write(
                catalogue.join(surface).join(format!("{name}.toml")),
                storyboard(Some("01-inbox-reading")),
            )
            .expect("board");
        }
        let screens = dir.path().join("screens");
        std::fs::create_dir_all(&screens).expect("dir");
        for name in ["01-inbox-reading", "step-screen"].iter().take(designs) {
            std::fs::write(screens.join(format!("{name}.png")), b"png").expect("png");
        }
        std::fs::write(dir.path().join("acc.md"), SENTINEL).expect("acc");
        let manifest = bundle::build(
            &Inputs {
                runs: &runs,
                base: None,
                base_sha: None,
                acceptance: &dir.path().join("acc.md"),
                catalogue: &catalogue,
                design_dirs: &[screens],
                out: &dir.path().join("bundle"),
            },
            &all_new,
        )
        .expect("bundle");
        (dir, manifest)
    }

    fn prompt(n: usize) -> String {
        let (dir, manifest) = bundled(2);
        render(&manifest, n, &dir.path().join("bundle")).expect("batch")
    }

    #[test]
    fn there_is_one_prompt_per_batch_and_none_beyond() {
        let (dir, manifest) = bundled(2);
        assert_eq!(count(&manifest), 2);
        assert!(render(&manifest, 1, dir.path()).is_some());
        assert!(render(&manifest, 2, dir.path()).is_some());
        assert!(render(&manifest, 0, dir.path()).is_none());
        assert!(render(&manifest, 3, dir.path()).is_none());
    }

    #[test]
    fn the_prompt_has_the_six_sections_of_the_contract() {
        let text = prompt(1);
        for heading in [
            "## 1. Role",
            "## 2. Read first",
            "## 3. For every step listed below",
            "## 4. Beyond the expectations",
            "## 5. How to word a finding",
            "## 6. Where to write",
        ] {
            assert!(text.contains(heading), "missing {heading}");
        }
        for needle in [
            "You are Postio's design and UX reviewer",
            ".claude/skills/ux-architect/SKILL.md",
            ".claude/skills/gtk-design/SKILL.md",
            "docs/PRODUCT.md",
            "Fail a step whose frame contradicts its `expect`",
            "Do not propose code",
            "\"verdicts\": [",
            "\"findings\": [",
        ] {
            assert!(text.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn the_prompt_lists_the_batch_and_nothing_of_the_other() {
        let text = prompt(1);
        assert!(text.contains("batch 1 of 2"));
        assert!(text.contains("App: focus"));
        assert!(text.contains("Surface: list"));
        assert!(text.contains("runs/focus/archive-walks-down/default/02.outlined.png"));
        assert!(text.contains("`design/01-inbox-reading.png`"));
        assert!(!text.contains("open-settings"), "the other batch's work");
        assert!(!text.contains("{{"), "every placeholder was filled");
    }

    #[test]
    fn a_design_screen_that_was_not_found_is_not_listed_for_reading() {
        let (dir, manifest) = bundled(0);
        let text = render(&manifest, 1, &dir.path().join("bundle")).expect("batch");
        assert!(!text.contains("design/01-inbox-reading.png"));
        assert!(text.contains("(none)"));
    }

    #[test]
    fn the_prompt_holds_no_text_from_outside_the_bundle_manifest() {
        let text = prompt(1);
        assert!(
            !text.contains(SENTINEL),
            "the acceptance is read, not pasted"
        );
        assert!(
            !text.contains("expectation 1"),
            "a step's expect is read from run.json, not pasted"
        );
    }

    #[test]
    fn the_template_hash_is_in_the_prompt_and_is_a_blake3_digest() {
        let hash = template_hash();
        assert_eq!(hash.len(), 64);
        assert!(prompt(1).contains(&format!("blake3 {hash}")));
        assert_eq!(hash, blake3::hash(TEMPLATE.as_bytes()).to_hex().to_string());
    }

    #[test]
    fn a_value_that_looks_like_a_placeholder_is_not_expanded() {
        let (_dir, mut manifest) = bundled(2);
        manifest.tree_key = "{{bundle}}".into();
        let text = render(&manifest, 1, &PathBuf::from("/b")).expect("batch");
        assert!(text.contains("Tree key: {{bundle}}"));
    }
}
