import PostioFFI
import SwiftUI

/// The Contacts view in the main window's place (handoff screens 01 to 03,
/// `g c`): the strip with its kind tabs, the filter and New contact; the
/// "Worth a look" line; the list in letter groups beside the card; and the
/// footer. Every word is the controller's (`ContactsViewFfi`, the rows and
/// the card), and the keys reach it through the key monitor.
public struct ContactsScreen: View {
    let model: ContactsModel
    @FocusState private var filtering: Bool

    public init(model: ContactsModel) {
        self.model = model
    }

    /// The list's width, and a row's height, from the spec.
    public static let listWidth: CGFloat = 440
    public static let rowHeight: CGFloat = 52
    public static let letterHeight: CGFloat = 26

    public var body: some View {
        if let view = model.view {
            VStack(spacing: 0) {
                strip(view.strip)
                Divider()
                if let review = view.strip.review { reviewLine(review) }
                HStack(spacing: 0) {
                    list(view)
                        .frame(width: Self.listWidth)
                    Divider()
                    ContactCardView(card: model.card, run: { model.run($0) })
                        .frame(maxWidth: .infinity)
                }
                Divider()
                footer(view.footer)
            }
            .background(Color(nsColor: .textBackgroundColor))
            .accessibilityElement(children: .contain)
            .accessibilityLabel(view.strip.title)
            .onChange(of: model.filterRequest) { _, _ in filtering = true }
        }
    }

    // MARK: the strip

    private func strip(_ strip: ContactsStripFfi) -> some View {
        HStack(spacing: 10) {
            HStack(spacing: 6) {
                Text(strip.title).font(.system(size: 14.5, weight: .bold))
                Image(systemName: "chevron.down").font(.system(size: 9, weight: .semibold))
                if let cap = KeyCapSpelling.cap(strip.key) { KeyCap(cap) }
            }
            Text(strip.total).font(.system(size: 12.5)).foregroundStyle(.secondary)
            HStack(spacing: 2) {
                ForEach(Array(strip.tabs.enumerated()), id: \.offset) { _, tab in
                    Button { model.choose(tab.tab) } label: {
                        HStack(spacing: 5) {
                            Text(tab.title).font(.system(size: 12.5, weight: tab.on ? .bold : .medium))
                            if let count = tab.count {
                                Text(count).font(.system(size: 12.5)).foregroundStyle(.secondary)
                            }
                        }
                        .padding(.horizontal, 11)
                        .frame(height: 26)
                        .background(
                            RoundedRectangle(cornerRadius: 6)
                                .fill(tab.on ? Color(nsColor: .controlBackgroundColor) : .clear)
                                .shadow(color: .black.opacity(tab.on ? 0.14 : 0), radius: 1, y: 1)
                        )
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(tab.on ? [.isSelected, .isButton] : .isButton)
                }
            }
            .padding(2)
            .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary))
            Spacer(minLength: 0)
            filterField(strip)
            newContact(strip)
        }
        .lineLimit(1)
        .padding(.horizontal, 20)
        .frame(height: 46)
    }

    private func filterField(_ strip: ContactsStripFfi) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").font(.system(size: 11)).foregroundStyle(.secondary)
            TextField(
                strip.filterPlaceholder,
                text: Binding(get: { model.filter }, set: { model.filter = $0; model.typed($0) })
            )
            .textFieldStyle(.plain)
            .font(.system(size: 12.5))
            .focused($filtering)
            if let cap = KeyCapSpelling.cap(strip.filterKey) { KeyCap(cap) }
        }
        .padding(.horizontal, 8)
        .frame(width: 240, height: 26)
        .background(RoundedRectangle(cornerRadius: 6).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator))
    }

    private func newContact(_ strip: ContactsStripFfi) -> some View {
        HStack(spacing: 6) {
            Text(strip.newLabel).font(.system(size: 12.5, weight: .semibold))
            if let cap = KeyCapSpelling.cap(strip.newKey) { KeyCap(cap) }
        }
        .padding(.horizontal, 10)
        .frame(height: 28)
        .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(0.07)))
        .opacity(strip.newEnabled ? 1 : 0.45)
    }

    private func reviewLine(_ line: ContactReviewLineFfi) -> some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Circle().fill(.secondary).frame(width: 6, height: 6)
                Text(line.lead).font(.system(size: 12.5, weight: .semibold))
                Text(line.text).font(.system(size: 12.5)).foregroundStyle(.secondary)
                Spacer(minLength: 0)
                HStack(spacing: 6) {
                    Text(line.button).font(.system(size: 12.5, weight: .semibold))
                    if let cap = KeyCapSpelling.cap(line.key) { KeyCap(cap) }
                }
                .opacity(line.enabled ? 1 : 0.45)
            }
            .lineLimit(1)
            .padding(.horizontal, 20)
            .frame(height: 36)
            .background(Color(nsColor: .windowBackgroundColor))
            Divider()
        }
    }

    // MARK: the list

    private func list(_ view: ContactsViewFfi) -> some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(0..<model.total, id: \.self) { index in
                        ContactRowView(
                            row: model.row(at: index), selected: model.cursor == index,
                            point: { model.point(index) })
                            .id(index)
                    }
                }
            }
            .onChange(of: model.cursor) { _, cursor in
                if let cursor { proxy.scrollTo(cursor) }
            }
        }
        .background(Color(nsColor: .textBackgroundColor))
        .accessibilityLabel("Contacts")
    }

    // MARK: the footer

    private func footer(_ footer: ContactsFooterFfi) -> some View {
        HStack(spacing: 14) {
            FocusHintRow(footer.hints.map { FocusHintRow.Hint(cap: KeyCapSpelling.cap($0.key) ?? $0.key, label: $0.label) })
            Spacer(minLength: 0)
            Text(footer.status).font(.system(size: 12)).foregroundStyle(.secondary)
        }
        .lineLimit(1)
        .padding(.horizontal, 20)
        .frame(height: 30)
        .background(.bar)
    }
}

/// One row of the list: the 32 avatar, the name with its tag, the address in
/// the monospaced face, and on the right when and what the mail adds up to.
/// A letter heading sits over the first row of a letter.
struct ContactRowView: View {
    let row: ContactRowViewFfi?
    let selected: Bool
    let point: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            if let letter = row?.letter {
                Text(letter)
                    .font(.system(size: 11.5, weight: .bold))
                    .foregroundStyle(.secondary)
                    .padding(.leading, 20)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .frame(height: ContactsScreen.letterHeight)
                    .background(Color(nsColor: .windowBackgroundColor))
                    .accessibilityAddTraits(.isHeader)
            }
            Group {
                if let row {
                    HStack(spacing: 10) {
                        ContactAvatarView(avatar: row.avatar, size: 32, radius: 8)
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                Text(row.name).font(.system(size: 13.5, weight: .semibold)).lineLimit(1)
                                if let tag = row.tag {
                                    Text(tag)
                                        .font(.system(size: 10.5)).foregroundStyle(.secondary)
                                        .padding(.horizontal, 6).padding(.vertical, 1)
                                        .overlay(Capsule().strokeBorder(.separator))
                                }
                            }
                            Text(row.address)
                                .font(.system(size: 11.5, design: .monospaced))
                                .foregroundStyle(.tertiary).lineLimit(1)
                        }
                        Spacer(minLength: 6)
                        VStack(alignment: .trailing, spacing: 2) {
                            Text(row.when).font(.system(size: 12))
                            Text(row.countLine).font(.system(size: 11.5)).foregroundStyle(.tertiary)
                        }
                        .lineLimit(1)
                    }
                    .padding(.horizontal, 14)
                } else {
                    Color.clear
                }
            }
            .frame(height: ContactsScreen.rowHeight)
            .background(selected ? Color.accentColor.opacity(0.08) : Color.clear)
            .overlay(selected ? RoundedRectangle(cornerRadius: 0).strokeBorder(Color.accentColor, lineWidth: 2) : nil)
            .contentShape(Rectangle())
            .onTapGesture(perform: point)
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// An avatar: the engine's initials or glyph on its colour, a circle for a
/// person and a rounded square for the rest.
struct ContactAvatarView: View {
    let avatar: AvatarFfi
    let size: CGFloat
    let radius: CGFloat

    var body: some View {
        Text(avatar.text)
            .font(.system(size: size * 0.36, weight: .bold))
            .foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(
                RoundedRectangle(cornerRadius: avatar.round ? size / 2 : radius)
                    .fill(ContactPopoverModel.colour(avatar.rgb))
            )
            .accessibilityHidden(true)
    }
}
