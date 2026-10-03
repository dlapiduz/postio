//! Focus's action rows as buttons: the verbs are `postio_ui::focus_dialog`'s,
//! and this dresses each as an [`Action`] a bar can build.

use std::sync::OnceLock;

use postio_ui::focus_dialog::Verb;
use postio_widgets::widgets::Action;

/// What identifies a table: its prefix, its first slug and its length.
type Key = (&'static str, Option<&'static str>, usize);

/// The tables dressed so far.
type Dressed = std::sync::Mutex<Vec<(Key, &'static [Action])>>;

/// `verbs` as actions whose CSS class is `prefix` and the verb's slug
/// (`focus-open-reply-all`). An `Action` holds `&'static str`s, so each
/// distinct table is dressed once, the first time it is asked for, and kept.
pub fn actions(prefix: &'static str, verbs: &'static [Verb]) -> &'static [Action] {
    static DRESSED: OnceLock<Dressed> = OnceLock::new();
    let key = (prefix, verbs.first().map(|verb| verb.slug), verbs.len());
    let mut dressed = DRESSED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((_, found)) = dressed.iter().find(|(held, _)| *held == key) {
        return found;
    }
    let built: Vec<Action> = verbs
        .iter()
        .map(|verb| {
            let class: &'static str = Box::leak(format!("{prefix}{}", verb.slug).into_boxed_str());
            Action::new(verb.command, verb.label, class)
        })
        .collect();
    let built: &'static [Action] = Box::leak(built.into_boxed_slice());
    dressed.push((key, built));
    built
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_is_dressed_once_with_its_slugs_in_the_class() {
        let first = actions("focus-open-", postio_ui::focus_dialog::OPEN_TOOLBAR);
        let again = actions("focus-open-", postio_ui::focus_dialog::OPEN_TOOLBAR);
        assert!(std::ptr::eq(first, again));
        assert_eq!(first[1].class, "focus-open-reply-all");
        assert_eq!(first[1].label, "Reply all");
    }
}
