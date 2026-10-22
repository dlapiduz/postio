import PostioFFI
import SwiftUI

/// A contact's card, beside the list (handoff screens 01 and 03): the 64
/// avatar, the name and the role line with the head's buttons; then two
/// columns. Left, the addresses, how mail arrives and its switches, and the
/// notes. Right, the tiles, the twelve-month strip, what is open between you
/// (a newsletter's recent issues) and the recent files. Every word is the
/// controller's (`ContactCardViewFfi`), a person's or a newsletter's alike.
struct ContactCardView: View {
    let card: ContactCardViewFfi?
    let run: (CardButtonFfi) -> Void
    let toggle: (Int) -> Void
    let split: (String) -> Void

    var body: some View {
        if let card {
            ScrollView {
                VStack(alignment: .leading, spacing: 22) {
                    head(card)
                    HStack(alignment: .top, spacing: 36) {
                        left(card).frame(maxWidth: .infinity, alignment: .leading)
                        right(card).frame(width: 446, alignment: .leading)
                    }
                }
                .padding(.init(top: 26, leading: 36, bottom: 26, trailing: 36))
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(card.name)
        } else {
            Color.clear
        }
    }

    // MARK: head

    private func head(_ card: ContactCardViewFfi) -> some View {
        HStack(alignment: .top, spacing: 16) {
            ContactAvatarView(avatar: card.avatar, size: 64, radius: 14)
            VStack(alignment: .leading, spacing: 3) {
                Text(card.name).font(.system(size: 24, weight: .bold)).lineLimit(1)
                Text(card.role).font(.system(size: 12.5)).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 12)
            HStack(spacing: 8) {
                ForEach(Array(card.buttons.enumerated()), id: \.offset) { _, button in
                    Button { run(button) } label: {
                        HStack(spacing: 6) {
                            Text(button.label).font(.system(size: 12.5, weight: .bold))
                            if let cap = KeyCapSpelling.cap(button.key) {
                                if button.primary {
                                    Text(cap).font(.system(size: 10, design: .monospaced))
                                        .padding(.horizontal, 4).frame(minWidth: 15, minHeight: 15)
                                        .background(RoundedRectangle(cornerRadius: 3).fill(.white.opacity(0.18)))
                                } else {
                                    KeyCap(cap)
                                }
                            }
                        }
                        .foregroundStyle(button.primary ? Color(nsColor: .textBackgroundColor) : Color.primary)
                        .padding(.horizontal, 10).frame(height: 28)
                        .background(
                            RoundedRectangle(cornerRadius: 6)
                                .fill(button.primary ? Color.primary : Color.primary.opacity(0.07)))
                        .opacity(button.enabled ? 1 : 0.45)
                    }
                    .buttonStyle(.plain)
                    .disabled(!button.enabled)
                }
            }
            .fixedSize()
        }
    }

    // MARK: left column

    private func left(_ card: ContactCardViewFfi) -> some View {
        VStack(alignment: .leading, spacing: 20) {
            section(card.addressesTitle) {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(Array(card.addresses.enumerated()), id: \.offset) { _, line in
                        HStack(alignment: .top, spacing: 8) {
                            Text(line.primary ? "\u{2605}" : "").font(.system(size: 10)).frame(width: 12)
                                .padding(.top, 3)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(line.address).font(.system(size: 12.5, design: .monospaced))
                                Text(line.note).font(.system(size: 11.5)).foregroundStyle(.secondary)
                            }
                            Spacer(minLength: 0)
                            if let bounced = line.bounced {
                                Text(bounced).font(.system(size: 11.5)).foregroundStyle(.orange)
                            }
                            if line.canSplit {
                                Button("Split") { split(line.address) }
                                    .buttonStyle(.plain).font(.system(size: 11.5, weight: .semibold))
                                    .foregroundStyle(.secondary)
                                    .accessibilityLabel("Split \(line.address) into its own contact")
                            }
                        }
                    }
                    if let seen = card.alsoSeen {
                        Text(seen).font(.system(size: 12)).foregroundStyle(.secondary)
                    }
                }
            }
            section(card.arrivalTitle) {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(spacing: 2) {
                        ForEach(Array(card.arrival.enumerated()), id: \.offset) { _, option in
                            Text(option.label)
                                .font(.system(size: 12.5, weight: option.on ? .bold : .medium))
                                .padding(.horizontal, 11).frame(height: 26)
                                .background(
                                    RoundedRectangle(cornerRadius: 6)
                                        .fill(option.on ? Color(nsColor: .controlBackgroundColor) : .clear)
                                        .shadow(color: .black.opacity(option.on ? 0.14 : 0), radius: 1, y: 1))
                        }
                        Spacer(minLength: 0)
                    }
                    .padding(2)
                    .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary))
                    ForEach(Array(card.switches.enumerated()), id: \.offset) { index, line in
                        HStack(alignment: .center, spacing: 12) {
                            VStack(alignment: .leading, spacing: 1) {
                                Text(line.title).font(.system(size: 13, weight: .semibold))
                                Text(line.note).font(.system(size: 12)).foregroundStyle(.secondary)
                            }
                            Spacer(minLength: 0)
                            Button { toggle(index) } label: { SwitchTrack(on: line.on) }
                                .buttonStyle(.plain)
                                .disabled(!line.canToggle)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
            }
            section(card.notesTitle) {
                VStack(alignment: .leading, spacing: 8) {
                    if let notes = card.notes {
                        Text(notes).font(.system(size: 13)).padding(12)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .windowBackgroundColor)))
                    }
                    Text(card.notesFoot).font(.system(size: 12)).foregroundStyle(.secondary)
                }
            }
        }
    }

    // MARK: right column

    private func right(_ card: ContactCardViewFfi) -> some View {
        VStack(alignment: .leading, spacing: 20) {
            section(card.statsTitle) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 6) {
                        ForEach(Array(card.tiles.enumerated()), id: \.offset) { _, tile in
                            VStack(alignment: .leading, spacing: 1) {
                                Text(tile.value).font(.system(size: 17, weight: .bold)).lineLimit(1)
                                Text(tile.caption).font(.system(size: 11)).foregroundStyle(.tertiary).lineLimit(2)
                            }
                            .frame(maxWidth: .infinity, minHeight: 52, alignment: .topLeading)
                            .padding(.init(top: 7, leading: 10, bottom: 7, trailing: 10))
                            .background(RoundedRectangle(cornerRadius: 7).fill(Color(nsColor: .windowBackgroundColor)))
                            .accessibilityElement(children: .combine)
                        }
                    }
                    MonthStrip(months: card.months)
                }
            }
            if !card.lines.isEmpty {
                section(card.linesTitle) {
                    VStack(spacing: 6) {
                        ForEach(Array(card.lines.enumerated()), id: \.offset) { _, line in
                            HStack(spacing: 10) {
                                Text(line.label)
                                    .font(.system(size: 12, weight: .semibold))
                                    .foregroundStyle(line.yours ? Color.accentColor : Color.secondary)
                                    .lineLimit(1).frame(width: 112, alignment: .leading)
                                Text(line.text).font(.system(size: 12.5)).lineLimit(1)
                                Spacer(minLength: 0)
                                Text(line.state).font(.system(size: 11.5)).foregroundStyle(.secondary)
                            }
                            .padding(.horizontal, 12).frame(height: 34)
                            .overlay(RoundedRectangle(cornerRadius: 7).strokeBorder(.separator))
                            .accessibilityElement(children: .combine)
                        }
                    }
                }
            }
            if let title = card.filesTitle, !card.files.isEmpty {
                section(title) {
                    WrapLayout(spacing: 8) {
                        ForEach(Array(card.files.enumerated()), id: \.offset) { _, file in
                            HStack(spacing: 6) {
                                Text(file.kind).font(.system(size: 9.5, weight: .bold, design: .monospaced))
                                    .foregroundStyle(.secondary)
                                Text(file.name).font(.system(size: 12.5))
                            }
                            .padding(.horizontal, 8).frame(height: 28)
                            .background(RoundedRectangle(cornerRadius: 6).fill(Color(nsColor: .windowBackgroundColor)))
                            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator))
                            .fixedSize()
                        }
                    }
                }
            }
        }
    }

    private func section<Content: View>(_ title: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.system(size: 12, weight: .bold)).foregroundStyle(.secondary)
            content()
        }
    }
}

/// The twelve-month strip and its axis.
private struct MonthStrip: View {
    let months: CardMonthsFfi

    var body: some View {
        VStack(spacing: 4) {
            HStack(alignment: .bottom, spacing: 3) {
                ForEach(Array(months.bars.enumerated()), id: \.offset) { _, bar in
                    RoundedRectangle(cornerRadius: 1.5)
                        .fill(Color.primary.opacity(0.18))
                        .frame(height: max(3, 44 * CGFloat(bar) / 1000))
                        .frame(maxWidth: .infinity)
                }
            }
            .frame(height: 44, alignment: .bottom)
            HStack {
                Text(months.from)
                Spacer()
                Text(months.caption)
                Spacer()
                Text(months.to)
            }
            .font(.system(size: 11)).foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(months.caption), \(months.from) to \(months.to)")
    }
}

/// A switch drawn, for what the card says: dark track with a light knob when
/// on, light track with a white knob when off. A click is the card's
/// (`toggle`), sent to the engine as a command.
private struct SwitchTrack: View {
    let on: Bool

    var body: some View {
        Capsule()
            .fill(on ? Color.primary : Color.primary.opacity(0.12))
            .frame(width: 36, height: 20)
            .overlay(alignment: on ? .trailing : .leading) {
                Circle().fill(on ? Color(nsColor: .textBackgroundColor) : .white)
                    .shadow(color: .black.opacity(0.2), radius: 1, y: 0.5)
                    .frame(width: 16, height: 16).padding(2)
            }
            .accessibilityLabel(on ? "On" : "Off")
    }
}
