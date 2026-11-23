//! What this terminal can do, found out once before raw mode. Colour depth comes from the
//! environment alone; the keyboard protocol and background are asked of the terminal at startup,
//! and a terminal that does not answer is taken to lack them.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Colour {
    None,
    Ansi16,
    Ansi256,
    TrueColor,
}

impl Colour {
    /// `NO_COLOR`, when non-empty, wins over everything (<https://no-color.org>).
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

    pub fn detect() -> Colour {
        let var = |name| std::env::var(name).ok();
        Colour::from_env(
            var("NO_COLOR").as_deref(),
            var("COLORTERM").as_deref(),
            var("TERM").as_deref(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Background {
    Light,
    Dark,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caps {
    pub colour: Colour,
    /// Whether the kitty keyboard protocol is spoken, so `ctrl+Return` is
    /// not `Return`.
    pub keyboard_enhancement: bool,
    pub mouse: bool,
    pub background: Background,
    /// Always `None`: reserved for an image protocol.
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
