import PostioFFI
import SwiftUI

/// The merge sheet over the Contacts view (handoff screen 07): 780 wide under
/// the strip, over a scrim. A card per record with the kept one in the accent
/// ring, "Why we think so", what the merge would make, and Not the same,
/// Later and Merge. Every word and every key is the controller's
/// (`MergeSheetViewFfi`).
struct MergeSheetView: View {
    let sheet: MergeSheetViewFfi
    let model: ContactsModel

    static let width: CGFloat = 780

    var body: some View {
        ZStack(alignment: .top) {
            Color.black.opacity(0.18).ignoresSafeArea()
            VStack(alignment: .leading, spacing: 0) {
                header
                cards
                why
                result
                foot
            }
            .frame(width: Self.width)
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 12))
            .shadow(color: .black.opacity(0.3), radius: 30, y: 18)
            .padding(.top, 0)
            .accessibilityElement(children: .contain)
            .accessibilityLabel(sheet.title)
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(sheet.title).font(.system(size: 17, weight: .bold))
                Spacer(minLength: 0)
                if let position = sheet.position {
                    Text(position).font(.system(size: 12)).foregroundStyle(.secondary)
                }
            }
            Text(sheet.subtitle).font(.system(size: 13)).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.init(top: 20, leading: 24, bottom: 14, trailing: 24))
    }

    private var cards: some View {
        HStack(spacing: 10) {
            ForEach(Array(sheet.cards.enumerated()), id: \.offset) { index, card in
                Button { model.keep(cardAt: index) } label: { cardView(card) }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(card.kept ? [.isSelected, .isButton] : .isButton)
            }
        }
        .padding(.horizontal, 24)
    }

    private func cardView(_ card: MergeCardFfi) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                ContactAvatarView(avatar: card.avatar, size: 30, radius: 8)
                VStack(alignment: .leading, spacing: 0) {
                    Text(card.name).font(.system(size: 13.5, weight: .bold)).lineLimit(1)
                    Text(card.source).font(.system(size: 11)).foregroundStyle(.tertiary)
                }
            }
            Text(card.address).font(.system(size: 12, design: .monospaced)).lineLimit(1)
            Text(card.stats).font(.system(size: 11.5)).foregroundStyle(.tertiary)
            Text(card.keep).font(.system(size: 11.5, weight: .semibold))
                .foregroundStyle(card.kept ? Color.primary : Color.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color(nsColor: .windowBackgroundColor)))
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder(card.kept ? Color.accentColor : Color.clear, lineWidth: 2))
    }

    private var why: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(sheet.whyTitle).font(.system(size: 12.5, weight: .bold)).foregroundStyle(.secondary)
            ForEach(Array(sheet.evidence.enumerated()), id: \.offset) { _, line in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text("\u{2713}").font(.system(size: 12, weight: .bold)).foregroundStyle(.secondary)
                    Text(line).font(.system(size: 13)).fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(.separator))
        .padding(.init(top: 14, leading: 24, bottom: 0, trailing: 24))
    }

    private var result: some View {
        HStack(spacing: 12) {
            ContactAvatarView(avatar: sheet.result.avatar, size: 40, radius: 10)
            VStack(alignment: .leading, spacing: 2) {
                (Text(sheet.result.name).font(.system(size: 14, weight: .bold))
                    + Text(sheet.result.role.isEmpty ? "" : "  \u{b7} " + sheet.result.role)
                    .font(.system(size: 13)).foregroundColor(.secondary))
                Text(sheet.result.addresses).font(.system(size: 11.5, design: .monospaced))
                    .foregroundStyle(.secondary).lineLimit(1)
                Text(sheet.result.line).font(.system(size: 11.5)).foregroundStyle(.tertiary)
            }
            Spacer(minLength: 0)
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color.accentColor.opacity(0.10)))
        .padding(.init(top: 12, leading: 24, bottom: 0, trailing: 24))
    }

    private var foot: some View {
        HStack(spacing: 8) {
            Text(sheet.foot).font(.system(size: 12)).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 12)
            button(sheet.notSame, primary: false, command: "suggestion_dismiss")
            button(sheet.later, primary: false, command: "back")
            button(sheet.merge, primary: true, command: "contact_show_mail")
        }
        .padding(.init(top: 14, leading: 24, bottom: 18, trailing: 24))
    }

    private func button(_ button: EditButtonFfi, primary: Bool, command: String) -> some View {
        Button { model.choose(button, command: command) } label: {
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
