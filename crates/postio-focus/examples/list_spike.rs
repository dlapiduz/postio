//! Spike S3 (specs/007-postio-focus T009, research R3). Throwaway: deleted
//! in the commit after the one that records its numbers.
//!
//! 100,000 synthetic conversations of two fixed heights, 40 and 72 px, plus
//! 50 spliced rows, in a `gtk::ListView`. The script scrolls it and jumps
//! around it, one step per frame, and counts per frame how many row widgets
//! the factory built (`setup`), how many it filled (`bind`) and how many
//! items the model was asked for. Counts, not timings: they are the same on
//! any machine.
//!
//! Run it on the headless compositor:
//! `scripts/test-headless.sh target/debug/examples/list_spike`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

const CONVERSATIONS: u32 = 100_000;
const SPLICED: u32 = 50;
const TOTAL: u32 = CONVERSATIONS + SPLICED;
const ONE_LINE: i32 = 40;
const TWO_LINES: i32 = 72;

/// Where the spliced rows sit: spread evenly through the list.
fn splice_positions() -> Vec<u32> {
    let step = TOTAL / (SPLICED + 1);
    (1..=SPLICED).map(|k| k * step).collect()
}

/// A row's height, by its kind: a spliced digest is one line and a spliced
/// reminder two; a conversation is two lines when it has a marker, which
/// one in five has here.
fn height_of(position: u32, spliced: &[u32]) -> i32 {
    if let Ok(index) = spliced.binary_search(&position) {
        return if index % 2 == 0 { ONE_LINE } else { TWO_LINES };
    }
    if position % 5 == 2 {
        TWO_LINES
    } else {
        ONE_LINE
    }
}

thread_local! {
    /// The positions bound to a row widget now, to check a jump landed.
    static BOUND: RefCell<std::collections::HashSet<u32>> = RefCell::new(std::collections::HashSet::new());
    static SETUPS: Cell<u64> = const { Cell::new(0) };
    static BINDS: Cell<u64> = const { Cell::new(0) };
    static ITEMS: Cell<u64> = const { Cell::new(0) };
}

fn bump(counter: &'static std::thread::LocalKey<Cell<u64>>) {
    counter.with(|count| count.set(count.get() + 1));
}

fn read(counter: &'static std::thread::LocalKey<Cell<u64>>) -> u64 {
    counter.with(Cell::get)
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SpikeItem {
        pub position: Cell<u32>,
        pub height: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpikeItem {
        const NAME: &'static str = "PostioFocusSpikeItem";
        type Type = super::SpikeItem;
    }

    impl ObjectImpl for SpikeItem {}

    #[derive(Default)]
    pub struct SpikeModel {
        pub spliced: RefCell<Vec<u32>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpikeModel {
        const NAME: &'static str = "PostioFocusSpikeModel";
        type Type = super::SpikeModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for SpikeModel {}

    impl ListModelImpl for SpikeModel {
        fn item_type(&self) -> glib::Type {
            super::SpikeItem::static_type()
        }

        fn n_items(&self) -> u32 {
            TOTAL
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            if position >= TOTAL {
                return None;
            }
            bump(&ITEMS);
            let item: super::SpikeItem = glib::Object::new();
            item.imp().position.set(position);
            item.imp()
                .height
                .set(height_of(position, &self.spliced.borrow()));
            Some(item.upcast())
        }
    }
}

glib::wrapper! {
    pub struct SpikeItem(ObjectSubclass<imp::SpikeItem>);
}

glib::wrapper! {
    pub struct SpikeModel(ObjectSubclass<imp::SpikeModel>) @implements gio::ListModel;
}

/// One step of the script, applied at the start of a frame.
#[derive(Clone, Copy, Debug)]
enum Step {
    /// Start counting under a new name.
    Phase(&'static str),
    /// Move the view by this many pixels.
    ScrollBy(f64),
    /// Bring this position into view.
    JumpTo(u32),
    /// Do nothing this frame.
    Wait,
}

/// What one phase cost, frame by frame.
#[derive(Default, Debug)]
struct Tally {
    name: &'static str,
    frames: u64,
    setups: u64,
    binds: u64,
    items: u64,
    max_setups: u64,
    max_binds: u64,
    max_items: u64,
}

fn script() -> Vec<Step> {
    let mut steps = vec![
        Step::Phase("first frames"),
        Step::Wait,
        Step::Wait,
        Step::Wait,
    ];
    steps.push(Step::Phase("scroll 40 px a frame"));
    steps.extend(std::iter::repeat_n(Step::ScrollBy(40.0), 200));
    steps.push(Step::Phase("scroll 800 px a frame"));
    steps.extend(std::iter::repeat_n(Step::ScrollBy(800.0), 200));
    steps.push(Step::Phase("jump anywhere"));
    // A fixed linear congruential sequence: the same jumps every run.
    let mut seed: u64 = 0x2545_f491;
    for _ in 0..60 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        steps.push(Step::JumpTo((seed >> 33) as u32 % TOTAL));
        steps.push(Step::Wait);
    }
    steps.push(Step::Phase("jump to each spliced row"));
    for position in splice_positions() {
        steps.push(Step::JumpTo(position));
        steps.push(Step::Wait);
    }
    steps.push(Step::Phase("bottom, then top"));
    steps.extend([
        Step::JumpTo(TOTAL - 1),
        Step::Wait,
        Step::Wait,
        Step::JumpTo(0),
        Step::Wait,
        Step::Wait,
    ]);
    steps.push(Step::Phase("end"));
    steps
}

fn main() -> glib::ExitCode {
    if gtk::init().is_err() {
        eprintln!("list_spike: no display");
        return glib::ExitCode::FAILURE;
    }
    let spliced = splice_positions();
    let model: SpikeModel = glib::Object::new();
    model.imp().spliced.replace(spliced.clone());

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        bump(&SETUPS);
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let row = gtk::Label::new(None);
        row.set_xalign(0.0);
        item.set_child(Some(&row));
    });
    factory.connect_unbind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        if let Some(data) = item.item().and_downcast::<SpikeItem>() {
            BOUND.with(|bound| bound.borrow_mut().remove(&data.imp().position.get()));
        }
    });
    factory.connect_bind(|_, item| {
        bump(&BINDS);
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let data = item
            .item()
            .and_downcast::<SpikeItem>()
            .expect("a spike item");
        let row = item.child().and_downcast::<gtk::Label>().expect("a label");
        row.set_size_request(-1, data.imp().height.get());
        BOUND.with(|bound| bound.borrow_mut().insert(data.imp().position.get()));
        row.set_label(&format!("row {}", data.imp().position.get()));
    });

    let list = gtk::ListView::new(Some(gtk::NoSelection::new(Some(model))), Some(factory));
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .build();
    let window = gtk::Window::builder()
        .default_width(1440)
        .default_height(900)
        .child(&scrolled)
        .build();

    let tallies: Rc<RefCell<Vec<Tally>>> = Rc::default();
    // (jumps made, jumps whose target was bound by the frame after)
    let landed = Rc::new(Cell::new((0u32, 0u32)));
    let target: Rc<Cell<Option<u32>>> = Rc::default();
    let last = Rc::new(Cell::new((0u64, 0u64, 0u64)));
    let steps = Rc::new(RefCell::new(std::collections::VecDeque::from(script())));
    let main_loop = glib::MainLoop::new(None, false);

    // Before the first frame, the step for each frame; after it, the count.
    list.add_tick_callback({
        let steps = Rc::clone(&steps);
        let tallies = Rc::clone(&tallies);
        let adjustment = scrolled.vadjustment();
        let main_loop = main_loop.clone();
        let landed = Rc::clone(&landed);
        let target = Rc::clone(&target);
        move |list, _| {
            // The jump made two frames ago has had its frame: did it bind?
            if let Some(position) = target.take() {
                let bound = BOUND.with(|bound| bound.borrow().contains(&position));
                let (jumps, hits) = landed.get();
                landed.set((jumps + 1, hits + u32::from(bound)));
            }
            let Some(step) = steps.borrow_mut().pop_front() else {
                main_loop.quit();
                return glib::ControlFlow::Break;
            };
            match step {
                Step::Phase(name) => tallies.borrow_mut().push(Tally {
                    name,
                    ..Tally::default()
                }),
                Step::ScrollBy(pixels) => adjustment.set_value(adjustment.value() + pixels),
                Step::JumpTo(position) => {
                    list.scroll_to(position, gtk::ListScrollFlags::NONE, None);
                    target.set(Some(position));
                }
                Step::Wait => {}
            }
            glib::ControlFlow::Continue
        }
    });

    window.connect_realize({
        let tallies = Rc::clone(&tallies);
        let last = Rc::clone(&last);
        move |window| {
            let clock = window.frame_clock().expect("a realized window has a clock");
            let tallies = Rc::clone(&tallies);
            let last = Rc::clone(&last);
            clock.connect_after_paint(move |_| {
                let now = (read(&SETUPS), read(&BINDS), read(&ITEMS));
                let before = last.replace(now);
                let (setups, binds, items) = (now.0 - before.0, now.1 - before.1, now.2 - before.2);
                if let Some(tally) = tallies.borrow_mut().last_mut() {
                    tally.frames += 1;
                    tally.setups += setups;
                    tally.binds += binds;
                    tally.items += items;
                    tally.max_setups = tally.max_setups.max(setups);
                    tally.max_binds = tally.max_binds.max(binds);
                    tally.max_items = tally.max_items.max(items);
                }
            });
        }
    });

    window.present();
    let estimated_before = Rc::new(Cell::new(0.0));
    glib::timeout_add_local_once(std::time::Duration::from_millis(50), {
        let adjustment = scrolled.vadjustment();
        let estimated_before = Rc::clone(&estimated_before);
        move || estimated_before.set(adjustment.upper())
    });
    main_loop.run();

    let true_height: i64 = (0..TOTAL)
        .map(|position| i64::from(height_of(position, &spliced)))
        .sum();
    let adjustment = scrolled.vadjustment();
    println!(
        "viewport {} px; {} rows ({} conversations, {} spliced)",
        adjustment.page_size(),
        TOTAL,
        CONVERSATIONS,
        SPLICED
    );
    println!(
        "list height: true {true_height} px; GTK's estimate at start {:.0}, at end {:.0}",
        estimated_before.get(),
        adjustment.upper()
    );
    println!(
        "phase | frames | setup (max/frame) | bind (max/frame, mean) | items asked (max/frame)"
    );
    for tally in tallies.borrow().iter().filter(|tally| tally.frames > 0) {
        println!(
            "{} | {} | {} ({}) | {} ({}, {:.1}) | {} ({})",
            tally.name,
            tally.frames,
            tally.setups,
            tally.max_setups,
            tally.binds,
            tally.max_binds,
            tally.binds as f64 / tally.frames as f64,
            tally.items,
            tally.max_items,
        );
    }
    let (jumps, hits) = landed.get();
    println!("jumps whose target had a bound row widget the frame after: {hits} of {jumps}");
    println!(
        "row widgets built in the whole run: {}; binds {}; items asked {}",
        read(&SETUPS),
        read(&BINDS),
        read(&ITEMS)
    );
    glib::ExitCode::SUCCESS
}
