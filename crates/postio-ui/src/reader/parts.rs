//! Resolving a `Content-ID` to the bytes it names.
//!
//! Toolkit-free and frontend-free on purpose. The GTK reader resolves `cid:`
//! through a registered URI scheme and a `WKWebView` will resolve it through
//! something else entirely, but *what a `Content-ID` may resolve to* is a
//! security property rather than a rendering detail, so it is stated once
//! here and both frontends are handed the same answer (ADR 0019 Q6, #608).

/// Resolves a `Content-ID` to its bytes and MIME type.
///
/// A `Content-ID` is passed exactly as `postio_body::sanitize::percent_decode`
/// recovered it: without the `cid:` prefix and without the angle brackets some
/// senders wrap it in (`sanitize_body` already strips those before encoding).
///
/// Synchronous and local by design — an implementation is a blob-store read or
/// a lookup into whatever the caller already has in memory for the open
/// message, never a call that blocks on I/O the reader would have to await.
/// That is not only about latency: a `cid:` that could reach the network would
/// be the tracking pixel the reader spends so much effort blocking, arriving
/// through the back door.
pub trait BlobSource {
    /// The part's bytes and MIME type, or `None` if no part carries this id.
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)>;

    /// The same, for a reference that names *which message* it belongs to.
    ///
    /// A single-message document does not need to: the reader knows which
    /// message is open, so `scope` is `None` and this is [`resolve`]. ADR
    /// 0032's conversation document holds a whole thread, where "whichever is
    /// open" names nothing — two messages may each carry a part called `logo`,
    /// and a sender may reference a `Content-ID` they know belongs to somebody
    /// else's message in the same thread.
    ///
    /// The default ignores the scope, which is right for a source that only
    /// ever holds one message's parts and wrong for one that holds a thread's.
    /// A thread source must override it, and a scope it does not recognise
    /// must resolve to nothing rather than to whatever it has.
    ///
    /// [`resolve`]: Self::resolve
    fn resolve_in(&self, scope: Option<&str>, content_id: &str) -> Option<(Vec<u8>, String)> {
        let _ = scope;
        self.resolve(content_id)
    }
}

impl<F: Fn(&str) -> Option<(Vec<u8>, String)>> BlobSource for F {
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)> {
        self(content_id)
    }
}

/// A closure is a blob source, and one that holds a single message's parts
/// answers the same whatever scope is asked for.
// ---------------------------------------------------------------------------
// The part tree: what came with a message, read as the tree its part ids
// describe. Toolkit-free, so both desktop apps' attachment chips and the
// classic app's parts panel read one answer (moved from postio-gtk's
// parts.rs, specs/007-postio-focus T019).
// ---------------------------------------------------------------------------
use postio_model::Attachment;
use postio_model::ids::AttachmentId;

use crate::format::human_size;

/// One node of the tree, flattened into the order the keyboard walks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The IMAP part id — `2.1`. Empty for the synthetic root.
    pub part_id: String,
    /// How deep it sits; the root is 0.
    pub depth: usize,
    /// `text/html`, `image/png`, `multipart/mixed`.
    pub mime: String,
    /// The name the sender gave it, if any.
    pub filename: Option<String>,
    /// Size in bytes as the server declared it. `0` for a container.
    pub size: u64,
    /// Whether the bytes are already in the blob store.
    pub downloaded: bool,
    /// Whether this is the last child of its parent, for `└` rather than `├`.
    pub last: bool,
    /// The attachment row this came from; `None` for the synthetic root.
    pub attachment: Option<AttachmentId>,
}

impl Node {
    /// What the row calls this part: its filename, or its type when the
    /// sender did not name it.
    pub fn label(&self) -> &str {
        match self.filename.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name,
            _ => &self.mime,
        }
    }

    /// Whether this part holds bytes worth saving, as opposed to being a
    /// container for other parts.
    pub fn is_leaf(&self) -> bool {
        !self.mime.starts_with("multipart/") && self.attachment.is_some()
    }
}

/// Reads `parts` as a tree, flattened in walk order.
///
/// `root` is the message's own content type — `multipart/mixed` — which is a
/// property of the message rather than of any part, so it is passed in rather
/// than guessed. A message with no parts still gets its root node: a tree
/// with one entry is a true answer, where an empty panel would look broken.
///
/// Parts are ordered by their id read as a path of numbers, so `2.10` sorts
/// after `2.9` rather than before it the way a string compare would.
pub fn tree(root: &str, parts: &[Attachment]) -> Vec<Node> {
    let mut ordered: Vec<(Vec<u32>, &Attachment)> = parts
        .iter()
        .map(|part| (path_of(part), part))
        .filter(|(path, _)| !path.is_empty())
        .collect();
    ordered.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut nodes = Vec::with_capacity(ordered.len() + 1);
    nodes.push(Node {
        part_id: String::new(),
        depth: 0,
        mime: root.to_owned(),
        filename: None,
        size: parts.iter().map(|part| part.size).sum(),
        downloaded: false,
        last: ordered.is_empty(),
        attachment: None,
    });

    for (index, (path, part)) in ordered.iter().enumerate() {
        // The last child of *this* parent, not the last row overall: a
        // deeper branch that ends before its parent's next sibling still
        // needs its own `└`.
        let last = ordered.get(index + 1).is_none_or(|(next, _)| {
            next.len() < path.len() || next[..path.len() - 1] != path[..path.len() - 1]
        });
        nodes.push(Node {
            part_id: part.part_id.clone().unwrap_or_default(),
            depth: path.len(),
            mime: part.mime_type.clone(),
            filename: part.filename.clone(),
            size: part.size,
            downloaded: part.blob_id.is_some(),
            last,
            attachment: Some(part.id),
        });
    }
    nodes
}

/// A part id read as a path: `"2.1"` becomes `[2, 1]`.
///
/// A part with no id at all, or one that is not a path of numbers, sorts as
/// nothing and is dropped — the tree draws what the server described, and a
/// row nothing can be fetched for is a row that leads nowhere.
fn path_of(part: &Attachment) -> Vec<u32> {
    let Some(id) = part.part_id.as_deref() else {
        return Vec::new();
    };
    let mut path = Vec::new();
    for segment in id.split('.') {
        match segment.parse::<u32>() {
            Ok(number) => path.push(number),
            Err(_) => return Vec::new(),
        }
    }
    path
}

/// What the detail pane says about one part: `text/html · 6 KB`.
pub fn detail(node: &Node) -> String {
    if node.depth == 0 || node.size == 0 {
        return node.mime.clone();
    }
    format!("{} · {}", node.mime, human_size(node.size))
}

/// How a part reads to a screen reader: what it is, how big, and whether
/// anything has actually been downloaded.
pub fn spoken(node: &Node) -> String {
    if node.depth == 0 {
        return format!("{}, the whole message", node.mime);
    }
    let name = match node.filename.as_deref() {
        Some(name) if !name.trim().is_empty() => format!("{name}, {}", node.mime),
        _ => node.mime.clone(),
    };
    if !node.is_leaf() {
        return format!("{name}, a container");
    }
    let fetched = if node.downloaded {
        "downloaded"
    } else {
        "not downloaded"
    };
    format!("{name}, {}, {fetched}", human_size(node.size))
}

#[cfg(test)]
mod blob_source_tests {
    use super::BlobSource;

    #[test]
    fn a_closure_resolves_and_ignores_the_scope_by_default() {
        let source =
            |id: &str| (id == "logo@example.com").then(|| (vec![1, 2], "image/png".to_owned()));
        assert_eq!(
            source.resolve("logo@example.com"),
            Some((vec![1, 2], "image/png".to_owned()))
        );
        assert_eq!(source.resolve("other@example.com"), None);
        assert_eq!(
            source.resolve_in(Some("7"), "logo@example.com"),
            source.resolve("logo@example.com")
        );
        assert_eq!(source.resolve_in(None, "other@example.com"), None);
    }
}
