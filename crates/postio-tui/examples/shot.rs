//! Render the terminal frontend with sample mail as an SVG, to judge the look as a picture.
//! `cargo run -p postio-tui --example shot -- /tmp/tui.svg [width] [height] [state]`;
//! `state` names a `test_support::sample` screen, with an optional `-nocolor` suffix.

use postio_tui::caps::Background;
use postio_tui::test_support;
use postio_tui::theme::Theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .unwrap_or_else(|| "/tmp/postio-tui.svg".to_owned());
    let width: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(160);
    let height: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(42);
    let state = args.next().unwrap_or_default();
    let (app, colour) = test_support::sample::state(&state, width, height);
    let (theme, _) = Theme::new(colour, Background::Dark, &Default::default());
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let local = test_support::now();
    terminal
        .draw(|frame| {
            postio_tui::view::draw(frame, &app, &theme, local);
        })
        .unwrap();
    std::fs::write(&out, svg(terminal.backend().buffer())).unwrap();
    eprintln!("wrote {out}");
}

const BG: &str = "#1e1e24";
const FG: &str = "#deddda";

fn rgb(color: Color, fallback: &str) -> String {
    match color {
        Color::Reset => fallback.to_owned(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => "#1e1e24".into(),
        Color::Red => "#e5484d".into(),
        Color::Green => "#46a758".into(),
        Color::Yellow => "#f5d90a".into(),
        Color::Blue => "#3e63dd".into(),
        Color::Magenta => "#ab4aba".into(),
        Color::Cyan => "#05a2c2".into(),
        Color::Gray => "#9a9996".into(),
        Color::DarkGray => "#5e5c64".into(),
        Color::LightRed => "#ff6369".into(),
        Color::LightGreen => "#63c174".into(),
        Color::LightYellow => "#ffe629".into(),
        Color::LightBlue => "#849dff".into(),
        Color::LightMagenta => "#d19dff".into(),
        Color::LightCyan => "#4ccce6".into(),
        Color::White => "#ffffff".into(),
        Color::Indexed(i) => {
            let named = [
                "#1e1e24", "#e5484d", "#46a758", "#f5d90a", "#3e63dd", "#ab4aba", "#05a2c2",
                "#deddda", "#5e5c64", "#ff6369", "#63c174", "#ffe629", "#849dff", "#d19dff",
                "#4ccce6", "#ffffff",
            ];
            if (i as usize) < named.len() {
                named[i as usize].into()
            } else if i >= 232 {
                let v = 8 + (i - 232) * 10;
                format!("#{v:02x}{v:02x}{v:02x}")
            } else {
                let i = i - 16;
                let level = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
                format!(
                    "#{:02x}{:02x}{:02x}",
                    level(i / 36),
                    level((i / 6) % 6),
                    level(i % 6)
                )
            }
        }
    }
}

fn svg(buffer: &ratatui::buffer::Buffer) -> String {
    const W: f32 = 9.0;
    const H: f32 = 19.0;
    let area = buffer.area;
    let mut out = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{}' height='{}' font-family='DejaVu Sans Mono, monospace' font-size='15'>\
         <rect width='100%' height='100%' fill='{BG}'/>",
        f32::from(area.width) * W,
        f32::from(area.height) * H
    );
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = &buffer[(x, y)];
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (mut fg, mut bg) = (rgb(cell.fg, FG), rgb(cell.bg, BG));
            if reversed {
                std::mem::swap(&mut fg, &mut bg);
            }
            if bg != BG {
                out += &format!(
                    "<rect x='{}' y='{}' width='{W}' height='{H}' fill='{bg}'/>",
                    f32::from(x) * W,
                    f32::from(y) * H
                );
            }
            let symbol = cell.symbol();
            if symbol.trim().is_empty() {
                continue;
            }
            let weight = if cell.modifier.contains(Modifier::BOLD) {
                "bold"
            } else {
                "normal"
            };
            let style = if cell.modifier.contains(Modifier::ITALIC) {
                "italic"
            } else {
                "normal"
            };
            let fg = if cell.modifier.contains(Modifier::DIM) {
                format!("{fg}99")
            } else {
                fg
            };
            let under = if cell.modifier.contains(Modifier::UNDERLINED) {
                " text-decoration='underline'"
            } else {
                ""
            };
            let escaped = symbol
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            out += &format!(
                "<text x='{}' y='{}' fill='{fg}' font-weight='{weight}' font-style='{style}'{under}>{escaped}</text>",
                f32::from(x) * W,
                f32::from(y) * H + 14.0
            );
        }
    }
    out + "</svg>"
}
