//! `text/*`: decoded by its byte-order mark, else its declared charset,
//! else UTF-8 when it is valid, else Windows-1252; one unit per line.

use encoding_rs::Encoding;

use crate::Location;
use crate::limits::{Budget, Stop};

/// How many lines pass between two looks at the clock.
const CLOCK_EVERY: usize = 256;

pub(crate) fn extract(
    bytes: &[u8],
    charset: Option<&str>,
    budget: &mut Budget<'_>,
) -> Result<(), Stop> {
    let declared = charset.and_then(|label| Encoding::for_label(label.trim().as_bytes()));
    let encoding = match Encoding::for_bom(bytes) {
        Some((found, _)) => found,
        None => match declared {
            Some(found) => found,
            None if std::str::from_utf8(bytes).is_ok() => encoding_rs::UTF_8,
            None => encoding_rs::WINDOWS_1252,
        },
    };
    // `decode` strips a BOM that matches and replaces malformed sequences:
    // a stray byte costs a character, never the file.
    let (text, _, _) = encoding.decode(bytes);
    for (index, line) in text.lines().enumerate() {
        if index.is_multiple_of(CLOCK_EVERY) {
            budget.check_time()?;
        }
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        budget.push(Location::Line(number), line)?;
    }
    Ok(())
}
