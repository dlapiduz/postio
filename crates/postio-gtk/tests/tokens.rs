//! The generated stylesheet is a build artefact of the design system. These
//! tests are what stops it drifting: they re-run the generator and compare it
//! with the copy checked in at `data/tokens.css`.
//!
//! No GTK here — see `gtk_style.rs` for the checks that need a display.

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
         the generated tokens cannot be checked against their source",
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
    std::fs::read_to_string(manifest_dir().join("data").join("tokens.css"))
        .expect("data/tokens.css is missing; run `cargo build -p postio-gtk`")
}

/// The checked-in sheet must be exactly what the generator produces from the
/// design system as it stands. CI runs this, so a hand edit to `tokens.css` —
/// or a retuned design system nobody rebuilt — fails the build.
#[test]
fn generated_tokens_are_reproducible() {
    let (path, parsed) = source_tokens();
    let expected = tokens::generate(&parsed, &label(&path)).expect("generation failed");
    let actual = generated();
    assert_eq!(
        expected, actual,
        "data/tokens.css is stale. Run `cargo build -p postio-gtk` and commit the result."
    );
}

/// The whole point of the build step: retune the source and the app follows.
#[test]
fn retuning_the_design_system_changes_the_generated_css() {
    let (path, mut parsed) = source_tokens();
    let before = tokens::generate(&parsed, &label(&path)).unwrap();
    assert!(
        before.contains("#5980a6"),
        "the steel accent should be there"
    );

    parsed.set("color-accent", "#ff0000");
    parsed.set("space-3", "99px");
    parsed.set("font-body", "\"Some Other Face\", sans-serif");
    let after = tokens::generate(&parsed, &label(&path)).unwrap();

    assert!(after.contains("--postio-color-accent: #ff0000;"));
    assert!(after.contains("--postio-space-3: 99px;"));
    assert!(after.contains("--postio-font-body: \"Some Other Face\", sans-serif;"));
    // Derived values follow too — the selected-row tint is mixed from the accent.
    assert!(
        after.contains("rgba(255, 0, 0, 0.12)"),
        "the selected-row tint should be re-derived from the new accent"
    );
    assert!(
        !after.contains("rgba(89, 128, 166, 0.12)"),
        "the old accent tint should be gone"
    );
}

/// Web-only syntax GTK would mis-parse must be folded away by the generator.
#[test]
fn generated_css_stays_inside_gtks_css_subset() {
    let css = generated();
    assert!(
        !css.contains("color-mix("),
        "color-mix() should be folded to rgba() at build time"
    );
    assert!(
        !css.contains("system-ui"),
        "`system-ui` is a web keyword; GTK would treat it as a family name"
    );
    assert!(
        !css.contains("@import"),
        "the fonts are embedded, so the sheet must not import anything"
    );
    assert!(
        !css.contains("http"),
        "nothing in the stylesheet may reference the network"
    );
    assert!(
        !css.contains("@media"),
        "GTK only honours @media in the theme provider; the schemes are classes"
    );
}

/// Keep the Industry identity, drop its wireframe chrome.
#[test]
fn wireframe_chrome_is_not_ported() {
    let css = generated();
    // The banner explains what was dropped, so look at the rules only.
    let rules = strip_comments(&css);
    for banned in ["corner", "registration", "blueprint", "duotone", "halftone"] {
        assert!(
            !rules.to_lowercase().contains(banned),
            "the {banned} treatment belongs to the design system's wireframe chrome, \
             which this app deliberately drops"
        );
    }
    // ...but the identity is intact.
    for kept in [
        "Barlow Condensed",
        "Barlow",
        "IBM Plex Mono",
        "#5980a6",
        "--postio-hairline",
    ] {
        assert!(css.contains(kept), "the design's `{kept}` should survive");
    }
}

/// Every `var()` the sheet uses must be defined by the sheet itself or by
/// libadwaita. A typo in a role name would otherwise silently drop a
/// declaration at runtime.
#[test]
fn every_referenced_variable_is_defined() {
    let css = generated();
    let defined: Vec<String> = css
        .lines()
        .filter_map(|l| l.trim().strip_prefix("--"))
        .filter_map(|l| l.split(':').next())
        .map(|n| format!("--{}", n.trim()))
        .collect();

    let mut rest = css.as_str();
    while let Some(i) = rest.find("var(") {
        rest = &rest[i + 4..];
        let end = rest.find(')').expect("unterminated var()");
        let name = rest[..end].trim().to_string();
        assert!(
            defined.contains(&name),
            "`{name}` is used but never defined in tokens.css"
        );
        rest = &rest[end..];
    }
}

/// The dark block is the canvas's dark board: the ground drops to the neutral
/// deep step, the selected row takes the accent's deep step, and the hairline
/// lifts rather than darkening.
#[test]
fn dark_scheme_lifts_the_hairlines() {
    let css = generated();
    let dark = css
        .split(":root.postio-dark {")
        .nth(1)
        .expect("no dark block")
        .split("}\n")
        .next()
        .unwrap();

    assert!(dark.contains("--postio-ground: var(--postio-color-neutral-900)"));
    assert!(dark.contains("--postio-hairline: var(--postio-color-neutral-700)"));
    assert!(dark.contains("--postio-selected-bg: var(--postio-color-accent-900)"));
    assert!(dark.contains("--postio-accent: var(--postio-color-accent-400)"));

    // neutral-700 is lighter than the neutral-900 ground it sits on: lifting,
    // not darkening.
    let (_, parsed) = source_tokens();
    let hairline = luminance(parsed.get("color-neutral-700").unwrap());
    let ground = luminance(parsed.get("color-neutral-900").unwrap());
    assert!(
        hairline > ground,
        "the dark hairline must be lighter than the dark ground"
    );

    // ...and the light one still darkens.
    let light_ground = luminance(parsed.get("color-bg").unwrap());
    let light_hairline = luminance(parsed.get("color-text").unwrap());
    assert!(light_hairline < light_ground);
}

/// Drop `/* … */` so a check can look at the rules rather than the commentary.
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

fn luminance(hex: &str) -> f32 {
    let hex = hex.trim_start_matches('#');
    let v = u32::from_str_radix(hex, 16).expect("not a hex colour");
    let (r, g, b) = ((v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff);
    0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32
}
