//! Size parsing for `larger:` and `smaller:`.
//!
//! `larger:1M` is the canvas spelling. Suffixes are binary, matching the way
//! mail clients and file managers report message sizes: `K` is 1024 bytes, `M`
//! is 1024 K, `G` is 1024 M. A bare number is bytes. Fractions are allowed
//! (`1.5M`), and the `B` in `MB` is optional.
//!
//! Anything else — `larger:`, `larger:big`, `larger:1X` — returns `None`, which
//! the parser turns into a [`crate::query::Partial`] rather than an error.

const KIB: f64 = 1024.0;

/// Parses a size value into bytes. Returns `None` for anything unrecognized.
pub(crate) fn parse_size(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let digits_end = value
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(value.len());
    let (number, suffix) = value.split_at(digits_end);
    if number.is_empty() || number == "." {
        return None;
    }

    let multiplier = match suffix.to_ascii_lowercase().as_str() {
        "" | "b" | "byte" | "bytes" => 1.0,
        "k" | "kb" | "kib" => KIB,
        "m" | "mb" | "mib" => KIB * KIB,
        "g" | "gb" | "gib" => KIB * KIB * KIB,
        _ => return None,
    };

    let parsed: f64 = number.parse().ok()?;
    let bytes = parsed * multiplier;
    if !bytes.is_finite() || bytes < 0.0 || bytes > u64::MAX as f64 {
        return None;
    }
    Some(bytes.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_suffixes() {
        assert_eq!(parse_size("1M"), Some(1024 * 1024));
        assert_eq!(parse_size("1m"), Some(1024 * 1024));
        assert_eq!(parse_size("1MB"), Some(1024 * 1024));
        assert_eq!(parse_size("1mib"), Some(1024 * 1024));
        assert_eq!(parse_size("512k"), Some(512 * 1024));
        assert_eq!(parse_size("2G"), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("100"), Some(100));
        assert_eq!(parse_size("100b"), Some(100));
        assert_eq!(parse_size(" 10 "), Some(10));
    }

    #[test]
    fn fractions_round() {
        assert_eq!(parse_size("1.5M"), Some(1_572_864));
        assert_eq!(parse_size("0.5k"), Some(512));
        assert_eq!(parse_size("1.4"), Some(1));
    }

    #[test]
    fn half_typed_and_nonsense_values_are_none() {
        for value in ["", "  ", "big", "M", "kb", "1X", "1.2.3", ".", "-1", "1e9"] {
            assert_eq!(parse_size(value), None, "{value}");
        }
    }

    #[test]
    fn absurd_sizes_do_not_overflow() {
        assert_eq!(parse_size("99999999999999999999999999G"), None);
    }
}
