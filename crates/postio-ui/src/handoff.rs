//! Which editor a draft hands off to, and what to do when it is not one (#1288).
//!
//! Canvas 26 labels the compose window's hand-off `Open in $EDITOR`. That
//! label is right on freedesktop and wrong on macOS, where an application
//! launched from Finder has **no shell environment** — `$EDITOR` is simply
//! absent for most people, so a button naming it would name nothing. So the
//! editor is a setting (`[compose] editor`), and this is the small decision
//! both frontends make about the value in it.
//!
//! # The one thing a frontend has to answer first
//!
//! Whether the platform can find an application by that name. Only the
//! platform knows — `NSWorkspace.urlForApplication(withName:)` on macOS, a
//! desktop entry or a `PATH` lookup on freedesktop — so it is an input here
//! rather than a guess. What is *not* a guess is what follows from it, and
//! that is the part worth having in one place: a name the platform cannot
//! open as an application is a command, a command needs a terminal, and
//! neither frontend can open one.
//!
//! Saying that plainly is the point. A hand-off that silently did nothing
//! for somebody who typed `vim` would be the worst of the three outcomes,
//! and it is the one that happens by default.

/// What the platform found when it looked for the configured editor.
///
/// The one question only the platform can answer, and it has **three**
/// answers rather than two. It carried two until GTK tried to adopt this:
/// on macOS a name that is not an application bundle is almost always a
/// terminal program, so "not an application" and "wants a terminal" looked
/// like the same fact. On freedesktop they are plainly not — a name that is
/// not on `PATH` is a *typo*, and telling somebody their editor "runs in a
/// terminal" when they misspelled it is a wrong answer confidently given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// Something this platform can open in a window of its own.
    Application,
    /// A real program, but a terminal one — no window to open it in.
    TerminalProgram,
    /// Nothing by that name at all.
    Nothing,
}

/// What the hand-off should do with the configured editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Nothing is configured: open the file the way this platform opens a
    /// text file, and let the user's own default associations decide.
    PlatformDefault,
    /// Open it with this application.
    Application(String),
    /// A program that wants a terminal, which neither frontend can give it.
    NeedsTerminal(String),
    /// A name the platform could not find. Almost always a typo.
    Missing(String),
}

/// What to do about `configured`, given what the platform found.
///
/// A blank setting is `PlatformDefault` whatever `found` says: an empty
/// string is "I have not chosen", and a lookup on it means nothing.
pub fn target(configured: &str, found: Found) -> Target {
    let name = configured.trim();
    if name.is_empty() {
        return Target::PlatformDefault;
    }
    match found {
        Found::Application => Target::Application(name.to_owned()),
        Found::TerminalProgram => Target::NeedsTerminal(name.to_owned()),
        Found::Nothing => Target::Missing(name.to_owned()),
    }
}

/// What to tell somebody whose editor is not there.
///
/// Named, and said as a fact rather than a diagnosis: the overwhelmingly
/// likely cause is a typo, and a sentence that guessed at *which* typo would
/// be wrong more often than not.
pub fn missing_advice(name: &str) -> String {
    format!(
        "There is no {name} on this machine. Check the name, or clear this \
         setting to use whatever already opens a text file here."
    )
}

/// What to tell somebody whose editor needs a terminal.
///
/// Names the program and says what to do instead, because the alternative —
/// a button that appears to work and does not — is what this exists to
/// prevent. It does not offer to open a terminal: which terminal, with which
/// profile, is a decision Postio would be guessing at, and a mail client
/// spawning terminals is a larger promise than one line of settings.
pub fn terminal_advice(program: &str) -> String {
    format!(
        "{program} runs in a terminal, and Postio cannot open one for it. \
         Choose an editor that opens in a window of its own, or clear this \
         setting to use whatever already opens a text file here."
    )
}

/// What the hand-off button should say, given the setting.
///
/// Named when there is a name, because "Open in BBEdit" tells somebody what
/// is about to happen and `Open in $EDITOR` tells them about an environment
/// variable. Unnamed when nothing is chosen — Postio does not know which
/// application the platform will pick, and inventing one in the label would
/// be a promise it cannot keep.
pub fn button_label(configured: &str) -> String {
    let name = configured.trim();
    if name.is_empty() {
        "Edit elsewhere".to_owned()
    } else {
        format!("Open in {name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_chosen_leaves_it_to_the_platform() {
        assert_eq!(target("", Found::Application), Target::PlatformDefault);
        assert_eq!(
            target("   ", Found::Nothing),
            Target::PlatformDefault,
            "whitespace is not a choice, and a lookup on it means nothing"
        );
    }

    #[test]
    fn a_name_the_platform_can_open_is_the_editor() {
        assert_eq!(
            target("Some Editor", Found::Application),
            Target::Application("Some Editor".to_owned())
        );
        assert_eq!(
            target("  Some Editor  ", Found::Application),
            Target::Application("Some Editor".to_owned()),
            "what somebody typed is trimmed before it is used"
        );
    }

    #[test]
    fn a_terminal_program_is_named_as_one() {
        // The case the module was written for: somebody types the name of
        // the editor they actually use, and it has no window to open.
        assert_eq!(
            target("vim", Found::TerminalProgram),
            Target::NeedsTerminal("vim".to_owned())
        );
    }

    #[test]
    fn a_name_that_is_not_there_is_a_typo_not_a_terminal_program() {
        // The distinction GTK's adoption forced (#1297). With two answers,
        // "not an application" meant "wants a terminal" — true on macOS,
        // where a name that is not a bundle usually is one, and plainly
        // false on freedesktop, where a name that is not on `PATH` is a
        // misspelling. Telling somebody their editor runs in a terminal
        // when they mistyped it is a wrong answer confidently given.
        assert_eq!(
            target("vum", Found::Nothing),
            Target::Missing("vum".to_owned())
        );

        let advice = missing_advice("vum");
        assert!(advice.contains("vum"), "{advice}");
        assert!(
            !advice.contains("terminal"),
            "a missing editor is not a terminal one: {advice}"
        );
        assert!(
            advice.contains("clear this setting"),
            "there has to be a way out of it: {advice}"
        );
    }

    #[test]
    fn the_advice_names_the_program_and_what_to_do_instead() {
        let advice = terminal_advice("vim");

        assert!(advice.contains("vim"), "{advice}");
        assert!(
            advice.contains("clear this setting"),
            "there has to be a way out of it: {advice}"
        );
    }

    #[test]
    fn the_button_says_where_the_draft_is_going_when_it_knows() {
        assert_eq!(button_label("Some Editor"), "Open in Some Editor");
        assert_eq!(
            button_label(""),
            "Edit elsewhere",
            "with nothing chosen Postio does not know which application the \
             platform will pick, and must not name one"
        );
    }
}
