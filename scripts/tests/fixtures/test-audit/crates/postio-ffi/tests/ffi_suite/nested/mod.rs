//! A module that lives in mod.rs.

#[test]
fn in_mod_rs() {
    let s = "{ not a brace }"; // }
    let c = '}';
    assert_eq!(s.len() + c.len_utf8(), 16);
}
