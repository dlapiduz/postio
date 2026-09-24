//! Which panes a terminal of a given size shows.
//!
//! ADR 0024's rule, in a terminal: the width decides what is **shown**, never
//! what the user **asked for**. Narrowing the terminal to one pane and
//! widening it again brings the sidebar back if it was open, because being
//! open is the user's and fitting is the terminal's.
//!
//! The widths are this one table, as the desktop app's are one table in
//! `postio-gtk::shell` (`contracts/tui-surface.md` §Layout).

/// One of the three panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pane {
    /// Folders, views and saved searches.
    Sidebar,
    /// The message list.
    List,
    /// The reading pane, which the composer takes over.
    Reader,
}

/// What the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requested {
    /// Whether the sidebar is open.
    pub sidebar: bool,
    /// Which pane is in front when not all fit: the list or the reader, or
    /// the sidebar when it was asked for where it does not fit beside them.
    pub front: Pane,
}

impl Default for Requested {
    fn default() -> Self {
        Requested {
            sidebar: true,
            front: Pane::List,
        }
    }
}

/// What a terminal of this size shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    /// These panes, left to right.
    Panes(Vec<Pane>),
    /// Nothing fits; say so and draw nothing else.
    TooSmall {
        /// The smallest size that works.
        needs: (u16, u16),
    },
}

/// Rows the screen gives to the top bar and the status line.
pub const CHROME_ROWS: u16 = 2;
/// Lines one list row takes.
pub const LIST_ROW_LINES: u16 = 2;

/// Narrower than this, or shorter, and nothing is drawn but the sentence.
pub const MINIMUM: (u16, u16) = (50, 12);
/// From this width the sidebar fits beside the list and reader.
pub const THREE_PANES: u16 = 140;
/// From this width the list and the reader fit side by side.
pub const TWO_PANES: u16 = 90;

/// The panes shown in a `width` × `height` terminal, given what was asked.
pub fn shown(width: u16, height: u16, requested: Requested) -> Shown {
    if width < MINIMUM.0 || height < MINIMUM.1 {
        return Shown::TooSmall { needs: MINIMUM };
    }
    let panes = if width >= THREE_PANES && (requested.sidebar || requested.front == Pane::Sidebar) {
        vec![Pane::Sidebar, Pane::List, Pane::Reader]
    } else if requested.front == Pane::Sidebar && width >= TWO_PANES {
        vec![Pane::Sidebar, Pane::List]
    } else if width >= TWO_PANES {
        vec![Pane::List, Pane::Reader]
    } else {
        vec![requested.front]
    };
    Shown::Panes(panes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_terminal_shows_all_three() {
        assert_eq!(
            shown(160, 40, Requested::default()),
            Shown::Panes(vec![Pane::Sidebar, Pane::List, Pane::Reader])
        );
    }

    #[test]
    fn a_sidebar_asked_for_in_front_shows_where_three_panes_do_not_fit() {
        let asked = Requested {
            front: Pane::Sidebar,
            ..Requested::default()
        };
        assert_eq!(
            shown(100, 30, asked),
            Shown::Panes(vec![Pane::Sidebar, Pane::List])
        );
        assert_eq!(shown(60, 30, asked), Shown::Panes(vec![Pane::Sidebar]));
        assert_eq!(
            shown(160, 30, asked),
            Shown::Panes(vec![Pane::Sidebar, Pane::List, Pane::Reader]),
            "where it fits anyway, nothing changes"
        );
    }

    #[test]
    fn a_closed_sidebar_stays_closed_however_wide() {
        let requested = Requested {
            sidebar: false,
            ..Requested::default()
        };
        assert_eq!(
            shown(200, 40, requested),
            Shown::Panes(vec![Pane::List, Pane::Reader])
        );
    }

    #[test]
    fn a_middling_terminal_drops_the_sidebar_but_keeps_list_and_reader() {
        assert_eq!(
            shown(100, 30, Requested::default()),
            Shown::Panes(vec![Pane::List, Pane::Reader])
        );
    }

    #[test]
    fn a_narrow_terminal_shows_only_the_pane_in_front() {
        let reading = Requested {
            front: Pane::Reader,
            ..Requested::default()
        };
        assert_eq!(shown(60, 30, reading), Shown::Panes(vec![Pane::Reader]));
        assert_eq!(
            shown(60, 30, Requested::default()),
            Shown::Panes(vec![Pane::List])
        );
    }

    #[test]
    fn too_small_says_how_big_it_needs_to_be() {
        assert_eq!(
            shown(40, 30, Requested::default()),
            Shown::TooSmall { needs: MINIMUM }
        );
        assert_eq!(
            shown(80, 10, Requested::default()),
            Shown::TooSmall { needs: MINIMUM }
        );
    }
}
