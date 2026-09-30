//! The two labels the settings surfaces write over and over.
//!
//! Neither is a control, and neither is worth a struct: a kicker is a label
//! with a class and a stat line is a label with a different class. They are
//! here rather than inline in each pane because the *classes* are the shared
//! thing — a kicker that is 0.7rem in one pane and 0.75rem in the next is
//! precisely the drift `widgets/` exists to stop.

use adw::prelude::*;

/// A section heading in letterspaced capitals: `THEME`, `MESSAGE LIST`,
/// `SCOPES REQUESTED`.
///
/// Written in sentence case at the call site (`kicker("Message list")`) and
/// drawn in capitals by the stylesheet, so no pane has to remember which case
/// its neighbours shouted in. The look is the canvas' section heading --
/// 10px Barlow Condensed, 0.18em, faint -- and it is defined exactly once, by
/// the token generator in `postio_ui::tokens`; `shell.css` may inset a
/// kicker but not restyle it.
///
/// Not a `<h2>`: it labels the group beneath it for a sighted reader, and
/// the group itself carries the accessible name a screen reader uses. Two
/// announcements of one heading is worse than one.
pub fn kicker(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("postio-kicker");
    label.set_xalign(0.0);
    label
}

/// A line of mono facts under a group: `26px rows · 41 per screen`,
/// `imap · password · 4291 msg · 1.8 GB · synced 12s`.
///
/// Mono because it is almost entirely number, and numbers in a column that
/// do not line up read as a mistake.
pub fn stat_line(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("postio-stat-line");
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}

#[cfg(test)]
mod tests {
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
