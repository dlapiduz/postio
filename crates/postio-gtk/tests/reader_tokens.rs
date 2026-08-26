//! The reader's `--r-*` palette (`data/reader-tokens.css`) is a build
//! artefact of the same `Tokens` pipeline `tests/tokens.rs` checks — see
//! `crates/postio-gtk/src/tokens.rs::generate_reader`. These tests are what
//! stops it drifting, and what stops `data/reader.css` regressing back into
//! restating colours by hand (#296).
//!
//! No GTK here — see `gtk_reader.rs` for the checks that need a display.

use std::path::PathBuf;

use postio_gtk::tokens::{self, Tokens};

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
        .expect("data/reader-tokens.css is missing; run `cargo build -p postio-gtk`")
}

fn reader_css() -> String {
    std::fs::read_to_string(manifest_dir().join("data").join("reader.css"))
        .expect("data/reader.css is missing")
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
        "data/reader-tokens.css is stale. Run `cargo build -p postio-gtk` and commit the result."
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

/// The dark scheme is a real `@media` query — WebKit, unlike an
/// application-priority GTK provider, honours it — so the reader can use the
/// same mechanism the web platform gives it instead of GTK's style-class
/// workaround (see `tokens.rs`'s module docs for why GTK needs one).
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
