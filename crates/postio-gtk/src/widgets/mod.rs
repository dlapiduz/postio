//! The controls every surface draws, built once.
//!
//! Postio had three of these in three or four hand-rolled copies each — a
//! button with its key beside it, a row of them, a one-line notice. Copies
//! drift: only one of the three keycap implementations read the live keymap,
//! so the other two claimed keys a rebind had already moved.
//!
//! What lives here is the *drawing*. The rules these draw — which
//! participants fit on a line, for one — are in `postio_ui::conversation`,
//! where they can be proven without a display and reached by a second
//! frontend.
//!
//! The fourth control the canvas asks for, the mark that says which of four
//! states a row is in (turn 8b), is **not here yet**: both surfaces that
//! draw it are single-`snapshot()` widgets and neither is built, and a
//! mechanism wired to nothing is what `check-uncalled-pub-fn` exists to
//! catch. It lands with the collapsed conversation row it belongs to.

// The controls both desktop apps draw moved to postio-widgets (ADR 0043).
// Re-exported under their old paths, so every surface here that names
// `crate::widgets::keyhint` or `crate::widgets::ActionBar` is unchanged.
pub use postio_widgets::widgets::{
    action_bar, button, checkrow, chip, chrome, field, keycap, keyhint, nav_row, notes, notice,
    plate, screen, segmented, settings_group, space,
};

pub use action_bar::{Action, ActionBar};
pub use button::{Kind, Size, icon_button};
pub use checkrow::CheckRow;
pub use chip::{chip_button, filter_chip};
pub use chrome::{kicker, stat_line};
pub use keycap::KeycapButton;
pub use keyhint::KeyLine;
pub use notes::{ListOrEmpty, callout, empty_note};
pub use notice::{NoticeBar, NoticeMenuItem};
pub use postio_widgets::widgets::{nav_count, nav_name};
pub use screen::under_window_chrome;
pub use segmented::SegmentedControl;
pub use settings_group::SettingsGroup;

#[cfg(test)]
mod kicker_css {
    //! The kicker's look belongs to the generated tokens, and the classic
    //! app's shell.css may inset a kicker but not restyle it. Here rather
    //! than beside `chrome::kicker` since the widget moved to
    //! postio-widgets: both stylesheets are this crate's.

    /// Declarations that decide what a kicker looks like, as opposed to where
    /// one sits. A contextual rule may inset a kicker; it may not restyle it.
    const LOOK: [&str; 6] = [
        "font-family",
        "font-size",
        "font-weight",
        "letter-spacing",
        "text-transform",
        "color",
    ];

    #[test]
    fn the_kicker_is_styled_once_by_the_generated_tokens() {
        let shell = include_str!("../../data/shell.css");
        let tokens = include_str!("../../data/tokens.css");

        let restyled: Vec<String> = rules(shell)
            .filter(|(selector, _)| selector.contains(".postio-kicker"))
            .filter(|(_, body)| looks(body))
            .map(|(selector, _)| selector.trim().to_owned())
            .collect();
        assert!(
            restyled.is_empty(),
            "shell.css restyles the kicker the token generator owns: {restyled:?}"
        );

        let defined = rules(tokens)
            .filter(|(selector, body)| selector.contains(".postio-kicker") && looks(body))
            .count();
        assert!(defined > 0, "tokens.css must define the kicker");
    }

    #[test]
    fn a_type_role_is_named_not_retyped() {
        let shell = include_str!("../../data/shell.css");
        let retyped: Vec<&str> = postio_ui::tokens::TYPE_ROLES
            .iter()
            .filter(|(_, size)| shell.contains(&format!("font-size: {size};")))
            .map(|(role, _)| *role)
            .collect();
        assert!(
            retyped.is_empty(),
            "shell.css retypes these roles' sizes instead of var(--postio-text-…): {retyped:?}"
        );
        assert!(
            !shell.contains("0.8863rem"),
            "13px is 0.8864rem; 0.8863rem is a second, rounded-differently copy"
        );
    }

    fn looks(body: &str) -> bool {
        body.split(';').any(|declaration| {
            let property = declaration.split(':').next().unwrap_or("").trim();
            LOOK.contains(&property)
        })
    }

    /// `(selector, declarations)` for each rule, comments dropped.
    fn rules(css: &str) -> impl Iterator<Item = (String, String)> + '_ {
        let mut text = String::with_capacity(css.len());
        let mut rest = css;
        while let Some(start) = rest.find("/*") {
            text.push_str(&rest[..start]);
            rest = rest[start..]
                .find("*/")
                .map_or("", |end| &rest[start + end + 2..]);
        }
        text.push_str(rest);
        text.split('}')
            .filter_map(|rule| {
                let (selector, body) = rule.split_once('{')?;
                Some((selector.to_owned(), body.to_owned()))
            })
            .collect::<Vec<_>>()
            .into_iter()
    }
}
