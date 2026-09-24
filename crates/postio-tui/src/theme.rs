//! Colours, as roles, resolved for this terminal.
//!
//! The terminal's own palette for everything -- so Postio looks at home in
//! whatever theme the user chose -- and, on a true-colour terminal, Postio's
//! accent for the selected row and the focus, read from the generated design
//! tokens and never retyped (clarified 2026-09-23, FR-054). Under `NO_COLOR`
//! there is no colour at all and every state keeps a mark that is not a
//! colour (`contracts/tui-surface.md` §Colour roles). Any role can be
//! overridden in `[tui.colors]`.

use std::collections::BTreeMap;
use std::str::FromStr;

use ratatui::style::{Color, Modifier, Style};

use crate::caps::{Background, Colour};

/// What a piece of the screen is, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Ordinary text.
    Text,
    /// Secondary text: dates, counts, previews.
    Dim,
    /// Postio's own emphasis.
    Accent,
    /// The selected rows.
    Selection,
    /// Where the keyboard is.
    Focus,
    /// An unread row.
    Unread,
    /// A flagged row's mark.
    Flagged,
    /// A link in a message.
    Link,
    /// Quoted text.
    Quote,
    /// Code in a message.
    Code,
    /// Something failed.
    Error,
    /// Something needs attention.
    Warning,
    /// Something worked.
    Success,
}

impl Role {
    /// Every role, for enumeration and validation.
    pub const ALL: [Role; 13] = [
        Role::Text,
        Role::Dim,
        Role::Accent,
        Role::Selection,
        Role::Focus,
        Role::Unread,
        Role::Flagged,
        Role::Link,
        Role::Quote,
        Role::Code,
        Role::Error,
        Role::Warning,
        Role::Success,
    ];

    /// The role's name in `[tui.colors]`.
    pub fn name(self) -> &'static str {
        match self {
            Role::Text => "text",
            Role::Dim => "dim",
            Role::Accent => "accent",
            Role::Selection => "selection",
            Role::Focus => "focus",
            Role::Unread => "unread",
            Role::Flagged => "flagged",
            Role::Link => "link",
            Role::Quote => "quote",
            Role::Code => "code",
            Role::Error => "error",
            Role::Warning => "warning",
            Role::Success => "success",
        }
    }
}

/// Every role's style for one terminal.
#[derive(Debug, Clone)]
pub struct Theme {
    styles: BTreeMap<Role, Style>,
}

impl Theme {
    /// The theme for a terminal of `colour` depth on `background`, with
    /// `overrides` from `[tui.colors]` applied, and what could not be.
    pub fn new(
        colour: Colour,
        background: Background,
        overrides: &BTreeMap<String, String>,
    ) -> (Theme, Vec<String>) {
        let mut styles: BTreeMap<Role, Style> = Role::ALL
            .iter()
            .map(|role| (*role, base(*role, colour, background)))
            .collect();
        let mut problems = Vec::new();
        for (name, value) in overrides {
            let Some(role) = Role::ALL.iter().find(|role| role.name() == name) else {
                problems.push(format!(
                    "[tui.colors] has no role called \"{name}\"; the roles are {}",
                    Role::ALL.map(Role::name).join(", ")
                ));
                continue;
            };
            match parse_colour(value) {
                // `NO_COLOR` means no colour, including a configured one.
                Some(_) if colour == Colour::None => {}
                Some(colour) => {
                    let style = styles.entry(*role).or_default();
                    *style = match role {
                        Role::Selection => style.bg(colour),
                        _ => style.fg(colour),
                    };
                }
                None => problems.push(format!(
                    "[tui.colors] {name} = \"{value}\" is not a colour; use a name like \"red\", \
                     a palette number, or \"#rrggbb\""
                )),
            }
        }
        (Theme { styles }, problems)
    }

    /// The style for `role`.
    pub fn style(&self, role: Role) -> Style {
        self.styles.get(&role).copied().unwrap_or_default()
    }
}

/// A role's style before any override.
///
/// Without colour, attributes carry the meaning. With the sixteen or 256
/// colours, the terminal's own palette. With true colour, the same, except
/// that the selected row and the focus take Postio's accent.
fn base(role: Role, colour: Colour, background: Background) -> Style {
    let plain = Style::default();
    if colour == Colour::None {
        return match role {
            Role::Selection => plain.add_modifier(Modifier::REVERSED),
            Role::Unread | Role::Focus | Role::Error | Role::Warning => {
                plain.add_modifier(Modifier::BOLD)
            }
            Role::Dim | Role::Quote => plain.add_modifier(Modifier::DIM),
            Role::Link => plain.add_modifier(Modifier::UNDERLINED),
            Role::Text | Role::Accent | Role::Flagged | Role::Code | Role::Success => plain,
        };
    }
    let accent = (colour == Colour::TrueColor).then(|| {
        let (light, dark) = postio_ui::tokens::accent_rgb();
        let (r, g, b) = match background {
            Background::Light => light,
            Background::Dark | Background::Unknown => dark,
        };
        Color::Rgb(r, g, b)
    });
    match role {
        Role::Text => plain,
        Role::Dim | Role::Quote => plain.fg(Color::DarkGray),
        Role::Accent => plain.fg(Color::Blue),
        Role::Selection => plain.bg(accent.unwrap_or(Color::Blue)).fg(Color::White),
        Role::Focus => plain
            .fg(accent.unwrap_or(Color::Cyan))
            .add_modifier(Modifier::BOLD),
        Role::Unread => plain.add_modifier(Modifier::BOLD),
        Role::Flagged | Role::Warning => plain.fg(Color::Yellow),
        Role::Link => plain.fg(Color::Blue).add_modifier(Modifier::UNDERLINED),
        Role::Code => plain.fg(Color::Cyan),
        Role::Error => plain.fg(Color::Red),
        Role::Success => plain.fg(Color::Green),
    }
}

/// A colour as `[tui.colors]` spells it: a name (`red`, `lightblue`), a
/// palette index (`12`), or `#rrggbb`.
fn parse_colour(text: &str) -> Option<Color> {
    Color::from_str(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(colour: Colour, overrides: &[(&str, &str)]) -> (Theme, Vec<String>) {
        let overrides = overrides
            .iter()
            .map(|(role, colour)| ((*role).to_owned(), (*colour).to_owned()))
            .collect();
        Theme::new(colour, Background::Dark, &overrides)
    }

    #[test]
    fn with_no_colour_nothing_is_coloured_and_selection_still_shows() {
        let (theme, _) = theme(Colour::None, &[]);
        for role in Role::ALL {
            let style = theme.style(role);
            assert_eq!(style.fg, None, "{role:?} has a foreground");
            assert_eq!(style.bg, None, "{role:?} has a background");
        }
        assert!(
            theme
                .style(Role::Selection)
                .add_modifier
                .contains(Modifier::REVERSED),
            "a selection still reads as one without colour"
        );
        assert!(
            theme
                .style(Role::Unread)
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn true_colour_uses_postios_accent_for_selection_and_focus_only() {
        let (theme, _) = theme(Colour::TrueColor, &[]);
        let (_, dark) = postio_ui::tokens::accent_rgb();
        let accent = Color::Rgb(dark.0, dark.1, dark.2);
        assert_eq!(theme.style(Role::Selection).bg, Some(accent));
        assert_eq!(theme.style(Role::Focus).fg, Some(accent));
        for role in Role::ALL {
            if !matches!(role, Role::Selection | Role::Focus) {
                let style = theme.style(role);
                assert!(
                    !matches!(style.fg, Some(Color::Rgb(..)))
                        && !matches!(style.bg, Some(Color::Rgb(..))),
                    "{role:?} left the terminal's palette: {style:?}"
                );
            }
        }
    }

    #[test]
    fn below_true_colour_the_accent_falls_back_to_the_palette() {
        let (theme, _) = theme(Colour::Ansi256, &[]);
        assert!(
            !matches!(theme.style(Role::Selection).bg, Some(Color::Rgb(..))),
            "no approximated shade"
        );
        assert!(theme.style(Role::Selection).bg.is_some());
    }

    #[test]
    fn a_role_can_be_overridden_and_a_bad_one_is_reported() {
        let (theme, problems) = theme(
            Colour::Ansi16,
            &[
                ("flagged", "#ff8800"),
                ("sparkle", "red"),
                ("link", "not-a-colour"),
            ],
        );
        assert_eq!(
            theme.style(Role::Flagged).fg,
            Some(Color::Rgb(0xff, 0x88, 0x00))
        );
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|problem| problem.contains("sparkle")));
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("not-a-colour"))
        );
    }
}
