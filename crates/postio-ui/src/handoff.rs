//! Which editor a draft hands off to (`[compose] editor`) and what to do when it is not one
//! (#1288).
//! A macOS app launched from Finder has no `$EDITOR`, so the editor is a setting; only the platform
//! can say whether it finds the name.

/// Three answers, not two: on freedesktop a name not on `PATH` is a typo, not a terminal program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Application,
    TerminalProgram,
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    PlatformDefault,
    Application(String),
    NeedsTerminal(String),
    Missing(String),
}

/// A blank setting is `PlatformDefault` whatever `found` says.
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

pub fn missing_advice(name: &str) -> String {
    format!(
        "There is no {name} on this machine. Check the name, or clear this \
         setting to use whatever already opens a text file here."
    )
}

/// Does not offer to open a terminal: which terminal and profile is a guess Postio should not make.
pub fn terminal_advice(program: &str) -> String {
    format!(
        "{program} runs in a terminal, and Postio cannot open one for it. \
         Choose an editor that opens in a window of its own, or clear this \
         setting to use whatever already opens a text file here."
    )
}

/// Named when there is a name; unnamed when nothing is chosen, since Postio does not know which
/// application the platform will pick.
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
        assert_eq!(
            target("vim", Found::TerminalProgram),
            Target::NeedsTerminal("vim".to_owned())
        );
    }

    #[test]
    fn a_name_that_is_not_there_is_a_typo_not_a_terminal_program() {
        // Two answers conflated "not an application" with "wants a terminal", wrong on freedesktop
        // where it is a misspelling (#1297).
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
