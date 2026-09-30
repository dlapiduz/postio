//! What this terminal can do, found out once, before raw mode.
//!
//! Colour depth comes from the environment and nothing else, so it is a pure
//! function and tested as one. The keyboard protocol and the background colour
//! are asked of the terminal itself; they are read at startup and handed in,
//! and a terminal that does not answer is taken to lack them
//! (`data-model.md`, Terminal session).

/// How many colours the terminal draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Colour {
    /// None: `NO_COLOR`, or a dumb terminal. Marks carry every meaning.
    None,
    /// The sixteen ANSI colours, as the user's theme defines them.
    Ansi16,
    /// The 256-colour palette.
    Ansi256,
    /// Any RGB colour.
    TrueColor,
}

impl Colour {
    /// The colour depth these environment variables describe.
    ///
    /// `NO_COLOR`, when set to anything non-empty, wins over everything
    /// (<https://no-color.org>).
    pub fn from_env(no_color: Option<&str>, colorterm: Option<&str>, term: Option<&str>) -> Colour {
        if no_color.is_some_and(|value| !value.is_empty()) {
            return Colour::None;
        }
        if matches!(colorterm, Some("truecolor" | "24bit")) {
            return Colour::TrueColor;
        }
        match term {
            Some("dumb") => Colour::None,
            Some(term) if term.contains("256color") => Colour::Ansi256,
            _ => Colour::Ansi16,
        }
    }

    /// The colour depth of this process's environment.
    pub fn detect() -> Colour {
        let var = |name| std::env::var(name).ok();
        Colour::from_env(
            var("NO_COLOR").as_deref(),
            var("COLORTERM").as_deref(),
            var("TERM").as_deref(),
        )
    }
}

/// Whether the terminal's background is light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Background {
    /// A light background.
    Light,
    /// A dark background.
    Dark,
    /// The terminal did not say.
    #[default]
    Unknown,
}

/// Everything the frontend knows about its terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caps {
    /// How many colours.
    pub colour: Colour,
    /// Whether the kitty keyboard protocol is spoken, so `ctrl+Return` is
    /// not `Return`.
    pub keyboard_enhancement: bool,
    /// Whether mouse events are wanted (`[tui].mouse`).
    pub mouse: bool,
    /// The background, for choosing readable colours.
    pub background: Background,
    /// Reserved for the next iteration: which image protocol, if any
    /// (research R8). Always `None` in this one.
    pub graphics: Option<()>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_wins_over_everything() {
        assert_eq!(
            Colour::from_env(Some("1"), Some("truecolor"), Some("xterm-256color")),
            Colour::None
        );
    }

    #[test]
    fn an_empty_no_color_is_no_request() {
        assert_eq!(
            Colour::from_env(Some(""), Some("truecolor"), Some("xterm")),
            Colour::TrueColor
        );
    }

    #[test]
    fn colorterm_names_true_colour() {
        for value in ["truecolor", "24bit"] {
            assert_eq!(
                Colour::from_env(None, Some(value), Some("xterm")),
                Colour::TrueColor
            );
        }
    }

    #[test]
    fn term_names_256_colours_or_none() {
        assert_eq!(
            Colour::from_env(None, None, Some("xterm-256color")),
            Colour::Ansi256
        );
        assert_eq!(Colour::from_env(None, None, Some("dumb")), Colour::None);
        assert_eq!(Colour::from_env(None, None, Some("xterm")), Colour::Ansi16);
        assert_eq!(Colour::from_env(None, None, None), Colour::Ansi16);
    }
}
