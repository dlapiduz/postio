import PostioFFI
import SwiftUI

/// The settings window: a fixed sidebar, exactly one pane, and a footer
/// naming the table that pane writes.
///
/// The frame does not move; only the sidebar's selection and the pane body do.
/// That is the whole navigation model, and it is the GTK window's — the eight
/// sections, their order, their two headings and their labels all come from
/// `postio_ui::settings` rather than from a list kept beside this view, so the
/// two frontends cannot drift into being two different applications.
///
/// # The controls are decided, not chosen
///
/// ADR 0029: segmented for a closed set of three or four, a checkbox for a
/// value in a form, a switch only for something that *acts* when flipped, a
/// dropdown only for an open list. SwiftUI's `Picker` defaults to a dropdown
/// on macOS, which is why every one here is `.segmented` explicitly — Theme
/// and Row density are closed sets of three, and a popup for three options is
/// the idiom that ADR exists to stop.
public struct SettingsPaneView: View {
    @Bindable private var store: SettingsStore
    /// The configured accounts, from the session rather than from
    /// `config.toml`: the store is the truth about which accounts exist.
    private let accounts: [AccountFfi]

    /// The session, for the things the accounts pane can actually *do* —
    /// adding one, mostly. `nil` when the store never opened, in which case
    /// the pane still draws: settings are a file, and being unable to read
    /// mail is not being unable to configure it.
    private let session: PostioSession?

    /// Which account's form is open, if any.
    @State private var selected: Int64?
    /// The add-account sheet, while it is up.
    @State private var adding: AddAccountModel?

    public init(
        store: SettingsStore,
        accounts: [AccountFfi] = [],
        session: PostioSession? = nil
    ) {
        self.store = store
        self.accounts = accounts
        self.session = session
    }

    public var body: some View {
        HStack(spacing: 0) {
            sidebar
            Divider()
            VStack(alignment: .leading, spacing: 0) {
                detail
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                Divider()
                footer
            }
        }
        .frame(minWidth: 720, minHeight: 460)
    }

    // MARK: - Sidebar

    private var sidebar: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                ForEach([GroupFfi.mail, GroupFfi.application], id: \.self) { group in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(settingsGroupLabel(group: group))
                            .font(.system(size: 10, weight: .semibold))
                            .kerning(0.6)
                            .foregroundStyle(.tertiary)
                            .padding(.horizontal, 14)
                            .padding(.bottom, 4)
                        ForEach(store.sections(in: group), id: \.key) { section in
                            row(section)
                        }
                    }
                }
            }
            .padding(.vertical, 16)
        }
        // The GTK window's fixed 214px sidebar. Fixed rather than resizable
        // because the pane is the thing that varies, and a settings window
        // whose nav can be dragged to nothing is a settings window with a bug
        // report waiting in it.
        .frame(width: 214)
        .background(.quaternary.opacity(0.35))
    }

    private func row(_ section: SettingsSectionFfi) -> some View {
        let selected = section.key == store.selected
        return Button {
            store.selected = section.key
        } label: {
            HStack(spacing: 8) {
                Image(systemName: symbol(for: section.key))
                    .frame(width: 16)
                    .foregroundStyle(selected ? AnyShapeStyle(.white) : AnyShapeStyle(.secondary))
                Text(section.label)
                    .foregroundStyle(selected ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
                Spacer(minLength: 0)
                // How many there are, where the design puts it. Only for
                // Accounts, and only when there are some: a "0" beside a
                // section is a fact nobody needed.
                if section.key == "accounts", !accounts.isEmpty {
                    Text("\(accounts.count)")
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(selected ? AnyShapeStyle(.white) : AnyShapeStyle(.tertiary))
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(selected ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.clear))
            .clipShape(RoundedRectangle(cornerRadius: 6))
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 8)
        .accessibilityAddTraits(selected ? [.isSelected] : [])
    }

    /// The macOS half of what `postio_gtk::settings::icon` answers for GTK.
    ///
    /// Beside the view rather than behind the boundary for the reason that
    /// function records: a symbolic icon name is not an SF Symbol, and the
    /// shared crate should carry neither.
    private func symbol(for key: String) -> String {
        switch key {
        case "accounts": return "person.crop.circle"
        case "filters": return "line.3.horizontal.decrease.circle"
        case "compose": return "square.and.pencil"
        case "ui": return "paintbrush"
        case "keys": return "keyboard"
        case "sync": return "arrow.triangle.2.circlepath"
        case "privacy": return "lock.shield"
        default: return "doc.plaintext"
        }
    }

    // MARK: - Detail

    @ViewBuilder private var detail: some View {
        if let section = store.current {
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(section.label).font(.system(size: 17, weight: .semibold))
                    Text(section.description).font(.callout).foregroundStyle(.secondary)
                }
                .padding(.horizontal, 24)
                .padding(.top, 20)
                .padding(.bottom, 16)
                Divider()
                pane
                    .padding(24)
            }
        }
    }

    @ViewBuilder private var pane: some View {
        switch store.selected {
        case "ui": appearance
        case "accounts": accountsPane
        default: unbuilt
        }
    }

    /// The accounts list. No form and no Add button yet — #1206 ships the
    /// list first, because a pane that cannot even show what is configured is
    /// the part that makes the rest unverifiable.
    @ViewBuilder private var accountsPane: some View {
        VStack(alignment: .leading, spacing: 0) {
            if accounts.isEmpty {
                ContentUnavailableView {
                    Label("No accounts", systemImage: "person.crop.circle.badge.questionmark")
                } description: {
                    Text(AccountRow.emptyMessage)
                }
                .frame(maxHeight: .infinity)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(accounts, id: \.id) { account in
                            accountRow(account)
                            if account.id != accounts.last?.id { Divider() }
                        }
                    }
                }
            }
            Divider()
            // Under the list, not in the sidebar: the buttons act on *this*
            // list, and a `+` in the section nav would read as "add a section"
            // (canvas 27).
            HStack(spacing: 6) {
                Button {
                    adding = AddAccountModel()
                } label: {
                    Image(systemName: "plus")
                }
                .help("Add an account")
                .accessibilityLabel("Add an account")
                Button {
                    // Removing an account takes its mail with it, so it is a
                    // confirmed action rather than a button (#1277).
                } label: {
                    Image(systemName: "minus")
                }
                .disabled(true)
                .help("Removing an account is not built yet")
                .accessibilityLabel("Remove the selected account")
                Spacer()
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
        .sheet(item: $adding) { model in
            AddAccountSheet(session: session, model: model) { adding = nil }
        }
    }

    /// One account, and its form when it is the selected one.
    @ViewBuilder private func accountRow(_ account: AccountFfi) -> some View {
        let open = selected == account.id
        VStack(alignment: .leading, spacing: 0) {
            Button {
                selected = open ? nil : account.id
            } label: {
                HStack(alignment: .top, spacing: 12) {
                    Text(account.initials)
                        .font(.system(size: 12, weight: .medium))
                        .frame(width: 30, height: 30)
                        .background(Color.accentColor.opacity(0.22))
                        .clipShape(Circle())
                    VStack(alignment: .leading, spacing: 3) {
                        HStack(spacing: 8) {
                            Text(account.address).font(.body)
                            if let tag = AccountRow.tag(account) {
                                Text(tag)
                                    .font(.system(.caption, design: .monospaced))
                                    .foregroundStyle(.secondary)
                            }
                        }
                        HStack(spacing: 6) {
                            if AccountRow.needsAttention(account) {
                                Image(systemName: "exclamationmark.triangle")
                                    .foregroundStyle(.secondary)
                            }
                            Text(AccountRow.line(account))
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                        }
                    }
                    Spacer(minLength: 0)
                    if AccountRow.needsAttention(account) {
                        // Inline, beside the account it is about: a token that
                        // expired is a thing to fix here rather than a banner
                        // somewhere else (#1276).
                        Button("Reconnect") {}
                            .controlSize(.small)
                            .disabled(true)
                            .help("Signing in again is not built here yet")
                    }
                }
                .contentShape(Rectangle())
                .padding(.vertical, 10)
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(open ? [.isSelected] : [])

            if open {
                accountForm(account)
            }
        }
    }

    /// What selecting an account reveals.
    private func accountForm(_ account: AccountFfi) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            field("DISPLAY NAME") {
                Text(account.displayName.isEmpty ? account.address : account.displayName)
            }
            field("LOCAL STORE") {
                Text(store.path)
                    .font(.system(.callout, design: .monospaced))
                    .textSelection(.enabled)
            }
            HStack(spacing: 8) {
                Button("Test connection") {}
                    .disabled(true)
                Button("Re-index store") {}
                    .disabled(true)
                Button("Remove account…") {}
                    .disabled(true)
            }
            // Said once, here, rather than three tooltips: these are the
            // account actions the boundary has no path for yet (#1277).
            Text("Testing, re-indexing and removing are not built here yet.")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
        .padding(.leading, 42)
        .padding(.bottom, 14)
    }

    @ViewBuilder private var appearance: some View {
        if let current = store.appearance {
            HStack(alignment: .top, spacing: 32) {
                VStack(alignment: .leading, spacing: 20) {
                    field("THEME") {
                        Picker("", selection: binding(current, \.theme)) {
                            Text("System").tag(ThemeFfi.system)
                            Text("Light").tag(ThemeFfi.light)
                            Text("Dark").tag(ThemeFfi.dark)
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                    }
                    field("ROW DENSITY") {
                        // "Snug" is what the middle setting is called on
                        // screen; `comfortable` is what it is called in the
                        // file. GTK says the same two things, and changing
                        // either alone would make one of them a lie.
                        Picker("", selection: binding(current, \.density)) {
                            Text("Airy").tag(DensityFfi.airy)
                            Text("Snug").tag(DensityFfi.comfortable)
                            Text("Compact").tag(DensityFfi.compact)
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                        // What the choice above actually costs, in the unit a
                        // person is choosing between. Measured off the real
                        // cell rather than tabulated, so it cannot drift from
                        // what the list draws -- GTK says the same sentence
                        // from the same kind of measurement.
                        Text(
                            "\(Int(MessageRowCell.preferredHeight(for: current.density, reservingHints: current.showKeyHints)))px rows"
                        )
                            .font(.system(.footnote, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                }
                Divider().frame(height: 120)
                field("MESSAGE LIST") {
                    VStack(alignment: .leading, spacing: 8) {
                        Toggle("Hover action icons", isOn: binding(current, \.showHoverActions))
                        Toggle("Key hints on the focused row", isOn: binding(current, \.showKeyHints))
                        Toggle("Sender avatars", isOn: binding(current, \.senderAvatars))
                    }
                    .toggleStyle(.checkbox)
                }
                Spacer(minLength: 0)
            }
        } else {
            unreadable
        }
    }

    private func field<Content: View>(
        _ kicker: String,
        @ViewBuilder _ content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(kicker)
                .font(.system(size: 10, weight: .semibold))
                .kerning(0.6)
                .foregroundStyle(.secondary)
            content()
        }
    }

    private var unreadable: some View {
        ContentUnavailableView {
            Label("This file will not parse", systemImage: "exclamationmark.triangle")
        } description: {
            Text(
                "Settings cannot be shown as fields until the file is valid TOML. "
                    + "The line below says where it went wrong; ⌘E opens it in your editor."
            )
        }
    }

    private var unbuilt: some View {
        ContentUnavailableView {
            Label("Not on macOS yet", systemImage: "gearshape")
        } description: {
            Text(
                "This section is only editable in the file for now — ⌘E opens it in your editor. "
                    + "The panes are shipping one at a time (#1156)."
            )
        }
    }

    private var footer: some View {
        HStack(spacing: 8) {
            Image(systemName: store.status.valid ? "checkmark.circle" : "exclamationmark.circle")
                .foregroundStyle(store.status.valid ? Color.secondary : Color.red)
            Text(store.footer)
                .font(.system(.footnote, design: .monospaced))
                .foregroundStyle(store.status.valid ? Color.secondary : Color.red)
            Spacer()
            Text(store.path)
                .font(.footnote)
                .foregroundStyle(.tertiary)
                .truncationMode(.head)
                .lineLimit(1)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    /// A binding that patches the file on change.
    ///
    /// There is no Save in this window because canvas 3f decided there is no
    /// second store to save *from* — which is exactly what the footer says.
    private func binding<T>(
        _ current: AppearanceFfi,
        _ field: WritableKeyPath<AppearanceFfi, T>
    ) -> Binding<T> {
        Binding(
            get: { current[keyPath: field] },
            // One field, applied to whatever the file says at the moment of
            // the click -- never to the copy this view was drawn from.
            set: { value in store.apply { $0[keyPath: field] = value } }
        )
    }
}
