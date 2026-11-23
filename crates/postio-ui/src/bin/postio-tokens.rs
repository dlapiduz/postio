//! Emit the design tokens for Swift, which cannot be handed Rust; GTK gets them via
//! `postio-widgets/build.rs`.
//! Both come from one parsed `Tokens`, so retuning moves both frontends or fails both builds.

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
