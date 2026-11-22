use postio_ui::observe::Tone;
use postio_widgets::widgets::toast::Toast;

pub fn a_toast_says_its_tone_and_whether_it_offers_undo() {
    let toast = Toast::new();
    assert_eq!(toast.tone(), None, "nothing is showing yet");
    assert!(!toast.offers_undo());

    toast.show_action_completed("Archived 1 message", true);
    assert_eq!(toast.tone(), Some(Tone::Info));
    assert!(toast.offers_undo(), "an undoable verb's toast offers undo");

    toast.show_action_completed("Marked as read", false);
    assert_eq!(toast.tone(), Some(Tone::Info));
    assert!(
        !toast.offers_undo(),
        "nothing to take back, nothing offered"
    );

    toast.show_notice("The mail is still opening");
    assert_eq!(
        toast.tone(),
        Some(Tone::Warning),
        "a notice says a gesture could not run"
    );
    assert!(!toast.offers_undo());

    toast.show_removable("Account removed", || {});
    assert_eq!(toast.tone(), Some(Tone::Info));
    assert!(toast.offers_undo());

    toast.show_undo_performed("Archived 1 message, undone");
    assert_eq!(toast.tone(), Some(Tone::Success));
    assert!(
        !toast.offers_undo(),
        "an undo is confirmed, not offered for undoing again"
    );

    toast
        .showing()
        .expect("the confirmation is showing")
        .dismiss();
    assert_eq!(toast.tone(), None, "a dismissed toast says nothing");
    assert!(!toast.offers_undo());
}
