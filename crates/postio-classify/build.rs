//! Validates `data/senders.toml` by compiling `src/senders/table.rs` a second time via `#[path]`,
//! so a broken shipped table never compiles.

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
