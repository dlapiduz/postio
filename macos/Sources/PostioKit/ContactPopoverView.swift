import PostioFFI
import SwiftUI

/// The known-sender popover's content (handoff screen 04): 400 wide, the
/// avatar with the name, address and role, three tiles, the open lines, the
/// arrival control, and the actions on the window's own tint. Every word is
/// the controller's (`ContactPopoverViewFfi`).
public struct ContactPopoverView: View {
    let model: ContactPopoverModel

    public init(model: ContactPopoverModel) {
        self.model = model
    }

    public static let width: CGFloat = 400

    public var body: some View {
        if let view = model.view {
            VStack(alignment: .leading, spacing: 0) {
                header(view)
                if let intro = view.intro {
                    introduction(intro)
                } else {
                    tiles(view)
                    if !view.open.isEmpty { openLines(view) }
                }
                arrival(view)
                actions(view)
            }
            .frame(width: Self.width)
            // An opaque card on the view's own colour, as screen 04 draws
            // it, not the popover's vibrant material.
            .background(Color(nsColor: .textBackgroundColor))
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Contact")
        }
    }

    private func header(_ view: ContactPopoverViewFfi) -> some View {
        HStack(spacing: 12) {
            Text(view.avatar.text)
                .font(.system(size: 16, weight: .bold))
                .foregroundStyle(.white)
                .frame(width: 44, height: 44)
                .background(
                    RoundedRectangle(cornerRadius: view.avatar.round ? 22 : 12)
                        .fill(ContactPopoverModel.colour(view.avatar.rgb))
                )
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                Text(view.name).font(.system(size: 15, weight: .bold))
                Text(view.subline).font(.system(size: 11.5, design: .monospaced))
                    .foregroundStyle(.tertiary)
                Text(view.intro?.sent ?? view.role).font(.system(size: 12)).foregroundStyle(.secondary)
            }
            .lineLimit(1)
            Spacer(minLength: 0)
        }
        .padding(.init(top: 16, leading: 16, bottom: 12, trailing: 16))
    }

    /// A sender not yet saved (handoff screen 05): how they reached you, and
    /// what their signature said, read on this Mac and kept only on Add.
    private func introduction(_ intro: PopoverIntroFfi) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(intro.status).font(.system(size: 12.5)).fixedSize(horizontal: false, vertical: true)
            if let title = intro.signatureTitle {
                VStack(alignment: .leading, spacing: 4) {
                    Text(title).font(.system(size: 11.5, weight: .bold)).foregroundStyle(.secondary)
                    ForEach(Array(intro.signatureLines.enumerated()), id: \.offset) { index, line in
                        Text(line)
                            .font(.system(size: 12.5, design: index == 0 ? .default : .monospaced))
                    }
                    if let foot = intro.signatureFoot {
                        Text(foot).font(.system(size: 11)).foregroundStyle(.tertiary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
                .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .windowBackgroundColor)))
                .accessibilityElement(children: .combine)
            }
        }
        .padding(.init(top: 0, leading: 16, bottom: 12, trailing: 16))
    }

    private func tiles(_ view: ContactPopoverViewFfi) -> some View {
        HStack(spacing: 6) {
            ForEach(Array(view.tiles.enumerated()), id: \.offset) { _, tile in
                VStack(alignment: .leading, spacing: 1) {
                    Text(tile.value).font(.system(size: 14, weight: .bold))
                    Text(tile.caption).font(.system(size: 10.5)).foregroundStyle(.tertiary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.init(top: 7, leading: 9, bottom: 7, trailing: 9))
                .background(RoundedRectangle(cornerRadius: 7).fill(Color(nsColor: .windowBackgroundColor)))
                .accessibilityElement(children: .combine)
            }
        }
        .padding(.init(top: 0, leading: 16, bottom: 12, trailing: 16))
    }

    private func openLines(_ view: ContactPopoverViewFfi) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(Array(view.open.enumerated()), id: \.offset) { _, line in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(line.label)
                        .font(.system(size: 11.5, weight: .semibold))
                        .foregroundStyle(line.yours ? Color.accentColor : Color.secondary)
                        .lineLimit(1).frame(width: 112, alignment: .leading)
                    Text(line.text).font(.system(size: 12.5)).lineLimit(1)
                }
                .accessibilityElement(children: .combine)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.init(top: 0, leading: 16, bottom: 12, trailing: 16))
    }

    private func arrival(_ view: ContactPopoverViewFfi) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Text(view.arrivalTitle).font(.system(size: 11.5, weight: .bold)).foregroundStyle(.secondary)
                Spacer(minLength: 0)
                ForEach(Array(view.hints.enumerated()), id: \.offset) { _, hint in
                    HStack(spacing: 4) {
                        if let cap = KeyCapSpelling.cap(hint.key) { KeyCap(cap) }
                        Text(hint.label).font(.system(size: 10.5)).foregroundStyle(.tertiary)
                    }
                }
            }
            HStack(spacing: 2) {
                ForEach(Array(view.arrival.enumerated()), id: \.offset) { _, option in
                    Button { model.choose(option.arrival) } label: {
                        Text(option.label)
                            .font(.system(size: 12.5, weight: option.on ? .bold : .medium))
                            .padding(.horizontal, 11)
                            .frame(height: 26)
                            .background(
                                RoundedRectangle(cornerRadius: 6)
                                    .fill(option.on ? Color(nsColor: .controlBackgroundColor) : .clear)
                                    .shadow(color: .black.opacity(option.on ? 0.14 : 0), radius: 1, y: 1)
                            )
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(option.on ? [.isSelected, .isButton] : .isButton)
                }
                Spacer(minLength: 0)
            }
            .padding(2)
            .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.init(top: 10, leading: 16, bottom: 12, trailing: 16))
        .overlay(alignment: .top) { Divider() }
    }

    private func actions(_ view: ContactPopoverViewFfi) -> some View {
        HStack(spacing: 6) {
            ForEach(Array(view.actions.enumerated()), id: \.offset) { _, action in
                Button { model.run(action) } label: {
                    HStack(spacing: 6) {
                        Text(action.label).font(.system(size: 12.5, weight: .semibold))
                        if let cap = KeyCapSpelling.cap(action.key) {
                            if action.primary {
                                Text(cap)
                                    .font(.system(size: 10, design: .monospaced))
                                    .padding(.horizontal, 4)
                                    .frame(minWidth: 15, minHeight: 15)
                                    .background(RoundedRectangle(cornerRadius: 3).fill(.white.opacity(0.18)))
                            } else {
                                KeyCap(cap)
                            }
                        }
                    }
                    .foregroundStyle(
                        action.primary ? Color(nsColor: .textBackgroundColor) : Color.primary
                    )
                    .padding(.init(top: 0, leading: 10, bottom: 0, trailing: 8))
                    .frame(height: 28)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(action.primary ? Color.primary : Color.primary.opacity(0.07))
                    )
                    .opacity(action.enabled ? 1 : 0.4)
                }
                .buttonStyle(.plain)
                .disabled(!action.enabled)
            }
            Spacer(minLength: 0)
        }
        .padding(.init(top: 10, leading: 12, bottom: 12, trailing: 12))
        .background(Color(nsColor: .windowBackgroundColor))
        .overlay(alignment: .top) { Divider() }
    }
}
