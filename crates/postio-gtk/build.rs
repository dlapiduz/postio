//! Build step for `postio-gtk`.
//!
//! Two jobs, in this order:
//!
//! 1. Read the Industry design system's `:root` token block and generate
//!    `data/tokens.css`, through `postio_ui::tokens` — a build-dependency
//!    now (#569) rather than `#[path = "src/tokens.rs"]`, so the parser and
//!    emitter this crate uses at build time are the same crate a second
//!    frontend links at run time, not two copies kept in step by
//!    convention. The generated file is checked in so that a build outside
//!    the repository (or without the `Design/` tree) still works, and
//!    `postio-ui`'s own drift tests fail if the checked-in copy has
//!    drifted from the source.
//! 2. Compile `data/postio.gresource.xml` into the GResource bundle that
//!    carries the stylesheet and the vendored OFL fonts, so the app resolves
//!    both without a system font installation and without touching the network.

use std::path::{Path, PathBuf};

use postio_ui::tokens;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/tokens.rs");
    println!("cargo:rerun-if-changed=data/postio.gresource.xml");
    println!("cargo:rerun-if-changed=data/shell.css");
    println!("cargo:rerun-if-env-changed=POSTIO_DESIGN_SYSTEM");

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let data_dir = manifest_dir.join("data");

    match design_system_path(&manifest_dir) {
        Some(source) => {
            println!("cargo:rerun-if-changed={}", source.display());
            generate_tokens(&source, &data_dir.join("tokens.css"));
            generate_reader_tokens(&source, &data_dir.join("reader-tokens.css"));
        }
        None => {
            println!(
                "cargo:warning=Industry design system not found; \
                 keeping the checked-in data/tokens.css and data/reader-tokens.css. \
                 Set POSTIO_DESIGN_SYSTEM to the styles.css to regenerate them."
            );
        }
    }

    for font in font_files(&data_dir) {
        println!("cargo:rerun-if-changed={}", font.display());
    }

    glib_build_tools::compile_resources(
        &[data_dir.to_str().expect("data dir path is not UTF-8")],
        data_dir
            .join("postio.gresource.xml")
            .to_str()
            .expect("gresource path is not UTF-8"),
        "postio.gresource",
    );
}

/// `Design/_ds/industry-<uuid>/styles.css`, or whatever `POSTIO_DESIGN_SYSTEM`
/// points at.
fn design_system_path(manifest_dir: &Path) -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("POSTIO_DESIGN_SYSTEM") {
        let path = PathBuf::from(explicit);
        return path.exists().then_some(path);
    }
    let ds = manifest_dir.parent()?.parent()?.join("Design").join("_ds");
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

fn generate_tokens(source: &Path, out: &Path) {
    let parsed = parse_source(source);
    let label = relative_label(source);
    let generated = tokens::generate(&parsed, &label)
        .unwrap_or_else(|e| panic!("cannot generate tokens.css: {e}"));
    write_if_changed(out, &generated);
}

fn generate_reader_tokens(source: &Path, out: &Path) {
    let parsed = parse_source(source);
    let label = relative_label(source);
    let generated = tokens::generate_reader(&parsed, &label)
        .unwrap_or_else(|e| panic!("cannot generate reader-tokens.css: {e}"));
    write_if_changed(out, &generated);
}

fn parse_source(source: &Path) -> tokens::Tokens {
    let css = std::fs::read_to_string(source)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", source.display()));
    tokens::Tokens::parse(&css)
        .unwrap_or_else(|e| panic!("cannot read the design tokens in {}: {e}", source.display()))
}

/// Write only on a real change: rewriting would bump the mtime on every
/// build and make `rerun-if-changed=data/` loop.
fn write_if_changed(out: &Path, generated: &str) {
    let current = std::fs::read_to_string(out).unwrap_or_default();
    if current != generated {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(out, generated)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
    }
}

/// `Design/_ds/industry-…/styles.css` — the tail of the path from the
/// repository root, so the generated banner is checkout-independent.
fn relative_label(source: &Path) -> String {
    let parts: Vec<String> = source
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    match parts.iter().position(|p| p == "Design") {
        Some(i) => parts[i..].join("/"),
        None => source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

fn font_files(data_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![data_dir.join("fonts")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}
