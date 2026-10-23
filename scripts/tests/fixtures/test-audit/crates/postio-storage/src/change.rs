//! Change detection.

#[cfg(test)]
mod tests {
    /// Bare FR-003 with two specs nowhere in scope.
    #[test]
    fn a_change_is_seen() {
        assert!(seen());
    }
}
