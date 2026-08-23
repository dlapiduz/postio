//! `docs/keybindings.md` is generated from the command registry.
//!
//! A hand-written key reference is wrong within a release — that is the whole
//! reason the registry exists (spec.md §8: one table, every surface). So the
//! document is rendered from [`registry::all()`] and this test fails when the
//! file on disk no longer matches, with `POSTIO_UPDATE_DOCS=1` to rewrite it.
//!
//! The `?` cheat sheet renders the same table at runtime; this is the copy
//! somebody reads before they have installed anything.

use std::fmt::Write as _;
use std::path::PathBuf;

use postio_core::{Context, ContextSet, Recovery, registry};

fn document_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/keybindings.md")
        .canonicalize()
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/keybindings.md")
        })
}

/// Where a command is available, phrased for a reader rather than a compiler.
fn where_available(contexts: ContextSet) -> String {
    if contexts == ContextSet::ANY {
        return "Everywhere".to_owned();
    }
    let names: Vec<&str> = contexts
        .iter()
        .map(|context| match context {
            Context::List => "list",
            Context::Thread => "thread",
            Context::Reader => "reader",
            Context::Composer => "composer",
            Context::Search => "search",
            Context::Palette => "palette",
        })
        .collect();
    let mut sentence = names.join(", ");
    if let Some(first) = sentence.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    sentence
}

fn keys(binding: &str) -> String {
    // A sequence reads better as `g g` than as one run of characters, and
    // backticks keep `?` and `/` from being read as Markdown.
    format!("`{binding}`")
}

fn render() -> String {
    let mut out = String::new();
    out.push_str(
        "# Keyboard reference\n\
         \n\
         <!-- Generated from `postio-core`'s command registry by\n\
         `crates/postio-core/tests/keybindings_doc.rs`. Do not edit by hand:\n\
         change the registry and run `POSTIO_UPDATE_DOCS=1 cargo test -p postio-core`. -->\n\
         \n\
         Every command below is also in the `Ctrl+K` palette and the `?` cheat\n\
         sheet, because all three are generated from one table.\n\
         \n\
         Bindings come from the design canvas, which is newer than `spec.md` §8\n\
         and wins where they disagree — `e` replies, not `r`.\n\
         \n\
         ## Rebinding\n\
         \n\
         Every binding is overridable from the `[keys]` section of\n\
         `config.toml`, keyed by the command id in the last column:\n\
         \n\
         ```toml\n\
         [keys]\n\
         archive = \"y\"\n\
         first_message = \"g g\"\n\
         ```\n\
         \n\
         A chord joins modifiers to a key with `+` (`ctrl+k`); a sequence\n\
         separates chords with a space (`g g`). Shift is written into the\n\
         character, so `A` is what you get by holding shift — `a` and `A` are\n\
         different bindings. An override that cannot be used, or that collides\n\
         with a key already taken in the same place, is reported in the settings\n\
         panel and the command keeps its default.\n\
         \n\
         While you are typing, single-key bindings do not fire. Only `Escape`,\n\
         the function keys, and chords holding `Ctrl`, `Alt` or `Super` reach a\n\
         command from inside a text field.\n\
         \n\
         ## Bindings\n\
         \n\
         | Keys | Command | Where | Undo | Id |\n\
         |---|---|---|---|---|\n",
    );

    for spec in registry::all() {
        let bindings = spec.bindings().map(keys).collect::<Vec<_>>().join(" or ");
        let recovery = match spec.recovery {
            Recovery::None => "",
            Recovery::Undo => "Undoable",
            Recovery::Confirm => "Asks first",
        };
        let _ = writeln!(
            out,
            "| {bindings} | {} | {} | {recovery} | `{}` |",
            spec.title,
            where_available(spec.contexts),
            spec.id
        );
    }

    out
}

#[test]
fn the_keyboard_reference_matches_the_registry() {
    let path = document_path();
    let rendered = render();

    if std::env::var_os("POSTIO_UPDATE_DOCS").is_some() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create docs/");
        }
        std::fs::write(&path, &rendered).expect("write the keyboard reference");
        return;
    }

    let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}\nrun `POSTIO_UPDATE_DOCS=1 cargo test -p postio-core` to generate it",
            path.display()
        )
    });

    assert_eq!(
        on_disk, rendered,
        "docs/keybindings.md is out of date with the registry; \
         run `POSTIO_UPDATE_DOCS=1 cargo test -p postio-core`"
    );
}

#[test]
fn the_reference_names_every_command() {
    let rendered = render();

    for spec in registry::all() {
        assert!(
            rendered.contains(&format!("`{}`", spec.id)),
            "`{}` is missing from the keyboard reference",
            spec.id
        );
        assert!(
            rendered.contains(spec.title),
            "`{}` is missing from the keyboard reference",
            spec.title
        );
    }
}
