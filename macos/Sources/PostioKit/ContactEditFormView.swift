import PostioFFI
import SwiftUI

/// The Edit and New contact form, in the card's place (handoff screen 06):
/// an accent-tinted bar with Cancel and Save, then the form, labels
/// right-aligned in a 110 column over fields up to 560 wide. Every word, the
/// rules for what may be saved and the Esc ladder are the controller's
/// (`ContactEditViewFfi`); this draws them and reports each change.
struct ContactEditFormView: View {
    let edit: ContactEditViewFfi
    let model: ContactsModel

    private enum Field: Hashable {
        case given
        case address(Int)
        case group
    }

    @FocusState private var focused: Field?

    static let labelWidth: CGFloat = 110
    static let fieldWidth: CGFloat = 560

    var body: some View {
        VStack(spacing: 0) {
            bar
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    row("Photo") { photo }
                    row("Kind") { segments(edit.kinds, note: edit.kindNote) { index in
                        model.change(.kind(kind: Self.kinds[index]))
                    } }
                    row("Name") { names }
                    row("Show as") { segments(edit.showAs, note: edit.showNote) { index in
                        model.change(.showAs(style: Self.styles[index]))
                    } }
                    row("Work") { work }
                    row("Addresses") { addresses }
                    if !edit.seen.isEmpty { row("Seen in mail") { seen } }
                    row("Groups") { groups }
                    if let problem = edit.problem {
                        row("") {
                            Text(problem).font(.system(size: 12.5, weight: .semibold))
                                .foregroundStyle(.red)
                        }
                    }
                }
                .padding(.init(top: 22, leading: 36, bottom: 22, trailing: 36))
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(edit.title)
        .onAppear { focus(edit.focus) }
        .onChange(of: edit.focus) { _, next in focus(next) }
    }

    private static let kinds: [ContactKindFfi] = [.person, .company, .list, .newsletter, .automated]
    private static let styles: [ShowAsFfi] = [.full, .first, .lastFirst]
    private static let labels: [AddressLabelFfi] = [.work, .personal, .old]

    private func focus(_ field: FormFieldFfi) {
        switch field {
        case .given: focused = .given
        case let .address(index): focused = .address(Int(index))
        case .group:
            focused = .group
        }
    }

    // MARK: the bar

    private var bar: some View {
        HStack(spacing: 10) {
            Text(edit.title).font(.system(size: 13, weight: .bold))
            Text(edit.note).font(.system(size: 12)).foregroundStyle(.secondary)
            if let armed = edit.armed {
                Text(armed).font(.system(size: 12, weight: .semibold)).foregroundStyle(.orange)
            }
            Spacer(minLength: 0)
            barButton(edit.cancel, primary: false) { model.cancel() }
            barButton(edit.save, primary: true) { model.save() }
        }
        .lineLimit(1)
        .padding(.leading, 36).padding(.trailing, 20)
        .frame(height: 44)
        .background(Color.accentColor.opacity(0.12))
        .overlay(alignment: .bottom) { Divider() }
    }

    private func barButton(_ button: EditButtonFfi, primary: Bool, _ action: @escaping () -> Void)
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

    // MARK: rows

    private func row<Content: View>(_ label: String, @ViewBuilder _ content: () -> Content)
        -> some View
    {
        HStack(alignment: .top, spacing: 16) {
            Text(label).font(.system(size: 12.5)).foregroundStyle(.tertiary)
                .frame(width: Self.labelWidth, alignment: .trailing).padding(.top, 7)
            content().frame(maxWidth: Self.fieldWidth, alignment: .leading)
        }
    }

    private var photo: some View {
        HStack(spacing: 12) {
            ContactAvatarView(avatar: edit.avatar, size: 48, radius: 12)
            Text(edit.photoNote).font(.system(size: 12)).foregroundStyle(.tertiary)
        }
    }

    private func segments(_ items: [EditSegmentFfi], note: String, _ pick: @escaping (Int) -> Void)
        -> some View
    {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 2) {
                ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                    Button { pick(index) } label: {
                        Text(item.label)
                            .font(.system(size: 12.5, weight: item.on ? .bold : .medium))
                            .padding(.horizontal, 11).frame(height: 26)
                            .background(
                                RoundedRectangle(cornerRadius: 6)
                                    .fill(item.on ? Color(nsColor: .controlBackgroundColor) : .clear)
                                    .shadow(color: .black.opacity(item.on ? 0.14 : 0), radius: 1, y: 1))
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(item.on ? [.isSelected, .isButton] : .isButton)
                }
                Spacer(minLength: 0)
            }
            .padding(2)
            .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary))
            Text(note).font(.system(size: 11.5)).foregroundStyle(.tertiary)
        }
    }

    private var names: some View {
        HStack(spacing: 8) {
            field(text: edit.given, focus: .given) { model.change(.given(text: $0)) }
            field(text: edit.family, focus: nil) { model.change(.family(text: $0)) }
        }
    }

    private var work: some View {
        HStack(spacing: 8) {
            field(text: edit.workTitle, focus: nil) { model.change(.title(text: $0)) }
            field(text: edit.workCompany, focus: nil) { model.change(.company(text: $0)) }
        }
    }

    private func field(
        text: String, focus: Field?, mono: Bool = false, _ set: @escaping (String) -> Void
    ) -> some View {
        let binding = Binding(get: { text }, set: set)
        let isFocused = focus != nil && focused == focus
        return Group {
            if let focus {
                TextField("", text: binding).focused($focused, equals: focus)
            } else {
                TextField("", text: binding)
            }
        }
        .textFieldStyle(.plain)
        .font(.system(size: mono ? 13 : 14, design: mono ? .monospaced : .default))
        .padding(.horizontal, 10).frame(height: 32)
        .overlay(
            RoundedRectangle(cornerRadius: 7)
                .strokeBorder(isFocused ? Color.accentColor : Color.primary.opacity(0.18),
                              lineWidth: isFocused ? 2 : 1))
    }

    private var addresses: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(edit.addresses.enumerated()), id: \.offset) { index, address in
                HStack(spacing: 8) {
                    Button { model.change(.primary(index: UInt32(index))) } label: {
                        Circle()
                            .strokeBorder(address.primary ? Color.primary : Color.secondary,
                                          lineWidth: address.primary ? 5 : 1.5)
                            .frame(width: 14, height: 14)
                            .frame(width: 22)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(address.primary ? "Primary address" : "Make primary")
                    field(text: address.text, focus: .address(index), mono: true) {
                        model.change(.address(index: UInt32(index), text: $0))
                    }
                    Menu {
                        ForEach(Array(Self.labels.enumerated()), id: \.offset) { _, label in
                            Button(Self.title(label)) {
                                model.change(.label(index: UInt32(index), label: label))
                            }
                        }
                        Button("Other") { model.change(.label(index: UInt32(index), label: nil)) }
                    } label: {
                        HStack {
                            Text(address.label).font(.system(size: 12.5))
                            Spacer(minLength: 0)
                            Image(systemName: "chevron.down").font(.system(size: 9, weight: .semibold))
                        }
                        .padding(.leading, 10).padding(.trailing, 8)
                    }
                    .menuStyle(.borderlessButton).menuIndicator(.hidden)
                    .frame(width: 120, height: 32)
                    .background(RoundedRectangle(cornerRadius: 7).fill(Color.primary.opacity(0.07)))
                    Button { model.change(.removeAddress(index: UInt32(index))) } label: {
                        Text("\u{2212}").font(.system(size: 16)).foregroundStyle(.tertiary)
                            .frame(width: 22)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Remove address")
                }
            }
            Button { model.change(.addAddress) } label: {
                Text(edit.addAddress).font(.system(size: 12.5, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            Text(edit.addressNote).font(.system(size: 11.5)).foregroundStyle(.tertiary)
        }
    }

    private static func title(_ label: AddressLabelFfi) -> String {
        switch label {
        case .work: "Work"
        case .personal: "Personal"
        case .old: "Old"
        }
    }

    private var seen: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(edit.seen.enumerated()), id: \.offset) { _, line in
                HStack(spacing: 10) {
                    (Text(line.what).font(.system(size: 13))
                        + Text("  " + line.from).font(.system(size: 11.5)).foregroundColor(.secondary))
                    Spacer(minLength: 0)
                    Text(line.action).font(.system(size: 12, weight: .semibold)).foregroundStyle(.secondary)
                }
                .padding(.horizontal, 10).frame(minHeight: 30)
                .background(RoundedRectangle(cornerRadius: 7).fill(Color(nsColor: .windowBackgroundColor)))
            }
        }
    }

    private var groups: some View {
        VStack(alignment: .leading, spacing: 6) {
            WrapLayout(spacing: 6) {
                ForEach(Array(edit.groups.enumerated()), id: \.offset) { index, name in
                    Button { model.change(.removeGroup(index: UInt32(index))) } label: {
                        Text("\(name) \u{00d7}").font(.system(size: 12.5))
                            .padding(.horizontal, 10).frame(height: 26)
                            .background(Capsule().fill(Color.primary.opacity(0.07)))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("\(name), remove from group")
                }
                if let picker = edit.groupPicker {
                    TextField(
                        "Group name",
                        text: Binding(
                            get: { picker.query },
                            set: { model.change(.groupQuery(query: $0)) })
                    )
                    .textFieldStyle(.plain).font(.system(size: 12.5))
                    .focused($focused, equals: .group)
                    .padding(.horizontal, 10).frame(width: 170, height: 26)
                    .overlay(Capsule().strokeBorder(Color.accentColor))
                    .onSubmit {
                        if !picker.options.isEmpty { model.change(.groupPick(index: 0)) }
                    }
                } else {
                    Button { model.change(.groupQuery(query: "")); focused = .group } label: {
                        Text(edit.addGroup).font(.system(size: 12.5, weight: .semibold))
                            .foregroundStyle(.secondary).padding(.leading, 4)
                    }
                    .buttonStyle(.plain)
                }
            }
            if let picker = edit.groupPicker, !picker.options.isEmpty {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(picker.options.enumerated()), id: \.offset) { index, option in
                        Button { model.change(.groupPick(index: UInt32(index))) } label: {
                            Text(option.label)
                                .font(.system(size: 12.5, weight: option.isNew ? .semibold : .regular))
                                .padding(.horizontal, 10).frame(maxWidth: .infinity, minHeight: 26, alignment: .leading)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .frame(width: 220)
                .background(RoundedRectangle(cornerRadius: 7).fill(Color(nsColor: .controlBackgroundColor)))
                .overlay(RoundedRectangle(cornerRadius: 7).strokeBorder(.separator))
            }
            Text(edit.groupNote).font(.system(size: 11.5)).foregroundStyle(.tertiary)
        }
    }
}
