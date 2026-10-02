//! The review key: which tree a set of runs and a review belong to
//! (research R13).
//!
//! A commit sha would be the obvious key and is the wrong one:
//! `issue-land.sh` rebases on every attempt, so the sha of a reviewed branch
//! changes before it lands even when nothing the review looked at did. The
//! key is instead built from the git *tree* ids of what a review depends on
//! -- the app's crates and the catalogue -- so a rebase that does not touch
//! them keeps it, and one that does changes it.

/// The key for these tree ids: blake3 over them in sorted order, so the
/// order a caller lists them in does not matter.
pub fn key(trees: &[&str]) -> String {
    let mut sorted = trees.to_vec();
    sorted.sort_unstable();
    let mut hasher = blake3::Hasher::new();
    for tree in sorted {
        // Length-prefixed, so two lists that join to the same text differ.
        hasher.update(&(tree.len() as u64).to_le_bytes());
        hasher.update(tree.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::key;

    #[test]
    fn the_key_ignores_the_order_trees_are_listed_in() {
        assert_eq!(key(&["aaa", "bbb", "ccc"]), key(&["ccc", "aaa", "bbb"]));
    }

    #[test]
    fn the_key_changes_when_any_tree_does() {
        let base = key(&["aaa", "bbb"]);
        assert_ne!(base, key(&["aaa", "bbc"]));
        assert_ne!(base, key(&["aaa"]));
    }

    #[test]
    fn the_key_is_a_hex_digest_and_is_not_a_join() {
        let k = key(&["aaa", "bbb"]);
        assert_eq!(k.len(), 64, "{k}");
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()), "{k}");
        // "aa"+"abbb" and "aaa"+"bbb" join to the same text; the key must
        // not confuse them.
        assert_ne!(key(&["aa", "abbb"]), key(&["aaa", "bbb"]));
    }
}
