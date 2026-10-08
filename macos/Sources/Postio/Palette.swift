import PostioFFI
import PostioAppKit
import PostioKit
import SwiftUI

/// The same list the palette reads, unfiltered — *"they are the same list read
/// two ways"* (#658). Built separately they would be two places deciding what
/// "available here" means, and they would disagree.
///
/// It is also the only surface that can describe a **sequence**: `g g` has no
/// accelerator spelling, so a menu cannot show it and this can. That is why
/// the cheat sheet is the Help menu's item.
struct CheatSheet: View {
    let session: PostioSession
    let context: UiContext
    let dismiss: () -> Void

    /// Read when the sheet opens, not per redraw: building it walks the
    /// registry and the keymap for every command, and nothing it depends on
    /// can change while the sheet is up. A computed property here would
    /// rebuild the whole sheet on every `body` evaluation.
    @State private var sections: [CheatSectionFfi] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Keyboard").font(.title2.bold())
                Spacer()
                Button("Done", action: dismiss).keyboardShortcut(.defaultAction)
            }
            .padding(16)
            Divider()
            CheatSheetList(sections: sections)
        }
        .frame(width: 880, height: 600)
        // Keyed on the context: `?` in the composer teaches a different
        // sheet than `?` in the list, and a sheet that kept the last one's
        // rows would be teaching keys that do not resolve here.
        .task(id: context) { sections = session.cheatSheetSections(in: context) }
    }
}
