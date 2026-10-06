//! Where the window's parts sit in a terminal of `W × H` cells.
//!
//! Every size here comes from the terminal's width and height and nothing
//! else, so stepping through mail never moves a frame (terminal.md,
//! "Units"). The window is the top bar, the strip, an optional banner, the
//! list and the bottom line; the open message's overlay and the reading pane
//! are functions of the width.

use ratatui::layout::Rect;

/// Narrower than this, or shorter, and nothing is drawn but the sentence.
pub const MINIMUM: (u16, u16) = (50, 12);

/// The reading pane sits beside the list from this width.
pub const PANE_FROM: u16 = 128;

/// Columns the list keeps beside a reading pane.
const PANE_LIST: u16 = 56;

/// The widest the reading pane gets.
const PANE_WIDEST: u16 = 100;

/// The overlay frame's narrowest and widest.
const FRAME_NARROWEST: u16 = 76;
const FRAME_WIDEST: u16 = 100;

/// Below this width the overlay takes the whole width.
const FRAME_WHOLE: u16 = 80;

/// Whether a terminal of `width` × `height` has room for the window.
pub fn fits(width: u16, height: u16) -> bool {
    width >= MINIMUM.0 && height >= MINIMUM.1
}

/// The rows of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// The top bar: Compose, the command field, the sync label, `? keys`.
    pub top: Rect,
    /// The strip: place, counts, the has-action toggle, filtered and rules.
    pub strip: Rect,
    /// The banner, when one is shown.
    pub banner: Option<Rect>,
    /// Day headings and rows: everything else.
    pub list: Rect,
    /// The bulk bar, the toast, a pending chord, or nothing.
    pub bottom: Rect,
}

/// The window's rows in `area`, with the banner row when `banner` is shown.
pub fn window(area: Rect, banner: bool) -> Window {
    let row = |offset: u16| Rect::new(area.x, area.y + offset.min(area.height), area.width, 1);
    let banner_rows = u16::from(banner);
    let list_height = area.height.saturating_sub(3 + banner_rows);
    Window {
        top: row(0),
        strip: row(1),
        banner: banner.then(|| row(2)),
        list: Rect::new(area.x, area.y + 2 + banner_rows, area.width, list_height),
        bottom: row(area.height.saturating_sub(1)),
    }
}

/// The open message's frame width: `clamp(76, W − 2·max(4, ⌊0.12·W⌋), 100)`,
/// or the whole width below 80 columns.
pub fn frame_width(width: u16) -> u16 {
    if width < FRAME_WHOLE {
        return width;
    }
    let margin = 4.max(width * 12 / 100);
    width
        .saturating_sub(2 * margin)
        .clamp(FRAME_NARROWEST, FRAME_WIDEST)
}

/// The frame holding the open message in a terminal of `area`: every row but
/// the top bar and the bottom line, `frame_width` wide and centred.
pub fn open_frame(area: Rect) -> Rect {
    let width = frame_width(area.width).min(area.width);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + 1,
        width,
        area.height.saturating_sub(2),
    )
}

/// Below this many columns of frame, Label, Move and Delete fold into More.
const MORE_BELOW: u16 = 96;

/// Whether a frame `width` columns wide folds Label, Move and Delete into
/// More.
pub fn folds_into_more(width: u16) -> bool {
    width < MORE_BELOW
}

/// The scrolling column's width in a frame or pane `width` wide:
/// `min(72, width − 8)`.
pub fn column_width(width: u16) -> u16 {
    72.min(width.saturating_sub(8))
}

/// The reading pane's width beside the list: `min(100, W − 56)` from 128
/// columns, and none below.
pub fn pane_width(width: u16) -> Option<u16> {
    (width >= PANE_FROM).then(|| PANE_WIDEST.min(width - PANE_LIST))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_a_top_bar_a_strip_a_list_and_a_bottom_line() {
        let window = window(Rect::new(0, 0, 120, 36), false);
        assert_eq!(window.top, Rect::new(0, 0, 120, 1));
        assert_eq!(window.strip, Rect::new(0, 1, 120, 1));
        assert_eq!(window.banner, None);
        assert_eq!(window.list, Rect::new(0, 2, 120, 33));
        assert_eq!(window.bottom, Rect::new(0, 35, 120, 1));
    }

    #[test]
    fn a_banner_takes_one_row_from_the_list() {
        let window = window(Rect::new(0, 0, 120, 36), true);
        assert_eq!(window.banner, Some(Rect::new(0, 2, 120, 1)));
        assert_eq!(window.list, Rect::new(0, 3, 120, 32));
    }

    #[test]
    fn the_frame_is_92_wide_at_120_and_100_from_136() {
        assert_eq!(frame_width(120), 92);
        assert_eq!(frame_width(136), 100);
        assert_eq!(frame_width(200), 100);
        assert_eq!(frame_width(80), 76, "never narrower than 76 from 80");
        assert_eq!(frame_width(79), 79, "the whole width below 80");
        assert_eq!(frame_width(50), 50);
    }

    #[test]
    fn the_frame_covers_every_row_but_the_top_bar_and_the_bottom_line() {
        let frame = open_frame(Rect::new(0, 0, 120, 36));
        assert_eq!(frame, Rect::new(14, 1, 92, 34));
        assert_eq!(open_frame(Rect::new(0, 0, 60, 14)), Rect::new(0, 1, 60, 12));
    }

    #[test]
    fn the_column_is_72_or_the_frame_less_8_and_the_action_row_folds_below_96() {
        assert_eq!(column_width(92), 72);
        assert_eq!(column_width(76), 68);
        assert_eq!(column_width(100), 72);
        assert!(folds_into_more(92));
        assert!(!folds_into_more(96));
    }

    #[test]
    fn the_pane_comes_at_128_columns_and_is_never_wider_than_100() {
        assert_eq!(pane_width(127), None);
        assert_eq!(pane_width(128), Some(72));
        assert_eq!(pane_width(156), Some(100));
        assert_eq!(pane_width(300), Some(100));
    }

    #[test]
    fn under_the_minimum_nothing_fits() {
        assert!(fits(50, 12));
        assert!(!fits(49, 12));
        assert!(!fits(50, 11));
    }
}
