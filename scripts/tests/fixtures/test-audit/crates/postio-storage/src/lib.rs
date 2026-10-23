//! Storage (fixture). Spec 005 and spec 011.
mod change;
mod deep;

pub fn rows() -> usize {
    3
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_rows() {
        assert_eq!(super::rows(), 3);
    }
}
