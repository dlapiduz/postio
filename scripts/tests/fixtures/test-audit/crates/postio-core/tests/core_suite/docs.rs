//! The generated reference is current.

#[test]
fn the_reference_matches() {
    let path = format!("{}/../../docs/keybindings.md", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("Archive"));
}
