import PostioKit
import SwiftUI

/// The page the main window shows when the store will not open on launch
/// (specs/009-focus-macos T100): GTK's refusal page on the Mac. The heading,
/// the sentence and the one button are `StoreRefusalModel`'s; a store from
/// another build offers "Start a fresh store", which sets the old one aside
/// (not deleted) and opens a fresh one with the accounts carried across.
///
/// The button is the default, so Return answers it as GTK's does; there is
/// no key monitor yet to take Return elsewhere, because there is no session
/// to resolve keys against.
struct StoreRefusalPage: View {
    let model: StoreRefusalModel

    var body: some View {
        ContentUnavailableView {
            Label(model.refusal.heading, systemImage: "exclamationmark.triangle")
        } description: {
            Text(model.refusal.sentence)
                .frame(maxWidth: 520)
        } actions: {
            Button(model.button) {
                Task { await model.act() }
            }
            .keyboardShortcut(.defaultAction)
            // A default button is filled with the label colour, lettered
            // in the background's (contracts/mac-surfaces.md): the accent
            // stays the markers' and the focus ring's.
            .buttonStyle(.borderedProminent)
            .tint(.primary)
            .disabled(model.working)
        }
    }
}
