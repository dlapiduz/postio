//! Validates `data/providers.toml` at build time with the same parser the runtime uses (#191).

#[path = "src/discovery/providers_toml.rs"]
mod providers_toml;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/discovery/providers_toml.rs");
    println!("cargo:rerun-if-changed=data/providers.toml");

    let path = "data/providers.toml";
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|error| panic!("cannot read {path}: {error}"));

    let parsed = providers_toml::parse(&text)
        .unwrap_or_else(|error| panic!("{path} does not parse: {error}"));

    // A secret-shaped key in the shipped table is a repository hygiene bug, not a warning.
    if !parsed.stripped_secrets.is_empty() {
        panic!(
            "{path} contains what looks like a secret at: {}. \
             Provider client secrets never belong in a shipped, checked-in \
             file -- see ADR 0006 Q4.",
            parsed.stripped_secrets.join(", ")
        );
    }
}
