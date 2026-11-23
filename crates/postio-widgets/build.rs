//! Generates the shared metrics, spacing and reader-token files from the design system's `:root`
//! tokens (checked in, so a build without `Design/` works), then compiles
//! `data/widgets.gresource.xml`.

use std::path::{Path, PathBuf};

// Compiled here by path, not as a build-dependency on postio-ui: an edit anywhere in the UI crate
// would otherwise rerun this script and force a host build of that crate.
#[allow(dead_code)]
#[path = "../postio-ui/src/tokens.rs"]
mod tokens;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../postio-ui/src/tokens.rs");
    println!("cargo:rerun-if-changed=data/widgets.gresource.xml");
    println!("cargo:rerun-if-changed=data/widgets.css");
    println!("cargo:rerun-if-changed=data/metrics.css");
    println!("cargo:rerun-if-env-changed=POSTIO_DESIGN_SYSTEM");

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let data_dir = manifest.join("data");
    let ui_data_dir = manifest
        .parent()
        .expect("crates/postio-widgets")
        .join("postio-ui")
        .join("data");

    match design_system_path(&manifest) {
        Some(source) => {
            println!("cargo:rerun-if-changed={}", source.display());
            let parsed = parse_source(&source);
            let label = relative_label(&source);
            write_if_changed(
                &data_dir.join("metrics.css"),
                &tokens::generate_metrics(&parsed, &label)
                    .unwrap_or_else(|e| panic!("cannot generate metrics.css: {e}")),
            );
            write_if_changed(
                &data_dir.join("space.rs"),
                &tokens::generate_space_rs(&parsed, &label)
                    .unwrap_or_else(|e| panic!("cannot generate space.rs: {e}")),
            );
            write_if_changed(
                &ui_data_dir.join("reader-tokens.css"),
                &tokens::generate_reader(&parsed, &label)
                    .unwrap_or_else(|e| panic!("cannot generate reader-tokens.css: {e}")),
            );
        }
        None => {
            println!(
                "cargo:warning=Industry design system not found; \
                 keeping the checked-in data/metrics.css, data/space.rs and \
                 ../postio-ui/data/reader-tokens.css. \
                 Set POSTIO_DESIGN_SYSTEM to the styles.css to regenerate them."
            );
        }
    }

    let icons_dir = data_dir.join("icons");
    for name in [
        "apps/dev.postio.Postio.svg",
        "apps/dev.postio.Postio-symbolic.svg",
        "actions/postio-heading-symbolic.svg",
    ] {
        println!(
            "cargo:rerun-if-changed={}",
            icons_dir.join("scalable").join(name).display()
        );
    }
    glib_build_tools::compile_resources(
        &[
            data_dir.to_str().expect("data dir path is not UTF-8"),
            icons_dir.to_str().expect("icons dir path is not UTF-8"),
        ],
        data_dir
            .join("widgets.gresource.xml")
            .to_str()
            .expect("gresource path is not UTF-8"),
        "postio-widgets.gresource",
    );
}

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

fn parse_source(source: &Path) -> tokens::Tokens {
    let css = std::fs::read_to_string(source)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", source.display()));
    tokens::Tokens::parse(&css)
        .unwrap_or_else(|e| panic!("cannot read the design tokens in {}: {e}", source.display()))
}

/// Writes only on a real change: a rewrite bumps the mtime and makes `rerun-if-changed=data/` loop.
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
