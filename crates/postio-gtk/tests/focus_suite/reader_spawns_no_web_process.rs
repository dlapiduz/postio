//! The reader draws in this process, so moving through ten conversations must start no web process
//! (counted as descendants named `WebKitWebProces`: Linux cuts `comm` to fifteen characters). The
//! composer is still WebKit and warms on an idle turn, so the count is taken against a baseline
//! after that.

use std::time::Duration;

use gtk::glib;

use crate::support::{self, Fixture};

fn web_processes() -> Vec<i32> {
    let me = std::process::id() as i32;
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .is_ok_and(|comm| comm.trim() == "WebKitWebProces")
                && descends_from(*pid, me)
        })
        .collect()
}

/// WebKit puts its processes under a `bwrap` sandbox, so they are grandchildren.
fn descends_from(mut pid: i32, ancestor: i32) -> bool {
    for _ in 0..16 {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let Some(parent) = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(1))
            .and_then(|parent| parent.parse::<i32>().ok())
        else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        if parent <= 1 {
            return false;
        }
        pid = parent;
    }
    false
}

/// POSTIO-FIXED-DEADLINE: gives a process that should not start, and a render that should not happen,
/// every chance to.
async fn spend(duration: Duration) {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub fn ten_conversations_start_no_web_process_and_hold_what_one_holds() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        for index in 0..11_i64 {
            let body = format!(
                "Message {index}: the numbers for the harbour schedule, in a \
                 few sentences so the body has lines to lay out and draw."
            );
            let (message, _) = fixture
                .file(
                    ("Ada Moreno", "ada@example.com"),
                    &format!("Conversation {index}"),
                    "A line of text.",
                    10 + index,
                )
                .await;
            fixture.write_body(message, &body).await;
        }
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() >= 11).await,
            "the fixture needs eleven conversations to move through"
        );

        let generation = |window: &postio_gtk::window::FocusWindow| {
            window
                .reading()
                .and_then(|reading| reading.reader().view().document())
                .map(|document| document.generation)
        };

        support::keys(&window, &["j"]);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || generation(&window).is_some()).await,
            "the first conversation never drew"
        );
        spend(Duration::from_millis(300)).await;
        window.warm_composer();
        spend(Duration::from_secs(2)).await;
        let baseline = web_processes();
        assert!(
            !baseline.is_empty(),
            "the composer's warm-up started no WebKit process this counter can \
             see, so a zero below would prove nothing"
        );
        let reading = window.reading().expect("the message is open");
        // The tiles are rasterised when the view draws, which a loaded machine reaches after the
        // snapshot: a baseline read earlier is 0 and every conversation looks like growth.
        assert!(
            crate::settle_until(async || reading.reader().view().tile_bytes() > 0).await,
            "the first conversation never rasterised a tile"
        );
        let tiles_after_one = reading.reader().view().tile_bytes();
        let live_after_one = postio_render::live_documents();

        for step in 2..=10 {
            let before = generation(&window);
            let (renders, counts) = postio_ui::test_support::snapshot_counts();
            support::deliver(&window, "j");
            assert!(
                crate::settle_until(
                    async || generation(&window).is_some() && generation(&window) != before
                )
                .await,
                "conversation {step} never drew"
            );
            spend(Duration::from_millis(300)).await;
            let (renders_after, counts_after) = postio_ui::test_support::snapshot_counts();
            // Focus clears the dialog to a placeholder on a move (`Absent::Partial`) and draws the
            // message when its body lands:
            // one snapshot, plus the placeholder's if a frame was drawn between. Never more.
            let renders_taken = renders_after - renders;
            assert!(
                (1..=2).contains(&renders_taken),
                "conversation {step} took {renders_taken} renders"
            );
            assert!(
                counts_after.style_passes - counts.style_passes <= 4,
                "conversation {step} took {} style passes",
                counts_after.style_passes - counts.style_passes
            );
        }

        let now = web_processes();
        assert!(
            now.iter().all(|pid| baseline.contains(pid)),
            "moving through conversations started WebKit processes: {now:?} \
             against {baseline:?} before"
        );
        assert_eq!(
            postio_render::live_documents(),
            live_after_one,
            "ten conversations left more snapshots alive than one"
        );
        let reading = window.reading().expect("the message is open");
        let tiles = reading.reader().view().tile_bytes();
        assert!(
            tiles <= tiles_after_one.max(1) * 2,
            "the tile cache grew with the conversations viewed: {tiles} bytes \
             against {tiles_after_one} after the first"
        );
    });
}
