//! WCAG contrast floors for the de-emphasised ink roles, light and dark
//! (#829, #895).
//!
//! The default theme shipped `--postio-dim` and `--postio-faint` chosen by
//! eye rather than measured, and both failed WCAG 2.2 against the row
//! background a person actually reads them on. The maintainer's decision on
//! #829 turned "three shades of grey" into two token classes, each with a
//! floor it must clear:
//!
//! - `--postio-dim` carries **content** — the list preview line and the
//!   timestamp, text a person reads to triage mail — and must clear
//!   **4.5:1** (WCAG 2.2 SC 1.4.3, normal text).
//! - `--postio-faint` carries an **affordance** — the key hint on the
//!   focused row, which is also taught by the cheat sheet and the palette —
//!   and must clear the lower **3:1** floor (SC 1.4.11, non-text/UI
//!   components).
//!
//! Measured against `#ffffff`, the row background a list row actually
//! renders against (`--postio-ground`/`--view-bg-color` are the *pane*
//! background under `.postio-list`; an individual unselected row falls
//! through to the list view's own white background, which is what #829's
//! screenshot measurement confirmed empirically).
//!
//! **Dark was measured second (#895), and passes as it stands.** It reaches
//! the same two roles by a different mechanism — palette steps
//! (`--postio-color-neutral-400`/`-500`) rather than an alpha tint of
//! `color-text` — so nothing about the light fix implied anything about it,
//! which is exactly why it needed its own measurement rather than an
//! assumption. Against `#2b2b2d`, the dark row background:
//!
//! ```text
//! --postio-dim    neutral-400  #b7b7ba   7.06:1   floor 4.5   pass
//! --postio-faint  neutral-500  #98989b   4.91:1   floor 3.0   pass
//! ```
//!
//! Both clear comfortably, and `dim` out-contrasts `faint` as it must, so no
//! retune was needed. The tests below are what stops that drifting the way
//! light did — the numbers were fine until somebody changed a palette step.
//!
//! High contrast already clears both floors comfortably in both schemes and
//! is not asserted here.
//!
//! No display to guard: this is arithmetic over two generated tokens.

use std::path::PathBuf;

use postio_ui::tokens::{self, Tokens};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn design_system() -> PathBuf {
    let ds = manifest_dir()
        .parent()
        .expect("crates/postio-ui")
        .parent()
        .expect("crates")
        .join("Design")
        .join("_ds");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&ds)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", ds.display()))
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
    candidates
        .pop()
        .map(|p| p.join("styles.css"))
        .expect("no industry-* design system found under Design/_ds")
}

fn light_tokens() -> String {
    let source = design_system();
    let css = std::fs::read_to_string(&source).expect("cannot read the design system stylesheet");
    let parsed = Tokens::parse(&css).expect("cannot parse the design system's :root block");
    tokens::generate(&parsed, "test").expect("generation failed")
}

/// Pull `--role: rgba(r, g, b, a);` (or a plain `#rrggbb`) for `role` out of
/// the **first** `:root { … }` block in `css` — the light, normal-contrast
/// scheme, which is the only one this file checks.
fn light_role_color(css: &str, role: &str) -> (u8, u8, u8, f32) {
    // split[0] is everything before the first `:root {`; split[1] is the raw
    // industry-tokens block (`--postio-color-*`); split[2] is the roles-light
    // block this function reads. `:root.postio-dark {` and `:root.postio-hc {`
    // do not match the plain `:root {` needle, so they never add a split.
    let block = css
        .split(":root {")
        .nth(2)
        .expect("no light roles :root block")
        .split("}\n")
        .next()
        .unwrap();
    let needle = format!("{role}: ");
    let line = block
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with(&needle))
        .unwrap_or_else(|| panic!("`{role}` is not defined in the light scheme"));
    let value = line
        .trim_start_matches(&needle)
        .trim_end_matches(';')
        .trim();
    parse_color(value)
}

fn parse_color(value: &str) -> (u8, u8, u8, f32) {
    if let Some(hex) = value.strip_prefix('#') {
        let v = u32::from_str_radix(hex, 16).expect("not a hex colour");
        return (
            ((v >> 16) & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            (v & 0xff) as u8,
            1.0,
        );
    }
    let inner = value
        .strip_prefix("rgba(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or_else(|| panic!("`{value}` is neither a hex colour nor rgba()"));
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    let channel = |i: usize| parts[i].parse::<f32>().expect("bad channel");
    (
        channel(0) as u8,
        channel(1) as u8,
        channel(2) as u8,
        parts[3].parse::<f32>().expect("bad alpha"),
    )
}

/// `rgba(r, g, b, a)` painted over an opaque `bg`, standard alpha-over —
/// what a compositor actually draws, matching how GTK renders `color:` over
/// a solid background.
fn composite(fg: (u8, u8, u8, f32), bg: (u8, u8, u8)) -> (f64, f64, f64) {
    let a = fg.3 as f64;
    let mix = |f: u8, b: u8| f64::from(b) * (1.0 - a) + f64::from(f) * a;
    (mix(fg.0, bg.0), mix(fg.1, bg.1), mix(fg.2, bg.2))
}

/// WCAG relative luminance: sRGB channels linearised, then the standard
/// 0.2126/0.7152/0.0722 weights. Not the same as the crude weighted sum
/// `tests/tokens.rs` uses for a same-direction "lighter than" comparison —
/// a contrast *ratio* needs the gamma-correct value.
fn relative_luminance((r, g, b): (f64, f64, f64)) -> f64 {
    let lin = |c: f64| {
        let c = c / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// WCAG 2.x contrast ratio between two colours, order-independent.
fn contrast_ratio(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (lighter, darker) = if la > lb { (la, lb) } else { (lb, la) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Pull `--role` out of the dark scheme, following one level of `var()`.
///
/// Dark states its roles as palette references (`var(--postio-color-neutral-400)`)
/// where light states literal `rgba()`, so this resolves the reference against
/// the palette block rather than assuming a colour is written where the role
/// is. That difference is the whole reason #895 existed: measuring light said
/// nothing about dark, because the two do not share a mechanism.
fn dark_role_color(css: &str, role: &str) -> (u8, u8, u8, f32) {
    // `:root.postio-dark {` matches once. The high-contrast block is
    // `:root.postio-dark.postio-hc {`, which does not contain this needle
    // because of what sits between the class and the brace.
    let block = css
        .split(":root.postio-dark {")
        .nth(1)
        .expect("no dark roles block")
        .split("}\n")
        .next()
        .unwrap();
    parse_color(&resolve(css, value_of(block, role)))
}

/// The dark row background, read from the same block rather than hard-coded:
/// a scheme that restyled its list would move this, and a floor measured
/// against the wrong background is worse than no floor.
fn dark_row_background(css: &str) -> (u8, u8, u8) {
    let (r, g, b, _) = dark_role_color(css, "--view-bg-color");
    (r, g, b)
}

/// The declared value of `role` inside one already-sliced block.
fn value_of(block: &str, role: &str) -> String {
    let needle = format!("{role}: ");
    block
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(&needle))
        .unwrap_or_else(|| panic!("`{role}` is not defined in this block"))
        .trim_start_matches(&needle)
        .trim_end_matches(';')
        .trim()
        .to_owned()
}

/// `var(--postio-color-x)` looked up in the palette block; anything else
/// returned as it stands.
fn resolve(css: &str, value: String) -> String {
    let Some(name) = value
        .strip_prefix("var(")
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return value;
    };
    let palette = css
        .split(":root {")
        .nth(1)
        .expect("no palette block")
        .split("}\n")
        .next()
        .unwrap();
    value_of(palette, name.trim())
}

const WHITE: (u8, u8, u8) = (255, 255, 255);

#[test]
fn dim_clears_the_content_floor_against_a_white_row() {
    let css = light_tokens();
    let dim = light_role_color(&css, "--postio-dim");
    let ratio = contrast_ratio(composite(dim, WHITE), (255.0, 255.0, 255.0));
    assert!(
        ratio >= 4.5,
        "--postio-dim is {ratio:.2}:1 against white, below the 4.5:1 WCAG \
         floor for content (the list preview line and the timestamp read it)"
    );
}

#[test]
fn faint_clears_the_affordance_floor_against_a_white_row() {
    let css = light_tokens();
    let faint = light_role_color(&css, "--postio-faint");
    let ratio = contrast_ratio(composite(faint, WHITE), (255.0, 255.0, 255.0));
    assert!(
        ratio >= 3.0,
        "--postio-faint is {ratio:.2}:1 against white, below the 3:1 WCAG \
         floor for a UI affordance (the focused row's key hint reads it)"
    );
}

/// `--postio-dim` must stay strictly the darker of the two: content needs to
/// out-rank a redundant affordance for a reader who cannot use colour alone
/// to tell them apart, and a generator change that quietly swapped the two
/// values would still pass the two floor checks above on their own.
#[test]
fn dim_is_darker_than_faint() {
    let css = light_tokens();
    let dim = light_role_color(&css, "--postio-dim");
    let faint = light_role_color(&css, "--postio-faint");
    let dim_ratio = contrast_ratio(composite(dim, WHITE), (255.0, 255.0, 255.0));
    let faint_ratio = contrast_ratio(composite(faint, WHITE), (255.0, 255.0, 255.0));
    assert!(
        dim_ratio > faint_ratio,
        "--postio-dim ({dim_ratio:.2}:1) should out-contrast --postio-faint \
         ({faint_ratio:.2}:1) — content should read more clearly than an \
         affordance, not the other way round"
    );
}

// ── The dark scheme (#895) ──────────────────────────────────────────────
//
// Same two floors, same reasoning about which role carries content and which
// carries an affordance — measured against the dark row background instead of
// the white one, and reading the roles through the palette because that is
// how dark states them.

#[test]
fn dark_dim_clears_the_content_floor_against_the_dark_row() {
    let css = light_tokens();
    let background = dark_row_background(&css);
    let dim = dark_role_color(&css, "--postio-dim");
    let ratio = contrast_ratio(
        composite(dim, background),
        (
            f64::from(background.0),
            f64::from(background.1),
            f64::from(background.2),
        ),
    );
    assert!(
        ratio >= 4.5,
        "dark --postio-dim is {ratio:.2}:1 against the row background, below \
         the 4.5:1 WCAG floor for content (the list preview line and the \
         timestamp read it)"
    );
}

#[test]
fn dark_faint_clears_the_affordance_floor_against_the_dark_row() {
    let css = light_tokens();
    let background = dark_row_background(&css);
    let faint = dark_role_color(&css, "--postio-faint");
    let ratio = contrast_ratio(
        composite(faint, background),
        (
            f64::from(background.0),
            f64::from(background.1),
            f64::from(background.2),
        ),
    );
    assert!(
        ratio >= 3.0,
        "dark --postio-faint is {ratio:.2}:1 against the row background, \
         below the 3:1 WCAG floor for a UI affordance (the focused row's key \
         hint reads it)"
    );
}

/// The same ordering invariant the light scheme has, for the same reason: two
/// palette steps that swapped would clear both floors above and still be
/// wrong.
#[test]
fn dark_dim_is_lighter_than_faint() {
    let css = light_tokens();
    let background = dark_row_background(&css);
    let against = (
        f64::from(background.0),
        f64::from(background.1),
        f64::from(background.2),
    );
    let dim = contrast_ratio(
        composite(dark_role_color(&css, "--postio-dim"), background),
        against,
    );
    let faint = contrast_ratio(
        composite(dark_role_color(&css, "--postio-faint"), background),
        against,
    );
    assert!(
        dim > faint,
        "dark --postio-dim ({dim:.2}:1) should out-contrast --postio-faint \
         ({faint:.2}:1) — content should read more clearly than an \
         affordance, not the other way round"
    );
}
