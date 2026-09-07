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
}

/// What to do about `configured`, given whether the platform found an
/// application by that name.
///
/// A blank setting is `PlatformDefault` whatever `is_application` says: an
/// empty string is "I have not chosen", and a lookup on it means nothing.
pub fn target(configured: &str, is_application: bool) -> Target {
    let name = configured.trim();
    if name.is_empty() {
        return Target::PlatformDefault;
    }
    if is_application {
        Target::Application(name.to_owned())
    } else {
        Target::NeedsTerminal(name.to_owned())
    }
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
        assert_eq!(target("", true), Target::PlatformDefault);
        assert_eq!(
            target("   ", false),
            Target::PlatformDefault,
            "whitespace is not a choice, and a lookup on it means nothing"
        );
    }

    #[test]
    fn a_name_the_platform_can_open_is_the_editor() {
        assert_eq!(
            target("Some Editor", true),
            Target::Application("Some Editor".to_owned())
        );
        assert_eq!(
            target("  Some Editor  ", true),
            Target::Application("Some Editor".to_owned()),
            "what somebody typed is trimmed before it is used"
        );
    }

    #[test]
    fn a_name_that_is_not_an_application_is_a_command_that_wants_a_terminal() {
        // The case the whole module exists for: this is what happens when
        // somebody types the name of the editor they actually use.
        assert_eq!(
            target("vim", false),
            Target::NeedsTerminal("vim".to_owned())
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
