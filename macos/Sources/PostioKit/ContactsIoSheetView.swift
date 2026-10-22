import PostioFFI
import SwiftUI

/// The sheet after an import or an export (specs/011-focus-contacts S11): a
/// card over a scrim with what happened, a line each, and Review (when the
/// import left people to look at) and Done. Every word and key is Rust's.
public struct ContactsIoSheetView: View {
    let sheet: ContactsIoSheetFfi
    let model: ContactsIoModel

    public init(sheet: ContactsIoSheetFfi, model: ContactsIoModel) {
        self.sheet = sheet
        self.model = model
    }

    public var body: some View {
        ZStack {
            Color.black.opacity(0.18).ignoresSafeArea()
            VStack(alignment: .leading, spacing: 12) {
                Text(sheet.title).font(.system(size: 17, weight: .bold))
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(sheet.lines.enumerated()), id: \.offset) { index, line in
                        Text(line)
                            .font(.system(size: 13))
                            .foregroundStyle(index == sheet.lines.count - 1 ? .secondary : .primary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                HStack(spacing: 8) {
                    Spacer(minLength: 0)
                    button(sheet.done, primary: sheet.review == nil) { model.done() }
                    if let review = sheet.review { button(review, primary: true) { model.review() } }
                }
                .padding(.top, 6)
            }
            .padding(24)
            .frame(width: 460, alignment: .leading)
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 12))
            .shadow(color: .black.opacity(0.3), radius: 30, y: 18)
            .accessibilityElement(children: .contain)
            .accessibilityLabel(sheet.title)
        }
    }

    private func button(_ button: EditButtonFfi, primary: Bool, _ action: @escaping () -> Void)
        -> some View
    {
        Button(action: action) {
            HStack(spacing: 6) {
                Text(button.label).font(.system(size: 12.5, weight: .semibold))
                if let cap = KeyCapSpelling.cap(button.key) {
                    if primary {
                        Text(cap).font(.system(size: 10, design: .monospaced))
                            .padding(.horizontal, 4).frame(minWidth: 15, minHeight: 15)
                            .background(RoundedRectangle(cornerRadius: 3).fill(.white.opacity(0.18)))
                    } else {
                        KeyCap(cap)
                    }
                }
            }
            .foregroundStyle(primary ? Color(nsColor: .textBackgroundColor) : Color.primary)
            .padding(.leading, 10).padding(.trailing, 8).frame(height: 28)
            .background(
                RoundedRectangle(cornerRadius: 6)
                    .fill(primary ? Color.primary : Color.primary.opacity(0.07)))
        }
        .buttonStyle(.plain)
    }
}
