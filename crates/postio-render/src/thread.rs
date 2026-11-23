//! The render thread: each reader owns one and every Blitz document on it; only the
//! immutable snapshot leaves. A result for a superseded or abandoned generation is never
//! delivered, and a panic drops the document and delivers the plain-text fallback.
//! A layout cannot be interrupted: `abandon` detaches the thread and the next request goes to a
//! fresh one.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use crate::fonts::FontSet;
use crate::{FallbackReason, RenderRequest, RenderedDocument};

/// Callers take their deadline as a parameter and default to this.
pub const DEFAULT_RENDER_DEADLINE: Duration = Duration::from_millis(400);

/// The render thread's stack. Deep markup is flattened by the input caps;
/// this is the margin under them, because an overflow cannot be caught.
const STACK: usize = 64 * 1024 * 1024;

const THREAD_NAME: &str = "postio-render";

struct Job {
    request: RenderRequest,
    reply: Sender<RenderedDocument>,
}

struct Worker {
    jobs: Sender<Job>,
    running: Arc<AtomicU64>,
}

#[derive(Default)]
struct Generations {
    latest: AtomicU64,
    abandoned: AtomicU64,
}

impl Generations {
    fn wanted(&self, generation: u64) -> bool {
        generation >= self.latest.load(Ordering::Acquire)
            && generation > self.abandoned.load(Ordering::Acquire)
    }
}

pub struct Renderer {
    fonts: FontSet,
    generations: Arc<Generations>,
    worker: Mutex<Worker>,
}

impl Renderer {
    pub fn new(fonts: &FontSet) -> Renderer {
        install_panic_hook();
        let generations = Arc::new(Generations::default());
        Renderer {
            worker: Mutex::new(spawn(fonts.clone(), generations.clone())),
            fonts: fonts.clone(),
            generations,
        }
    }

    /// Non-blocking: the receiver disconnects if the request is superseded or abandoned first.
    pub fn request(&self, request: RenderRequest) -> Receiver<RenderedDocument> {
        self.generations
            .latest
            .fetch_max(request.generation, Ordering::AcqRel);
        let generation = request.generation;
        let (reply, result) = mpsc::channel();
        let job = Job { request, reply };
        let mut worker = self.worker.lock().expect("the worker is never poisoned");
        // A render still running for an older generation is one nobody will see: queueing behind it
        // would spend this request's deadline.
        let running = worker.running.load(Ordering::Acquire);
        if running != 0 && running < generation {
            *worker = spawn(self.fonts.clone(), self.generations.clone());
        }
        if let Err(mpsc::SendError(job)) = worker.jobs.send(job) {
            *worker = spawn(self.fonts.clone(), self.generations.clone());
            let _ = worker.jobs.send(job);
        }
        result
    }

    pub fn abandon(&self, generation: u64) {
        self.generations
            .abandoned
            .fetch_max(generation, Ordering::AcqRel);
        let mut worker = self.worker.lock().expect("the worker is never poisoned");
        if worker.running.load(Ordering::Acquire) == generation {
            *worker = spawn(self.fonts.clone(), self.generations.clone());
        }
    }

    pub fn fallback(&self, request: &RenderRequest, reason: FallbackReason) -> RenderedDocument {
        fallback(&self.fonts, request, reason)
    }
}

fn spawn(fonts: FontSet, generations: Arc<Generations>) -> Worker {
    let (jobs, queue) = mpsc::channel::<Job>();
    let running = Arc::new(AtomicU64::new(0));
    let busy = running.clone();
    std::thread::Builder::new()
        .name(THREAD_NAME.to_owned())
        .stack_size(STACK)
        .spawn(move || {
            for Job { request, reply } in queue {
                let generation = request.generation;
                if !generations.wanted(generation) {
                    continue;
                }
                if let Some(cap) = request.over_cap {
                    let document = fallback(&fonts, &request, FallbackReason::OverCap(cap));
                    if generations.wanted(generation) {
                        let _ = reply.send(document);
                    }
                    continue;
                }
                busy.store(generation, Ordering::Release);
                let stale = || !generations.wanted(generation);
                let rendered = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    crate::render::render_unless(&request, &fonts, &stale)
                }));
                busy.store(0, Ordering::Release);
                let document = match rendered {
                    Ok(Some(document)) => document,
                    Ok(None) => continue,
                    Err(_) => fallback(&fonts, &request, FallbackReason::Panicked),
                };
                if generations.wanted(generation) {
                    let _ = reply.send(document);
                }
            }
        })
        .expect("a render thread can be spawned");
    Worker { jobs, running }
}

fn fallback(fonts: &FontSet, request: &RenderRequest, reason: FallbackReason) -> RenderedDocument {
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let notice = format!(
        "<p class=\"{NOTICE_CLASS}\">{}</p>",
        escape(&reason.notice())
    );
    let document = match &request.fallback {
        Some(composed) if composed.notice => with_notice(&composed.document, &notice),
        Some(composed) => composed.document.clone(),
        None => {
            let (ground, ink) = if request.theme.dark {
                ("#1e1e1e", "#e8e8e8")
            } else {
                ("#ffffff", "#1a1a1a")
            };
            format!(
                "<!DOCTYPE html><html><body style=\"margin:0;padding:16px;\
                 background:{ground};color:{ink};font-family:sans-serif\">\
                 <p style=\"margin:0 0 12px;font-size:12.5px;opacity:0.75\">{}</p>\
                 <pre style=\"white-space:pre-wrap;font-family:sans-serif;margin:0\">{}</pre>\
                 </body></html>",
                escape(&reason.notice()),
                escape(&request.plain_text)
            )
        }
    };
    let request = RenderRequest {
        document,
        plain_text: String::new(),
        fallback: None,
        over_cap: None,
        // Never the message's table: it is what failed or is in use by the render given up on.
        resources: Arc::new(crate::Resources::new()),
        darkened: Vec::new(),
        toggled_folds: Vec::new(),
        reader_view: Vec::new(),
        ..request.clone()
    };
    let mut document = crate::render(&request, fonts);
    document.outcome = crate::Outcome::FellBack(reason);
    document
}

pub const NOTICE_CLASS: &str = "postio-fallback-notice";

/// The `<body>` after the head: the head's sheets and comments can spell the word too.
fn with_notice(document: &str, notice: &str) -> String {
    let head = document.find("</head>").unwrap_or(0);
    let at = document[head..]
        .find("<body")
        .map(|start| head + start)
        .and_then(|start| document[start..].find('>').map(|end| start + end + 1));
    match at {
        Some(at) => format!("{}{notice}{}", &document[..at], &document[at..]),
        None => format!("{notice}{document}"),
    }
}

/// Log a render thread's panic by where it happened, never by what it
/// said: a payload can quote the message (logs carry no content). Every
/// other thread's panic goes to the hook that was there before.
fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if std::thread::current().name() == Some(THREAD_NAME) {
                tracing::error!(
                    location = %info.location().map_or_else(String::new, ToString::to_string),
                    "render panicked; showing the plain-text fallback"
                );
            } else {
                previous(info);
            }
        }));
    });
}
