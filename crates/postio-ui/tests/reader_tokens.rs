//! The reader's `--r-*` palette (`postio-gtk/data/reader-tokens.css`) is a
//! build artefact of the same `Tokens` pipeline `tests/tokens.rs` checks —
//! see `postio_ui::tokens::generate_reader`. These tests are what stops it
//! drifting. The checks over `postio-gtk/data/reader.css` itself — the
//! hand-authored consumer stylesheet, not a generated artefact — stay in
//! `postio-gtk/tests/reader_tokens.rs`, where that file lives (#569).
//!
//! No display to guard: this crate has no toolkit dependency at all.

use std::path::PathBuf;

use postio_ui::tokens::{self, Tokens};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `postio-gtk`'s own directory, a sibling under `crates/`.
fn gtk_dir() -> PathBuf {
    manifest_dir()
        .parent()
        .expect("crates/postio-ui")
        .join("postio-gtk")
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
    std::fs::read_to_string(gtk_dir().join("data").join("reader-tokens.css"))
        .expect("data/reader-tokens.css is missing; run `cargo build -p postio-gtk`")
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
