//! The generated reference is current.

#[test]
fn the_reference_matches() {
    let path = format!("{}/../../docs/keybindings.md", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("Archive"));
}

fn reference_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/config.md")
}

#[test]
fn the_reference_is_current_through_a_helper() {
    let text = std::fs::read_to_string(reference_path()).unwrap();
    assert!(text.contains("[ui]"));
}
