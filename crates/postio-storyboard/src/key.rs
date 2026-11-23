//! The review key: which tree a set of runs and a review belong to. Built from git tree ids, not a
//! commit sha, because `issue-land.sh` rebases on every attempt and changes the sha even when
//! nothing reviewed did.

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
        assert_ne!(key(&["aa", "abbb"]), key(&["aaa", "bbb"]));
    }
}
