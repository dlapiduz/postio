//! Postio's own links (spec 007 FR-185, US15): `postio://message/<id>`
//! names a message in this machine's store, by its local id. Opening one
//! goes to the message and does nothing else; a link that names nothing
//! Postio can find is refused with a message.

use postio_model::MessageId;

/// The scheme the desktop entry registers (`x-scheme-handler/postio`).
pub const SCHEME: &str = "postio";

/// What a refused link says.
pub const UNKNOWN: &str = "Postio can\u{2019}t open that link";

/// What a link to a message no longer here says.
pub const GONE: &str = "That message isn\u{2019}t on this computer";

/// The link to `message`.
pub fn message_uri(message: MessageId) -> String {
    format!("{SCHEME}://message/{}", message.get())
}

/// The message `uri` names, when it is a `postio://message/<id>` link.
pub fn message(uri: &str) -> Option<MessageId> {
    let (scheme, rest) = uri.split_once("://")?;
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    let id = rest.strip_prefix("message/")?.trim_end_matches('/');
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let id: i64 = id.parse().ok()?;
    (id > 0).then(|| MessageId::new(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_link_names_its_message_and_nothing_else_does() {
        let id = MessageId::new(42);
        assert_eq!(message(&message_uri(id)), Some(id));
        assert_eq!(
            message("postio://message/42/"),
            Some(id),
            "a trailing slash"
        );
        assert_eq!(
            message("POSTIO://message/42"),
            Some(id),
            "the scheme in any case"
        );
        for refused in [
            "postio://message/",
            "postio://message/abc",
            "postio://message/-3",
            "postio://message/0",
            "postio://thread/42",
            "https://example.com/message/42",
            "postio:message/42",
            "",
        ] {
            assert_eq!(message(refused), None, "{refused:?}");
        }
    }
}
