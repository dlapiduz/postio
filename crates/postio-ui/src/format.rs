//! How Postio writes numbers a person reads.
//!
//! One helper per kind of number, shared by every surface that shows it.
//! Two surfaces formatting bytes their own way is not a cosmetic problem:
//! the status line saying `1.4 GB` while the settings panel says `1,400 MB`
//! reads as two different measurements of two different things (#411).

/// A size, the way the canvas writes it: `11 KB`, `1.1 MB`.
///
/// Binary units, matching `postio-search`'s `larger:`/`smaller:` parser — a
/// part the reader calls `1.0 MB` has to be one `larger:1M` finds, or the two
/// halves of the application disagree about what a megabyte is.
pub fn human_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let bytes = bytes as f64;
    let (value, unit) = if bytes < KIB {
        return format!("{bytes:.0} B");
    } else if bytes < KIB * KIB {
        (bytes / KIB, "KB")
    } else if bytes < KIB * KIB * KIB {
        (bytes / (KIB * KIB), "MB")
    } else {
        (bytes / (KIB * KIB * KIB), "GB")
    };
    // One decimal below ten, none above: `1.1 MB` and `340 KB`, never
    // `1.1 KB` sitting in a column beside `340.2 KB`.
    if value < 10.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.0} {unit}")
    }
}

/// A size that is still being counted: `over 1.4 GB`.
///
/// `MailFootprint::complete == false` means every byte figure is a lower
/// bound, and a surface has to say so. A total that silently climbs every few
/// seconds reads as a bug, and one that is simply wrong is worse than one
/// that admits it is not finished — so the hedge is part of the number rather
/// than a footnote somewhere else on the screen (#411, ADR 0017).
pub fn human_size_bound(bytes: u64, complete: bool) -> String {
    let size = human_size(bytes);
    if complete {
        size
    } else {
        format!("over {size}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_the_way_the_canvas_writes_them() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(11 * 1024), "11 KB");
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn an_incomplete_count_says_so_rather_than_claiming_a_total() {
        // The state the issue calls the one most likely to be got wrong: the
        // header pass has not finished, so this number will grow.
        assert_eq!(human_size_bound(1_503_238_553, false), "over 1.4 GB");
        assert_eq!(human_size_bound(1_503_238_553, true), "1.4 GB");
    }
}
