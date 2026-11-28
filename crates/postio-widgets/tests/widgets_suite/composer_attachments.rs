use crate::support_compose::Window;
use crate::support_compose::composer;
use crate::support_compose::settle;
use crate::support_compose::{app, fonts, style};
use gtk::gdk;
use postio_model::ids::MessageId;
use postio_model::{AccountId, Attachment, Draft};

pub fn attaching_shows_the_row_and_removing_cleans_it_up() {
    let state_dir_guard = tempfile::tempdir().expect("a state directory");
    let state_dir = state_dir_guard.path();

    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = composer::install(&window);
    composer.open(Draft::new(AccountId::UNASSIGNED));
    settle();

    let file_path = state_dir.join("report.pdf");
    std::fs::write(&file_path, b"not a real pdf").unwrap();

    composer.test_attach_path(&file_path);
    settle();
    assert_eq!(composer.test_attachment_count(), 0);
    assert!(!composer.test_attachments_visible());
    assert!(
        composer.status().contains("not attached"),
        "status should name the missing handler, not stay silent: {}",
        composer.status()
    );

    composer.connect_attach(|path, then| {
        let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 2_048);
        attachment.filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        then(Some(attachment));
    });
    composer.test_attach_path(&file_path);
    settle();
    assert_eq!(composer.test_attachment_count(), 1);
    assert!(composer.test_attachments_visible());
    let draft = composer.draft();
    assert_eq!(draft.attachments.len(), 1);
    assert_eq!(draft.attachments[0].filename.as_deref(), Some("report.pdf"));
    assert_eq!(draft.attachments[0].size, 2_048);

    // Two targets: one on the composer so a file dropped anywhere attaches, one on the body, which
    // inlines an image and attaches anything else.
    let targets = composer.test_drop_targets();
    assert!(
        targets.iter().any(|(place, _)| *place == "composer"),
        "nothing on the composer accepts a drop, so dropping a file on it \
         does nothing at all: {targets:?}"
    );
    assert!(
        targets.iter().any(|(place, _)| *place == "body"),
        "the body has no drop target of its own, so an image dropped into \
         the message would attach beside it instead of landing in it: \
         {targets:?}"
    );
    // A native text view also has its own target for dragged text; what matters is that each
    // place has one that takes files.
    for place in ["composer", "body"] {
        assert!(
            targets
                .iter()
                .any(|(at, formats)| *at == place && formats.contains("FileList")),
            "no {place} drop target accepts files: {targets:?}"
        );
    }

    // Stands in for a handler doing a slow blob-store write and calling back later; nothing blocks
    // on `then`.
    composer.connect_attach(|_, then| {
        glib::idle_add_local_once(move || {
            then(Some(Attachment::new(
                MessageId::UNASSIGNED,
                "application/zip",
                64 * 1024 * 1024,
            )));
        });
    });
    composer.test_attach_path(&file_path);
    settle();
    assert_eq!(
        composer.test_attachment_count(),
        2,
        "the row appears once the deferred handler calls back"
    );

    composer.test_remove_attachment(0);
    settle();
    assert_eq!(
        composer.test_attachment_count(),
        1,
        "removing index 0 should drop the first attachment, not the second"
    );
    assert_eq!(composer.draft().attachments[0].size, 64 * 1024 * 1024);

    composer.test_remove_attachment(0);
    settle();
    assert_eq!(composer.test_attachment_count(), 0);
    assert!(
        !composer.test_attachments_visible(),
        "the row hides again once nothing is left to show"
    );
}
