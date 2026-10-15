//! A section a pane rewrites is set off from the one before it.
//!
//! Every structured pane writes its table by removing it and splicing a fresh
//! one in at the end. The fresh table came out of `toml` with no blank line
//! above it, so `[compose]`'s last key ran straight into `[sync]` -- in the
//! file, in `$EDITOR`, and in the Config file pane of both apps.

use std::collections::BTreeMap;

use postio_config::{
    ComposeConfig, Config, ReaderConfig, SyncConfig, patch_compose, patch_keys, patch_reader,
    patch_sync, patch_ui,
};

/// Each header line after the first, with the line above it, where that
/// line is not blank.
fn crowded_headers(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    lines
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, line)| line.starts_with('['))
        .filter(|(at, _)| !lines[at - 1].trim().is_empty())
        .map(|(at, line)| format!("{:?} then {line:?}", lines[at - 1]))
        .collect()
}

#[test]
fn every_pane_s_section_is_set_off_by_a_blank_line() {
    let starter = Config::default().to_toml_string().expect("the defaults");
    assert_eq!(crowded_headers(&starter), Vec::<String>::new(), "{starter}");

    let defaults = Config::default();
    let mut keys = BTreeMap::new();
    keys.insert("archive".to_owned(), "e".to_owned());
    let mut text = starter;
    text = patch_compose(&text, &ComposeConfig::default()).expect("compose");
    text = patch_sync(&text, &SyncConfig::default()).expect("sync");
    text = patch_ui(&text, &defaults.ui).expect("ui");
    text = patch_keys(&text, &keys).expect("keys");
    text = patch_reader(&text, &ReaderConfig::default()).expect("reader");

    assert_eq!(crowded_headers(&text), Vec::<String>::new(), "{text}");
    Config::from_toml_str(&text).expect("still a valid config");
}

#[test]
fn a_section_alone_in_the_file_has_no_blank_line_above_it() {
    let text = patch_sync("", &SyncConfig::default()).expect("sync");
    assert!(text.starts_with("[sync]"), "{text:?}");
}
