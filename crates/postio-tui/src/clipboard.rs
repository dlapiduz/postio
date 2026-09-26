//! Reading an image off the clipboard, on the paste key and never otherwise
//! (FR-026, research R7).
//!
//! A terminal's own paste carries text only, so an image has to be asked for.
//! The sources are tried in order -- `arboard` (Wayland data-control, X11),
//! then `wl-paste`, then `xclip` -- because a compositor without the
//! data-control protocol answers only the command-line tools. The first that
//! can reach a clipboard answers; one that reaches it and finds no image is an
//! answer too, and ends the search.

use std::process::{Command, Stdio};

/// What a clipboard source found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// An image, as PNG bytes.
    Image(Vec<u8>),
    /// The clipboard was reached and holds no image.
    NoImage,
    /// This source cannot reach a clipboard here.
    Unreachable,
}

/// One way of reading the clipboard.
pub trait Source {
    /// Ask for an image.
    fn image(&mut self) -> Found;
}

/// What the chain says when nothing could reach a clipboard: over SSH, or
/// with no tool installed. Drops and text pastes still work.
pub const UNAVAILABLE: &str = "Clipboard unavailable here";

/// Ask each source in turn; the first that reaches a clipboard answers.
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

/// The sources this machine has, in order.
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

/// `arboard`: the data-control protocol on Wayland, the selection on X11.
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

/// RGBA pixels as a PNG file.
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

/// A command-line tool that writes the clipboard's PNG to its output.
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
            // Not installed, or no display to talk to.
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
    /// Whether this tool has a clipboard to talk to at all.
    fn reaches_a_clipboard(&self) -> bool {
        match self.program {
            "wl-paste" => std::env::var_os("WAYLAND_DISPLAY").is_some(),
            _ => std::env::var_os("DISPLAY").is_some(),
        }
    }
}

/// A PNG file's first bytes.
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

    /// How many times a fake source was asked.
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
