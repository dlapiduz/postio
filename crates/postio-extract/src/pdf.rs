//! PDF, a page at a time, through `pdf-extract` (research R5).
//!
//! `pdf-extract` is the only pure-Rust text extractor worth the name, and
//! it was not written for hostile input: it `unwrap`s freely, and it
//! recurses without a guard in two places a file controls — a form
//! XObject that draws itself, and a page tree whose `Parent` chain loops
//! back while it looks for an inherited key. A panic can be caught; a
//! stack overflow cannot, and it takes the whole application with it. So
//! three things stand between it and the caller:
//!
//! 1. **A scan before each page** ([`page_is_safe`]): the `Parent` chain
//!    is walked with a bound and a memory, and every form XObject the
//!    page reaches through `Do` is followed with its chain of ancestors,
//!    so a cycle, a chain deeper than [`MAX_FORM_DEPTH`] or a fan-out past
//!    [`MAX_FORMS`] marks the page unreadable before `pdf-extract` sees
//!    it. The scan is iterative; it cannot overflow what it guards.
//! 2. **`catch_unwind` around every call** into the library, per page, so
//!    one page that panics costs that page and the file is `Failed` with
//!    the pages read before it kept.
//! 3. **A thread of its own** with a deep stack, which the caller waits on
//!    only until the deadline: a page that is slow to lay out cannot hold
//!    the indexer past `Limits::max_time`. Past it the thread is told to
//!    stop and finishes the page it is on; it cannot be killed, which is
//!    why (1) and the input limit bound what one page can cost.
//!
//! If a file is ever found that still overflows the stack, the remaining
//! answer is research R5's: extraction moves to a child process.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use pdf_extract::content::Content;
use pdf_extract::{Dictionary, Document, Object, ObjectId};

use crate::limits::{Budget, Stop};
use crate::{Location, Skip};

/// The deepest chain of form XObjects drawn inside one another that a page
/// may have. Real documents nest a handful; a cycle is infinitely deep.
const MAX_FORM_DEPTH: usize = 16;

/// The most form XObjects one page's scan will follow, counting each time
/// one is drawn: a form drawn twice by a form drawn twice, sixteen deep, is
/// 65,536 draws with no cycle anywhere, and costs the extractor the same.
const MAX_FORMS: usize = 4_096;

/// The longest `Parent` chain a page may have.
const MAX_PARENTS: usize = 64;

/// The extraction thread's stack. The library's ordinary recursion (fonts,
/// content streams, the forms the scan allowed) is shallow; this is room
/// for an honest document that is deeper than usual.
const STACK: usize = 16 * 1024 * 1024;

/// What the extraction thread reports.
enum Message {
    Page(u32, String),
    Encrypted,
    Unreadable,
    Done { failed: bool },
}

pub(crate) fn extract(bytes: &[u8], budget: &mut Budget<'_>) -> Result<(), Stop> {
    let (sender, receiver) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let owned = bytes.to_vec();
    let halt = Arc::clone(&stop);
    let spawned = std::thread::Builder::new()
        .name("postio-extract-pdf".to_owned())
        .stack_size(STACK)
        .spawn(move || read(&owned, &sender, &halt));
    if spawned.is_err() {
        return Err(Stop::Failed);
    }

    let outcome = loop {
        let now = Instant::now();
        let Some(left) = budget.deadline.checked_duration_since(now) else {
            break Err(Stop::Limit(crate::Limit::Time));
        };
        match receiver.recv_timeout(left) {
            Ok(Message::Page(number, text)) => {
                if let Err(limit) = budget.push(Location::Page(number), &text) {
                    break Err(Stop::Limit(limit));
                }
            }
            Ok(Message::Encrypted) => break Err(Stop::Skip(Skip::Encrypted)),
            Ok(Message::Unreadable) => break Err(Stop::Failed),
            Ok(Message::Done { failed }) => break if failed { Err(Stop::Failed) } else { Ok(()) },
            Err(mpsc::RecvTimeoutError::Timeout) => break Err(Stop::Limit(crate::Limit::Time)),
            // The thread ended without saying how: a panic outside the
            // guarded calls. What it sent before is kept.
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err(Stop::Failed),
        }
    };
    stop.store(true, Ordering::Relaxed);
    outcome
}

/// The thread's whole life: load, decrypt with the empty password if it
/// must, then each page in order until told to stop.
fn read(bytes: &[u8], sender: &mpsc::Sender<Message>, stop: &AtomicBool) {
    let send = |message| {
        // A caller that has gone (its deadline passed) is not an error.
        let _ = sender.send(message);
    };
    let loaded = catch_unwind(AssertUnwindSafe(|| Document::load_mem(bytes)));
    let mut document = match loaded {
        Ok(Ok(document)) => document,
        Ok(Err(pdf_extract::Error::Decryption(_))) => return send(Message::Encrypted),
        Ok(Err(_)) | Err(_) => return send(Message::Unreadable),
    };
    if document.is_encrypted() {
        // An owner password alone restricts printing and copying, not
        // reading, and opens with the empty user password; anything else
        // is a password nobody gave this extractor.
        let opened = catch_unwind(AssertUnwindSafe(|| document.decrypt("")));
        if !matches!(opened, Ok(Ok(()))) {
            return send(Message::Encrypted);
        }
    }

    let pages = match catch_unwind(AssertUnwindSafe(|| document.get_pages())) {
        Ok(pages) => pages,
        Err(_) => return send(Message::Unreadable),
    };
    let mut failed = false;
    for (number, id) in pages {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if !page_is_safe(&document, id) {
            failed = true;
            continue;
        }
        let text = catch_unwind(AssertUnwindSafe(|| {
            let mut text = String::new();
            let mut output = pdf_extract::PlainTextOutput::new(&mut text);
            pdf_extract::output_doc_page(&document, &mut output, number).map(|()| text)
        }));
        match text {
            Ok(Ok(text)) => send(Message::Page(number, text)),
            Ok(Err(_)) | Err(_) => failed = true,
        }
    }
    send(Message::Done { failed });
}

/// Whether `pdf-extract` can be handed this page without recursing past
/// what any stack holds. See the module's first point.
fn page_is_safe(document: &Document, page: ObjectId) -> bool {
    let Ok(dictionary) = document.get_dictionary(page) else {
        return false;
    };
    let Some(chain) = parents(document, page, dictionary) else {
        return false;
    };
    let resources = chain
        .iter()
        .find_map(|dictionary| dictionary.get(b"Resources").ok())
        .and_then(|object| dictionary_of(document, object));
    let Ok(content) = document.get_page_content(page) else {
        // Nothing to draw is nothing to recurse into.
        return true;
    };
    forms_are_finite(document, content, resources)
}

/// The page and each ancestor, nearest first; `None` when the chain loops
/// or runs past [`MAX_PARENTS`].
fn parents<'d>(
    document: &'d Document,
    page: ObjectId,
    dictionary: &'d Dictionary,
) -> Option<Vec<&'d Dictionary>> {
    let mut seen = vec![page];
    let mut chain = vec![dictionary];
    let mut current = dictionary;
    while let Ok(parent) = current.get(b"Parent").and_then(Object::as_reference) {
        if seen.contains(&parent) || seen.len() > MAX_PARENTS {
            return None;
        }
        seen.push(parent);
        match document.get_dictionary(parent) {
            Ok(next) => {
                chain.push(next);
                current = next;
            }
            Err(_) => break,
        }
    }
    Some(chain)
}

/// Follow every `Do` from `content`, depth first with an explicit stack,
/// carrying each form's chain of ancestors.
fn forms_are_finite(document: &Document, content: Vec<u8>, resources: Option<&Dictionary>) -> bool {
    let mut work: Vec<(Vec<u8>, Option<&Dictionary>, Vec<ObjectId>)> =
        vec![(content, resources, Vec::new())];
    let mut drawn = 0usize;
    while let Some((bytes, resources, chain)) = work.pop() {
        let Ok(content) = Content::decode(&bytes) else {
            continue;
        };
        for operation in content.operations {
            if operation.operator != "Do" {
                continue;
            }
            let Some(name) = operation.operands.first().and_then(|o| o.as_name().ok()) else {
                continue;
            };
            let Some(Object::Reference(id)) = resources
                .and_then(|r| r.get(b"XObject").ok())
                .and_then(|o| dictionary_of(document, o))
                .and_then(|x| x.get(name).ok())
            else {
                // A form stored inline cannot name itself; only a
                // reference can close a loop.
                continue;
            };
            if chain.contains(id) || chain.len() >= MAX_FORM_DEPTH {
                return false;
            }
            drawn += 1;
            if drawn > MAX_FORMS {
                return false;
            }
            let Ok(Object::Stream(form)) = document.get_object(*id) else {
                continue;
            };
            let inner = form
                .dict
                .get(b"Resources")
                .ok()
                .and_then(|o| dictionary_of(document, o))
                .or(resources);
            let bytes = form
                .decompressed_content()
                .unwrap_or_else(|_| form.content.clone());
            let mut next = chain.clone();
            next.push(*id);
            work.push((bytes, inner, next));
        }
    }
    true
}

/// A dictionary, directly or through a few references.
fn dictionary_of<'d>(document: &'d Document, object: &'d Object) -> Option<&'d Dictionary> {
    let mut object = object;
    for _ in 0..8 {
        match object {
            Object::Reference(id) => object = document.get_object(*id).ok()?,
            Object::Dictionary(dictionary) => return Some(dictionary),
            _ => return None,
        }
    }
    None
}
