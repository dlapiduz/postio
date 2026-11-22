use gtk::prelude::*;

use crate::support;

#[allow(dead_code)]
#[path = "../../examples/shot.rs"]
mod example;

fn run(args: &[&str]) -> Result<String, String> {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    example::run(&args)
}

/// Smaller than the references' 1440x900: the suite's compositor has a 1280x800 monitor, and mutter
/// maximizes a window that nearly fills one.
pub fn screen_01_is_written_as_a_png() {
    let directory = tempfile::tempdir().expect("a scratch directory");
    let path = directory.path().join("01.png");
    let path_text = path.to_str().expect("a UTF-8 path");
    let said = run(&[path_text, "01", "960x600"]);
    assert!(said.is_ok(), "screen 01 was not rendered: {said:?}");
    let texture = gtk::gdk::Texture::from_filename(&path).expect("a PNG GDK can read back");
    assert_eq!((texture.width(), texture.height()), (960, 600));
}

pub fn an_unknown_screen_writes_nothing_and_says_so() {
    let directory = tempfile::tempdir().expect("a scratch directory");
    let path = directory.path().join("99.png");
    let path_text = path.to_str().expect("a UTF-8 path");
    let error = run(&[path_text, "99"]).expect_err("screen 99 does not exist");
    assert!(
        error.contains("NO IMAGE WAS WRITTEN"),
        "the failure does not say no image was written: {error}"
    );
    assert!(!path.exists(), "an unknown screen left a file behind");
}

pub fn the_demo_inbox_opens_with_its_first_heading_on_screen() {
    crate::gtk_case(async {
        let (database, _) = postio_gtk::demo::demo().await;
        let blobs = tempfile::tempdir().expect("a blob directory");
        let store = postio_storage::BlobStore::open(
            blobs.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let host = postio_host::Host::start(database, store, |wiring| wiring).expect("a host");
        let window = postio_gtk::window::FocusWindow::new(None);
        window.set_default_size(1000, 640);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        support::keep(blobs);
        let pane = window.pane().expect("the inbox");
        assert!(
            crate::settle_until(async || {
                let rows = pane.rows_on_screen();
                !rows.is_empty() && rows.iter().all(|row| !row.drawn().texts.is_empty())
            })
            .await,
            "the demo inbox never reached the screen"
        );
        crate::settle();
        assert_eq!(
            pane.widget().vadjustment().value(),
            0.0,
            "the demo inbox opened scrolled past its first heading"
        );
    });
}
