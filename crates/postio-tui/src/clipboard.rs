//! Reads an image off the clipboard, only on the paste key. Sources are tried in order (`arboard`,
//! `wl-paste`, `xclip`) because a compositor without data-control answers only the tools; the
//! first that reaches a clipboard answers, even with no image.

use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Image(Vec<u8>),
    /// The clipboard was reached and holds no image.
    NoImage,
    /// This source cannot reach a clipboard here.
    Unreachable,
}

pub trait Source {
    fn image(&mut self) -> Found;
}

/// Nothing could reach a clipboard (over SSH, or no tool installed).
pub const UNAVAILABLE: &str = "Clipboard unavailable here";

pub fn read(sources: &mut [Box<dyn Source>]) -> Result<Option<Vec<u8>>, String> {
    for source in sources {
        match source.image() {
            Found::Image(png) => return Ok(Some(png)),
            Found::NoImage => return Ok(None),
            Found::Unreachable => {}
        }
    }
    Err(UNAVAILABLE.to_owned())
}

pub fn system() -> Vec<Box<dyn Source>> {
    vec![
        Box::new(Arboard),
        Box::new(Tool {
            program: "wl-paste",
            arguments: &["--no-newline", "--type", "image/png"],
        }),
        Box::new(Tool {
            program: "xclip",
            arguments: &["-selection", "clipboard", "-target", "image/png", "-out"],
        }),
    ]
}

struct Arboard;

impl Source for Arboard {
    fn image(&mut self) -> Found {
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return Found::Unreachable;
        };
        match clipboard.get_image() {
            Ok(image) => match png_of(image.width, image.height, &image.bytes) {
                Some(png) => Found::Image(png),
                None => Found::NoImage,
            },
            Err(arboard::Error::ContentNotAvailable) => Found::NoImage,
            Err(_) => Found::Unreachable,
        }
    }
}

fn png_of(width: usize, height: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    let pixels = image::RgbaImage::from_raw(
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
        rgba.to_vec(),
    )?;
    let mut png = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

struct Tool {
    program: &'static str,
    arguments: &'static [&'static str],
}

impl Source for Tool {
    fn image(&mut self) -> Found {
        let output = Command::new(self.program)
            .args(self.arguments)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match output {
            Err(_) => Found::Unreachable,
            Ok(output) if output.status.success() && output.stdout.starts_with(PNG) => {
                Found::Image(output.stdout)
            }
            // It ran and there is no PNG to give: either no image, or no
            // clipboard server. Only the first is an answer.
            Ok(_) if self.reaches_a_clipboard() => Found::NoImage,
            Ok(_) => Found::Unreachable,
        }
    }
}

impl Tool {
    fn reaches_a_clipboard(&self) -> bool {
        match self.program {
            "wl-paste" => std::env::var_os("WAYLAND_DISPLAY").is_some(),
            _ => std::env::var_os("DISPLAY").is_some(),
        }
    }
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Found, std::rc::Rc<std::cell::Cell<u32>>);

    impl Source for Fake {
        fn image(&mut self) -> Found {
            self.1.set(self.1.get() + 1);
            self.0.clone()
        }
    }

    type Asked = std::rc::Rc<std::cell::Cell<u32>>;

    fn chain(found: &[Found]) -> (Vec<Box<dyn Source>>, Vec<Asked>) {
        let counters: Vec<_> = found.iter().map(|_| std::rc::Rc::default()).collect();
        let sources = found
            .iter()
            .zip(&counters)
            .map(|(found, count)| {
                Box::new(Fake(found.clone(), std::rc::Rc::clone(count))) as Box<dyn Source>
            })
            .collect();
        (sources, counters)
    }

    #[test]
    fn the_first_source_that_reaches_a_clipboard_answers() {
        let (mut sources, asked) = chain(&[
            Found::Unreachable,
            Found::Image(b"png".to_vec()),
            Found::NoImage,
        ]);
        assert_eq!(read(&mut sources), Ok(Some(b"png".to_vec())));
        assert_eq!(asked[2].get(), 0, "no need to ask the third");
    }

    #[test]
    fn a_clipboard_with_no_image_is_an_answer() {
        let (mut sources, asked) = chain(&[Found::NoImage, Found::Image(b"png".to_vec())]);
        assert_eq!(read(&mut sources), Ok(None));
        assert_eq!(asked[1].get(), 0);
    }

    #[test]
    fn nothing_reaching_a_clipboard_says_so() {
        let (mut sources, _) = chain(&[Found::Unreachable, Found::Unreachable]);
        assert_eq!(read(&mut sources), Err(UNAVAILABLE.to_owned()));
    }

    #[test]
    fn pixels_become_a_png_file() {
        let png = png_of(1, 1, &[255, 0, 0, 255]).expect("encoded");
        assert!(png.starts_with(PNG));
    }
}
