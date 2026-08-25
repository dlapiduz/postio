//! Build step for `postio-imap`: validate `data/providers.toml`.
//!
//! Issue #191: the shipped provider table is data, not Rust literals, and
//! `build.rs` is what keeps a broken shipped file from ever compiling. This
//! compiles `providers_toml.rs` a second time via `#[path]` -- the same
//! idiom `postio-gtk/build.rs` uses for design tokens (`docs/ARCHITECTURE.md`
//! §10) -- so the parser that runs here and the one the crate uses at
//! runtime for this same file (`discovery/builtin.rs`) and for a user's own
//! `providers.toml` overlay are the identical module. There is nothing to
//! generate: the checked-in file is read directly at runtime via
//! `include_str!`, so this build script's only job is to fail loudly, at
//! `cargo build` time, if that file will not parse and validate -- rather
//! than at first run, or never, if nobody happened to run the tests.

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

    // A secret-shaped key in the *shipped* table is a repository hygiene
    // bug, not a user's mistake to warn about and move past -- see
    // `providers_toml`'s own doc comment on why this same check answers
    // differently for the shipped table than for a user's overlay.
    if !parsed.stripped_secrets.is_empty() {
        panic!(
            "{path} contains what looks like a secret at: {}. \
             Provider client secrets never belong in a shipped, checked-in \
             file -- see ADR 0006 Q4.",
            parsed.stripped_secrets.join(", ")
        );
    }
}
