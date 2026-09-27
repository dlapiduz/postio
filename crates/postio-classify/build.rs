//! Build step for `postio-classify`: validate `data/senders.toml`.
//!
//! The automated-senders table is data, not Rust literals (FR-114), and this
//! is what keeps a broken shipped table from ever compiling. It compiles
//! `src/senders/table.rs` a second time via `#[path]`, the idiom
//! `postio-account/build.rs` uses for its provider presets, so the parser
//! that runs here is the one the classifier loads the table with. There is
//! nothing to generate: the crate reads the file with `include_str!`.

#[path = "src/senders/table.rs"]
#[allow(dead_code)]
mod table;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/senders/table.rs");
    println!("cargo:rerun-if-changed=data/senders.toml");

    let path = "data/senders.toml";
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|error| panic!("cannot read {path}: {error}"));
    if let Err(error) = table::parse(&text) {
        panic!("{path} does not load: {error}");
    }
}
