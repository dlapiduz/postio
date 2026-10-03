//! The generated stylesheets are build artefacts of the design system. These
//! tests are what stops them drifting: they re-run the generator and compare
//! it with the copies checked in under `postio-widgets/data/` and
//! `postio-ui/data/`, which is where the running app's GResource bundle reads
//! them from (#569).
//!
//! No display to guard: this crate has no toolkit dependency at all.

use std::path::PathBuf;

use postio_ui::tokens::{self, Tokens};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `postio-widgets`' own directory, where the metrics both desktop apps
/// share are generated (specs/007-postio-focus research R11).
fn widgets_dir() -> PathBuf {
    manifest_dir()
        .parent()
        .expect("crates/postio-ui")
        .join("postio-widgets")
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

fn generated_metrics() -> String {
    std::fs::read_to_string(widgets_dir().join("data").join("metrics.css"))
        .expect("postio-widgets/data/metrics.css is missing; run `cargo build -p postio-widgets`")
}

/// The shared metrics, the same way: the checked-in sheet is exactly what the
/// generator makes of the design system as it stands.
#[test]
fn generated_metrics_are_reproducible() {
    let (path, parsed) = source_tokens();
    let expected = tokens::generate_metrics(&parsed, &label(&path)).expect("generation failed");
    assert_eq!(
        expected,
        generated_metrics(),
        "postio-widgets/data/metrics.css is stale. Run `cargo build -p postio-widgets` and commit the result."
    );
}

/// The whole point of the build step: retune the source and the app follows.
#[test]
fn retuning_the_design_system_changes_the_generated_css() {
    let (path, mut parsed) = source_tokens();
    parsed.set("space-3", "99px");
    let metrics = tokens::generate_metrics(&parsed, &label(&path)).unwrap();
    assert!(metrics.contains("--postio-space-3: 99px;"));
}

/// Web-only syntax GTK would mis-parse must be folded away by the generator.
#[test]
fn generated_css_stays_inside_gtks_css_subset() {
    let css = generated_metrics();
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

/// Every `var()` the sheet uses must be defined by the sheet itself or by
/// libadwaita. A typo in a role name would otherwise silently drop a
/// declaration at runtime.
#[test]
fn every_referenced_variable_is_defined() {
    let css = generated_metrics();
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
            "`{name}` is used but never defined in metrics.css"
        );
        rest = &rest[end..];
    }
}

#[test]
fn the_swift_emitter_carries_the_same_values() {
    // The point of generating rather than typing: a Swift file with `#5980a6`
    // in it is a copy that is right on the day it is written. Ghostty
    // hand-writes 921 lines of the equivalent and calls it their worst
    // duplication.
    let path = design_system().expect("the design system is present");
    let css = std::fs::read_to_string(&path).expect("the design system");
    let parsed = tokens::Tokens::parse(&css).expect("parse failed");
    let swift = tokens::generate_swift(&parsed, &label(&path)).expect("generation failed");

    assert!(swift.contains("public enum PostioTokens"));
    assert!(swift.contains("import AppKit"));
    assert!(
        swift.contains("GENERATED FILE"),
        "a generated file must say so, or somebody edits it"
    );

    // Names arrive in Swift's shape, not CSS's.
    assert!(swift.contains("public static let colorBg"));
    assert!(
        !swift.contains("color-bg ="),
        "a CSS name leaked into Swift and would not compile"
    );

    // The accent is the design system's, not a colour invented here.
    let accent = parsed.get("color-accent").expect("an accent");
    assert!(
        swift.contains(&format!("`{accent}`")),
        "the emitted Swift does not cite the source value for color-accent"
    );
}

#[test]
fn the_swift_emitter_converts_rather_than_copying_css() {
    let path = design_system().expect("the design system is present");
    let css = std::fs::read_to_string(&path).expect("the design system");
    let parsed = tokens::Tokens::parse(&css).expect("parse failed");
    let swift = tokens::generate_swift(&parsed, &label(&path)).expect("generation failed");

    // Colours become NSColor, not strings a runtime would have to parse — a
    // hex string in Swift is a parse that can fail at the worst moment.
    assert!(swift.contains("NSColor(srgbRed:"));
    // Lengths become CGFloat, so layout arithmetic is arithmetic.
    assert!(swift.contains(": CGFloat ="));
    // No `px` survives: a CSS unit in Swift means something was copied.
    assert!(
        !swift.contains("px\n"),
        "a CSS length reached Swift without being converted"
    );
}

#[test]
fn every_required_token_reaches_swift() {
    // The guard the first version of the emitter needed and did not have: it
    // parsed lengths as whole pixels, the spacing ramp is `3.4px`, and it
    // emitted *nothing* for every space token. The file compiled and was
    // missing half the design system, which is the worst shape a generator
    // failure can take.
    let path = design_system().expect("the design system is present");
    let css = std::fs::read_to_string(&path).expect("the design system");
    let parsed = tokens::Tokens::parse(&css).expect("parse failed");
    let swift = tokens::generate_swift(&parsed, &label(&path)).expect("generation failed");

    for name in ["color-bg", "color-accent", "color-text", "color-divider"] {
        let swift_name = name.replace("color-", "color").replace('-', "");
        assert!(
            swift.to_lowercase().contains(&swift_name.to_lowercase()),
            "{name} did not reach Swift"
        );
    }
    for step in ["space1", "space2", "space3", "space4", "space6", "space8"] {
        assert!(
            swift.contains(&format!("let {step}:")),
            "{step} did not reach Swift; the spacing ramp is fractional and an \
             integer parse drops all of it"
        );
    }
    for radius in ["radiusSm", "radiusMd", "radiusLg"] {
        assert!(
            swift.contains(&format!("let {radius}:")),
            "{radius} did not reach Swift"
        );
    }
}

/// GTK lays widgets out in whole pixels, and Rust spacing is an `i32`, so
/// the design system's 3.4px step reaches both as the nearest whole pixel --
/// once, here, rather than as 14 hand-rounded spacings across the frontend.
#[test]
fn the_spacing_scale_is_whole_pixels() {
    let (path, parsed) = source_tokens();
    let css = tokens::generate_metrics(&parsed, &label(&path)).unwrap();
    let spaces: Vec<&str> = css
        .lines()
        .filter(|line| line.trim_start().starts_with("--postio-space-"))
        .collect();
    assert!(!spaces.is_empty(), "no spacing tokens were generated");
    for line in spaces {
        let value = line.split(':').nth(1).unwrap().trim().trim_end_matches(';');
        let px = value.strip_suffix("px").expect("a pixel length");
        assert!(
            px.parse::<i32>().is_ok(),
            "`{line}` is not a whole pixel; GTK cannot lay it out and Rust cannot name it"
        );
    }
}

/// The same scale as Rust constants, so a margin in code and a padding in
/// the stylesheet are the same number by construction.
#[test]
fn the_rust_spacing_scale_is_the_css_one_and_checked_in() {
    let (path, parsed) = source_tokens();
    let rust = tokens::generate_space_rs(&parsed, &label(&path)).unwrap();
    assert!(rust.contains("pub const S3: i32 = 10;"), "{rust}");
    let checked_in = std::fs::read_to_string(widgets_dir().join("data").join("space.rs"))
        .expect("postio-widgets/data/space.rs is missing; run `cargo build -p postio-widgets`");
    assert_eq!(
        rust, checked_in,
        "postio-widgets/data/space.rs is stale. Run `cargo build -p postio-widgets` and commit the result."
    );
}

/// The sizes Postio sets type at are named roles, so a stylesheet says
/// `var(--postio-text-body)` rather than retyping 0.8864rem -- which it had
/// done as 0.8863rem five times.
#[test]
fn the_type_roles_are_named_sizes() {
    let (path, parsed) = source_tokens();
    let css = tokens::generate_metrics(&parsed, &label(&path)).unwrap();
    for (role, size) in tokens::TYPE_ROLES {
        assert!(
            css.contains(&format!("--postio-text-{role}: {size};")),
            "`--postio-text-{role}` is not generated"
        );
    }
}
