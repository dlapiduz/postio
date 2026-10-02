//! The reader's `--r-*` palette (`data/reader-tokens.css`, in this crate's
//! own data directory since #799) is a build artefact of the same `Tokens`
//! pipeline `tests/tokens.rs` checks — see `postio_ui::tokens::generate_reader`.
//! These tests are what stops it drifting. The rest check `data/reader.css`
//! itself — the hand-authored consumer stylesheet — which holds only
//! structure, referencing the generated `--r-*` palette rather than restating
//! a colour by hand (#296).
//!
//! No display to guard: this crate has no toolkit dependency at all.

use std::path::PathBuf;

use postio_ui::tokens::{self, Tokens};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The same discovery `build.rs` does, so the two cannot disagree.
fn design_system() -> Option<PathBuf> {
    let ds = manifest_dir()
        .parent()?
        .parent()?
        .join("Design")
        .join("_ds");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(ds)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("industry-"))
                && p.join("styles.css").exists()
        })
        .collect();
    candidates.sort();
    candidates.pop().map(|p| p.join("styles.css"))
}

fn source_tokens() -> (PathBuf, Tokens) {
    let path = design_system().expect(
        "the Industry design system is missing from Design/_ds — \
         the generated reader tokens cannot be checked against their source",
    );
    let css = std::fs::read_to_string(&path).expect("cannot read the design system stylesheet");
    let parsed = Tokens::parse(&css).expect("cannot parse the design system's :root block");
    (path, parsed)
}

fn label(path: &std::path::Path) -> String {
    let parts: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let i = parts.iter().position(|p| p == "Design").unwrap();
    parts[i..].join("/")
}

fn generated() -> String {
    std::fs::read_to_string(manifest_dir().join("data").join("reader-tokens.css"))
        .expect("data/reader-tokens.css is missing; run `cargo build -p postio-widgets`")
}

/// The checked-in sheet must be exactly what the generator produces from the
/// design system as it stands. CI runs this, so a hand edit to
/// `reader-tokens.css` — or a retuned design system nobody rebuilt — fails
/// the build.
#[test]
fn generated_reader_tokens_are_reproducible() {
    let (path, parsed) = source_tokens();
    let expected = tokens::generate_reader(&parsed, &label(&path)).expect("generation failed");
    let actual = generated();
    assert_eq!(
        expected, actual,
        "data/reader-tokens.css is stale. Run `cargo build -p postio-widgets` and commit the result."
    );
}

/// The whole point of the build step: retune the source and the reader
/// follows, the same as the GTK chrome already does.
#[test]
fn retuning_the_design_system_changes_the_generated_reader_css() {
    let (path, mut parsed) = source_tokens();
    let before = tokens::generate_reader(&parsed, &label(&path)).unwrap();
    assert!(
        before.contains("#5980a6"),
        "the light-scheme steel accent should be there"
    );

    parsed.set("color-accent", "#ff0000");
    let after = tokens::generate_reader(&parsed, &label(&path)).unwrap();

    assert!(after.contains("--r-accent: #ff0000;"));
    assert!(!after.contains("#5980a6"), "the old accent should be gone");
}

fn reader_css() -> String {
    std::fs::read_to_string(manifest_dir().join("data").join("reader.css"))
        .expect("data/reader.css is missing")
}

/// The dark scheme is a real `@media` query — WebKit, unlike an
/// application-priority GTK provider, honours it — so the reader can use the
/// same mechanism the web platform gives it instead of GTK's style-class
/// workaround (see `postio_ui::tokens`'s module docs for why GTK needs one).
#[test]
fn the_dark_scheme_is_a_prefers_color_scheme_media_query() {
    let css = generated();
    assert!(css.contains("@media (prefers-color-scheme: dark)"));
    let dark = css
        .split("@media (prefers-color-scheme: dark)")
        .nth(1)
        .expect("no dark block");
    assert!(dark.contains("--r-ground:"));
    assert!(dark.contains("--r-accent:"));
}

/// #296's acceptance criterion: `reader.css` keeps only structure. Every
/// colour it used to restate by hand now lives in the generated file above,
/// referenced through `var(--r-*)`. Comments may still cite an issue number
/// (`#296`), so look at the rules only, the way `tests/tokens.rs` does.
#[test]
fn reader_css_has_no_colour_literal_tokens_rs_also_computes() {
    let css = strip_comments(&reader_css());
    assert!(
        !css.contains('#'),
        "reader.css should reference var(--r-*), not a hex colour literal: {css}"
    );
    assert!(
        !css.contains("rgba("),
        "reader.css should reference var(--r-*), not a literal rgba(): {css}"
    );
}

/// Every `var(--r-*)` `reader.css` uses must be defined in the generated
/// palette. A typo in a role name would otherwise silently drop a
/// declaration at runtime — the same check `tests/tokens.rs` runs for the
/// GTK sheet.
#[test]
fn every_r_variable_reader_css_uses_is_defined() {
    let palette = generated();
    let defined: Vec<String> = palette
        .lines()
        .filter_map(|l| l.trim().strip_prefix("--r-"))
        .filter_map(|l| l.split(':').next())
        .map(|n| format!("--r-{}", n.trim()))
        .collect();

    let css = strip_comments(&reader_css());
    let mut rest = css.as_str();
    while let Some(i) = rest.find("var(--r-") {
        rest = &rest[i + 4..];
        let end = rest.find(')').expect("unterminated var()");
        let name = rest[..end].trim().to_string();
        assert!(
            defined.contains(&name),
            "`{name}` is used in reader.css but never defined in reader-tokens.css"
        );
        rest = &rest[end..];
    }
}

/// #323's acceptance: the message body renders inside a bounded surface,
/// with an edge that gains weight under `prefers-contrast: more` rather than
/// disappearing — the same "hairlines carry meaning" rule tokens.css follows.
#[test]
fn the_body_has_a_bounded_container_with_a_high_contrast_edge() {
    let css = reader_css();
    assert!(
        css.contains(".postio-body {"),
        "reader.css should define the body's container"
    );
    assert!(css.contains("border: 1px solid var(--r-hairline)"));
    assert!(css.contains("border-radius: var(--r-radius)"));
    assert!(css.contains("@media (prefers-contrast: more)"));
    assert!(css.contains("var(--r-hairline-strong)"));
}

/// Drop `/* … */` so a check can look at the rules rather than the
/// commentary — comments legitimately cite an issue number like `#296`.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i..].find("*/") {
            Some(end) => rest = &rest[i + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The dark reader is designed, not derived (spec 006 FR-016, #1588): the
/// generated dark block carries the design system's named reader roles,
/// and its ground is no longer the chrome's `neutral-900`.
#[test]
fn the_dark_reader_roles_are_the_designed_ones() {
    let generated = generated();
    let dark = generated
        .split("@media (prefers-color-scheme: dark)")
        .nth(1)
        .expect("a dark block");
    for (role, value) in [
        ("--r-ground", "#2e2e31"),
        ("--r-ink", "#f5f5f8"),
        ("--r-ink-secondary", "#e7e7ea"),
        ("--r-dim", "#b7b7ba"),
        ("--r-hairline", "#424244"),
        ("--r-hairline-strong", "#5d5d60"),
        ("--r-accent", "#94bce3"),
    ] {
        assert!(
            dark.contains(&format!("{role}: {value};")),
            "{role} is not {value} in dark:\n{dark}"
        );
    }
    assert!(
        !dark.contains("--r-ground: #2b2b2d;"),
        "the ground is still neutral-900"
    );
}
