//! Emit the design tokens for a frontend that cannot be handed Rust.
//!
//! The GTK frontend gets its tokens through `postio-gtk/build.rs`, which can
//! call the emitter directly. Swift cannot, so this writes the file that
//! `scripts/macos-build.sh` puts into the package.
//!
//! Both come from one parsed `Tokens` and one set of required names, so
//! retuning the design system moves both frontends or fails the build for
//! both — which is the whole reason the values are generated rather than
//! typed twice.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let source: PathBuf = args
        .next()
        .ok_or("usage: postio-tokens <design-system.css> <out.swift>")?
        .into();
    let out: PathBuf = args
        .next()
        .ok_or("usage: postio-tokens <design-system.css> <out.swift>")?
        .into();

    let css = std::fs::read_to_string(&source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    let tokens = postio_ui::tokens::Tokens::parse(&css)?;
    let label = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.display().to_string());
    let swift = postio_ui::tokens::generate_swift(&tokens, &label)?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, swift)?;
    println!("{}", out.display());
    Ok(())
}
