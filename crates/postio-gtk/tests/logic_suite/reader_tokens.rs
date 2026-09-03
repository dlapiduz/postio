//! `reader.css`'s own discipline: it holds only structure, referencing the
//! generated `--r-*` palette rather than restating a colour by hand (#296).
//! The checks that the generated palette itself faithfully reproduces the
//! design system live with the generator, in `postio-ui/tests/reader_tokens.rs`
//! (#569) — this file is what is left once those move: tests about
//! `reader.css`, the hand-authored file, not about the `Tokens` pipeline that
//! produces its palette.
//!
//! Both files live in `postio-ui`'s own data directory (#799) — the faces and
//! the reader stylesheets are `postio-ui`'s data, read here by relative path
//! across crates the same way `postio-ui`'s own tests do.
//!
//! No GTK here — see `gtk_reader.rs` for the checks that need a display.

use std::path::PathBuf;

/// `postio-ui`'s data directory, a sibling under `crates/`.
fn ui_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/postio-gtk")
        .join("postio-ui")
        .join("data")
}

fn generated() -> String {
    std::fs::read_to_string(ui_data_dir().join("reader-tokens.css"))
        .expect("postio-ui/data/reader-tokens.css is missing; run `cargo build -p postio-gtk`")
}

fn reader_css() -> String {
    std::fs::read_to_string(ui_data_dir().join("reader.css"))
        .expect("postio-ui/data/reader.css is missing")
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
