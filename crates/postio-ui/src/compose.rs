//! What the composer says about itself.
//!
//! Two frontends write mail and both have a footer that names what will
//! leave. The words are here so they cannot differ, for the same reason the
//! reader's document assembly is shared: a claim about what Postio puts on
//! the wire is one it should be willing to make identically everywhere.

/// The MIME shape an outgoing message will have: `html + text/plain`, or
/// `text/plain, format=flowed`.
///
/// **The plain part is not optional.** Rich mail sends `text/html` *and* a
/// `text/plain` alternative, always — a recipient reading in a terminal, a
/// screen reader, or a client that refuses HTML gets the message rather than
/// an apology. Plain mail is wrapped at 72 columns and flowed (RFC 3676), so
/// it reads correctly whether the receiving client rewraps it or not.
pub fn outgoing_shape(rich: bool) -> String {
    if rich {
        "html + text/plain".to_owned()
    } else {
        "text/plain, format=flowed".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_always_carries_a_plain_alternative() {
        // The half that is easy to drop and expensive to notice: it is
        // invisible to the sender and it is the whole message to some
        // recipients.
        assert!(outgoing_shape(true).contains("text/plain"));
    }

    #[test]
    fn plain_says_it_is_flowed_rather_than_just_plain() {
        // `format=flowed` is the difference between a paragraph that rewraps
        // in a narrow window and one that arrives with a ragged 72-column
        // edge, and the footer is where a person can see which they chose.
        assert_eq!(outgoing_shape(false), "text/plain, format=flowed");
    }
}
