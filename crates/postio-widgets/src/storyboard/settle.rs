//! Frames are sampled on the window's frame clock and a step is settled when they stop changing.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;

#[derive(Debug, Clone)]
pub struct Settings {
    pub identical: usize,
    /// Capture has a cost, so a slow machine can look less often.
    pub stride: u32,
    pub watch: Duration,
    pub max: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            identical: 6,
            stride: 1,
            watch: Duration::from_millis(300),
            max: Duration::from_millis(3000),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Settled { ms: u64 },
    Jumped { frames: usize },
    Blanked { frames: usize },
    Unsettled { ms: u64 },
}

#[derive(Debug)]
pub struct Settled {
    /// What watching showed.
    pub verdict: Verdict,
    /// The step's frame: the settled one, or the last seen if it never was.
    pub texture: gdk::Texture,
    /// Hex blake3 of `texture`'s pixels.
    pub hash: String,
    /// Hex blake3 of every sample, in order.
    pub hashes: Vec<String>,
    pub extra: Vec<gdk::Texture>,
}

struct Frame {
    texture: gdk::Texture,
    hash: String,
    blank: bool,
}

/// `blank` is set when there are no pixels or every pixel is the same.
fn frame(texture: gdk::Texture) -> Frame {
    let mut downloader = gdk::TextureDownloader::new(&texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8Premultiplied);
    let (bytes, _stride) = downloader.download_bytes();
    let (pixels, _) = bytes.as_chunks::<4>();
    let blank = pixels
        .first()
        .is_none_or(|first| pixels.iter().all(|p| p == first));
    Frame {
        hash: blake3::hash(&bytes).to_hex().to_string(),
        texture,
        blank,
    }
}

/// A visibly empty frame for a window with nothing to draw.
fn empty_texture(width: i32, height: i32) -> gdk::Texture {
    let (width, height) = (width.max(1), height.max(1));
    let pixels = gtk::glib::Bytes::from_owned(vec![0u8; (width * height * 4) as usize]);
    gdk::MemoryTexture::new(
        width,
        height,
        gdk::MemoryFormat::R8g8b8a8Premultiplied,
        &pixels,
        (width * 4) as usize,
    )
    .upcast()
}

struct Sampling {
    settings: Settings,
    started: Instant,
    ticks: u32,
    hashes: Vec<String>,
    last: Option<Frame>,
    run: usize,
    settled: Option<(Frame, Instant)>,
    extra: Vec<gdk::Texture>,
    done: Option<Verdict>,
}

impl Sampling {
    fn ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn tick(&mut self, window: &gtk::Window) {
        if self.done.is_some() {
            return;
        }
        self.ticks += 1;
        if !self.ticks.is_multiple_of(self.settings.stride.max(1)) {
            return;
        }
        // A window with no allocation yet is not a blank one: it has had no
        // layout. Skip the sample; `max` still bounds the wait.
        if let Ok(texture) = crate::capture::texture_now(window) {
            self.sample(frame(texture));
        }
        if self.done.is_none()
            && self.settled.is_none()
            && self.started.elapsed() >= self.settings.max
        {
            self.done = Some(Verdict::Unsettled { ms: self.ms() });
        }
    }

    fn sample(&mut self, frame: Frame) {
        self.hashes.push(frame.hash.clone());
        if let Some((settled, at)) = &self.settled {
            if frame.blank {
                self.extra.push(frame.texture.clone());
                self.done = Some(Verdict::Blanked {
                    frames: self.extra.len(),
                });
            } else if frame.hash != settled.hash {
                self.extra.push(settled.texture.clone());
                self.extra.push(frame.texture.clone());
                self.done = Some(Verdict::Jumped {
                    frames: self.extra.len(),
                });
            } else if at.elapsed() >= self.settings.watch {
                self.done = Some(Verdict::Settled {
                    ms: at.duration_since(self.started).as_millis() as u64,
                });
            }
            if self.done.is_some() {
                self.last = Some(frame);
            }
            return;
        }
        self.run = match &self.last {
            Some(last) if last.hash == frame.hash => self.run + 1,
            _ => 1,
        };
        if self.run >= self.settings.identical {
            if frame.blank {
                self.extra.push(frame.texture.clone());
                self.done = Some(Verdict::Blanked { frames: 1 });
                self.last = Some(frame);
            } else {
                self.settled = Some((
                    Frame {
                        texture: frame.texture.clone(),
                        hash: frame.hash.clone(),
                        blank: false,
                    },
                    Instant::now(),
                ));
                self.last = Some(frame);
            }
        } else {
            self.last = Some(frame);
        }
    }
}

/// Turns the main loop so the frame clock runs. After a jump the frame is the one the window
/// ended on, with the settled one in `extra`.
pub fn settle(window: &gtk::Window, settings: &Settings) -> Settled {
    let sampling = Rc::new(RefCell::new(Sampling {
        settings: settings.clone(),
        started: Instant::now(),
        ticks: 0,
        hashes: Vec::new(),
        last: None,
        run: 0,
        settled: None,
        extra: Vec::new(),
        done: None,
    }));
    let watcher = sampling.clone();
    let tick = window.add_tick_callback(move |window, _clock| {
        let mut sampling = watcher.borrow_mut();
        sampling.tick(window);
        if sampling.done.is_some() {
            gtk::glib::ControlFlow::Break
        } else {
            gtk::glib::ControlFlow::Continue
        }
    });

    // A blocking iteration lets the frame clock tick; the heartbeat guarantees it returns, so a
    // stopped clock still ends the step as unsettled.
    let context = gtk::glib::MainContext::default();
    let heartbeat = gtk::glib::timeout_add_local(Duration::from_millis(10), || {
        gtk::glib::ControlFlow::Continue
    });
    let give_up = settings.max + settings.watch + Duration::from_secs(1);
    while sampling.borrow().done.is_none() {
        if sampling.borrow().started.elapsed() >= give_up {
            let ms = sampling.borrow().ms();
            sampling.borrow_mut().done = Some(Verdict::Unsettled { ms });
            break;
        }
        context.iteration(true);
    }
    heartbeat.remove();
    tick.remove();

    let mut sampling = sampling.borrow_mut();
    let verdict = sampling.done.take().expect("sampling ended");
    let last = sampling.last.take();
    let (texture, hash) = match last {
        Some(frame) => (frame.texture, frame.hash),
        None => {
            let texture = empty_texture(window.width(), window.height());
            let hash = frame(texture.clone()).hash;
            (texture, hash)
        }
    };
    let mut extra = std::mem::take(&mut sampling.extra);
    let verdict = match verdict {
        // Nothing was ever drawable: a blank, and the blank picture is kept.
        Verdict::Unsettled { .. } if sampling.hashes.is_empty() => {
            extra.push(texture.clone());
            Verdict::Blanked { frames: 1 }
        }
        other => other,
    };
    Settled {
        verdict,
        texture,
        hash,
        hashes: std::mem::take(&mut sampling.hashes),
        extra,
    }
}
