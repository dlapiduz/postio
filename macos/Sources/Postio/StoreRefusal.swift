import PostioKit
import SwiftUI

/// The button is the default, so Return answers it; there is no key monitor yet because there is
/// no session to resolve keys against.
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
            .buttonStyle(.borderedProminent)
            .tint(.primary)
            .disabled(model.working)
        }
    }
}
