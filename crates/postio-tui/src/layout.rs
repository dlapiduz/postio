//! Where the window's parts sit in a terminal of `W x H` cells. Every size comes from the
//! terminal's width and height alone, so stepping through mail never moves a frame.

use ratatui::layout::Rect;

pub const MINIMUM: (u16, u16) = (50, 12);

pub const PANE_FROM: u16 = 128;

const PANE_LIST: u16 = 56;

const PANE_WIDEST: u16 = 100;

const FRAME_NARROWEST: u16 = 76;
const FRAME_WIDEST: u16 = 100;

const FRAME_WHOLE: u16 = 80;

pub fn fits(width: u16, height: u16) -> bool {
    width >= MINIMUM.0 && height >= MINIMUM.1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub top: Rect,
    pub strip: Rect,
    pub banner: Option<Rect>,
    pub list: Rect,
    pub bottom: Rect,
}

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

pub fn frame_width(width: u16) -> u16 {
    if width < FRAME_WHOLE {
        return width;
    }
    let margin = 4.max(width * 12 / 100);
    width
        .saturating_sub(2 * margin)
        .clamp(FRAME_NARROWEST, FRAME_WIDEST)
}

pub fn open_frame(area: Rect) -> Rect {
    let width = frame_width(area.width).min(area.width);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + 1,
        width,
        area.height.saturating_sub(2),
    )
}

const MORE_BELOW: u16 = 96;

pub fn folds_into_more(width: u16) -> bool {
    width < MORE_BELOW
}

pub fn column_width(width: u16) -> u16 {
    72.min(width.saturating_sub(8))
}

/// `None` where there is no room for a pane.
pub fn split_pane(list: Rect) -> Option<(Rect, Rect)> {
    let pane = pane_width(list.width)?;
    Some((
        Rect::new(list.x, list.y, list.width - pane, list.height),
        Rect::new(list.x + list.width - pane, list.y, pane, list.height),
    ))
}

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
    fn the_list_keeps_at_least_56_beside_the_pane() {
        let (list, pane) = split_pane(Rect::new(0, 2, 128, 30)).unwrap();
        assert_eq!((list.width, pane.width, pane.x), (56, 72, 56));
        let (list, pane) = split_pane(Rect::new(0, 2, 200, 30)).unwrap();
        assert_eq!((list.width, pane.width), (100, 100));
        assert!(split_pane(Rect::new(0, 2, 127, 30)).is_none());
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
