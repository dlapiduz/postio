//! The words of the Junk and Trash auto-empty setting (#1932), so the two
//! panes that draw it say the same thing: the choices, what each is called,
//! and the one sentence that says what it does.
//!
//! The setting itself is `[sync] empty_junk_after_days` and
//! `empty_trash_after_days`; the rule it feeds is
//! `postio_core::retention::AutoEmpty`.

/// The control's heading.
pub const HEADING: &str = "Empty Junk and Trash";

/// What choosing anything but Never does, said plainly because it cannot be
/// undone: the mail is deleted here and on the server.
pub const NOTE: &str =
    "Deletes mail older than this for good, here and on the server. Flagged mail is kept.";

/// The days a pane offers; `0` is never, the default.
pub const CHOICES: [u32; 4] = [0, 7, 30, 90];

/// What a pane calls `days`: `Never`, `7 days`, or, for a number only the
/// file could have written, its own count.
pub fn label(days: u32) -> String {
    match days {
        0 => "Never".to_owned(),
        1 => "1 day".to_owned(),
        n => format!("{n} days"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_never_and_every_choice_has_a_name() {
        assert_eq!(CHOICES[0], 0, "off comes first and is the default");
        let named: Vec<String> = CHOICES.iter().map(|days| label(*days)).collect();
        assert_eq!(named, ["Never", "7 days", "30 days", "90 days"]);
        assert_eq!(label(1), "1 day");
        assert_eq!(label(14), "14 days", "a hand-written count is still said");
    }

    #[test]
    fn the_note_says_it_is_permanent_and_spares_flagged_mail() {
        assert!(NOTE.contains("for good") && NOTE.contains("Flagged"));
    }
}
