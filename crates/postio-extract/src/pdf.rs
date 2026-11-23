//! PDF a page at a time through `pdf-extract`, which `unwrap`s freely and recurses
//! unguarded on loops a file controls; a stack overflow cannot be caught.
//! So each page is scanned first ([`page_is_safe`]), each call runs under
//! `catch_unwind`, and extraction runs on a thread the caller abandons at the deadline.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use pdf_extract::content::Content;
use pdf_extract::{Dictionary, Document, Object, ObjectId};

use crate::limits::{Budget, Stop};
use crate::{Location, Skip};

/// Real documents nest a handful; a cycle is infinitely deep.
const MAX_FORM_DEPTH: usize = 16;

/// Counts each draw: a form drawn twice by a form drawn twice, sixteen deep,
/// is 65,536 draws with no cycle.
const MAX_FORMS: usize = 4_096;

const MAX_PARENTS: usize = 64;

const STACK: usize = 16 * 1024 * 1024;

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
            // The thread ended without saying how: a panic outside the guarded calls.
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err(Stop::Failed),
        }
    };
    stop.store(true, Ordering::Relaxed);
    outcome
}

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
        // An owner password alone restricts printing, not reading, and opens
        // with the empty user password.
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
        return true;
    };
    forms_are_finite(document, content, resources)
}

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
