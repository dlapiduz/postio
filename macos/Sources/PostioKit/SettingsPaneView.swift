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

    /// Every folder, so an account row can say how much mail it has without
    /// asking the store to count it again.
    private let mailboxes: [MailboxFfi]

    /// The session, for the things the accounts pane can actually *do* —
    /// adding one, mostly. `nil` when the store never opened, in which case
    /// the pane still draws: settings are a file, and being unable to read
    /// mail is not being unable to configure it.
    private let session: PostioSession?

    /// Which account's form is open, if any.
    @State private var selected: Int64?
    /// The add-account sheet, while it is up.
    @State private var adding: AddAccountModel?
    /// The account being signed in again, if one is.
    @State private var reconnecting: Int64?
    /// Test, re-index and remove, and what the last one said.
    ///
    /// Owned by the application rather than by this view: a re-index reports
    /// through the event stream, and a window that owned it would have to be
    /// open at the moment each report landed.
    private let actions: AccountActions
    /// What the display-name field currently holds.
    @State private var renamedTo = ""
    /// The remote-image grants, read when the Privacy pane appears.
    @State private var grants: [GrantFfi] = []
    /// The new filter being typed, if one is.
    @State private var newFilterKey = ""
    @State private var newFilterQuery = ""

    public init(
        store: SettingsStore,
        accounts: [AccountFfi] = [],
        mailboxes: [MailboxFfi] = [],
        actions: AccountActions = AccountActions(),
        session: PostioSession? = nil
    ) {
        self.store = store
        self.accounts = accounts
        self.mailboxes = mailboxes
        self.actions = actions
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
        case "compose": composing
        case "accounts": accountsPane
        case "sync": syncing
        case "keys": keyboard
        case "privacy": privacy
        case "filters": filters
        case "": configFile
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
                    if let account = accounts.first(where: { $0.id == selected }) {
                        actions.askToRemove(account)
                    }
                } label: {
                    Image(systemName: "minus")
                }
                .disabled(selected == nil || actions.isBusy)
                .help("Remove the selected account")
                .accessibilityLabel("Remove the selected account")
                Spacer()
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
        // Removing takes the account's mail with it, so it is asked about
        // and the question names what goes.
        .alert(
            "Remove this account?",
            isPresented: Binding(
                get: { actions.confirmingRemoval != nil },
                set: { if !$0 { actions.cancelRemoval() } }
            ),
            presenting: actions.confirmingRemoval
        ) { _ in
            Button("Remove", role: .destructive) {
                Task { await actions.confirmRemoval(through: session) }
            }
            Button("Cancel", role: .cancel) { actions.cancelRemoval() }
        } message: { account in
            Text(actions.removalWarning(for: account))
        }
        .sheet(item: $adding) { model in
            AddAccountSheet(session: session, model: model) { added in
                // Only when the sheet actually wrote one: cancelling must not
                // announce an account that is not there. Without this the row
                // exists and nothing syncs it until the next launch, which
                // reads as an account that did not save (#1299).
                if added { actions.added() }
                adding = nil
                reconnecting = nil
            }
        }
    }

    /// What the re-index button says, including how far it has got.
    private var reindexTitle: String {
        guard actions.running == .reindexing else { return "Re-index store" }
        guard let progress = actions.progressLabel else { return "Re-indexing…" }
        return "Re-indexing… \(progress)"
    }

    /// Sign in to `account` again, in the browser.
    ///
    /// The sheet, pre-filled and opened at the step that asks: an expired
    /// token needs consent, and consent is the one thing this application
    /// never collects itself. Reusing the sheet rather than growing a second
    /// path is the point — there is one place a browser sign-in happens.
    private func reconnect(_ account: AccountFfi) {
        let model = AddAccountModel()
        model.address = account.address
        model.next()
        reconnecting = account.id
        adding = model
    }

    /// One account, and its form when it is the selected one.
    @ViewBuilder private func accountRow(_ account: AccountFfi) -> some View {
        let open = selected == account.id
        VStack(alignment: .leading, spacing: 0) {
            Button {
                selected = open ? nil : account.id
                renamedTo = account.displayName
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
                            Text(
                                AccountRow.line(
                                    account,
                                    mailboxes: mailboxes,
                                    weight: session?.weight(of: account.id)
                                )
                            )
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                        }
                    }
                    Spacer(minLength: 0)
                    if AccountRow.needsAttention(account) {
                        // Inline, beside the account it is about: a token
                        // that expired is a thing to fix here rather than a
                        // banner somewhere else. It is the *same* sign-in the
                        // sheet runs — the account is already configured, so
                        // what it needs is consent again, not another row.
                        Button(reconnecting == account.id ? "Signing in…" : "Reconnect") {
                            reconnect(account)
                        }
                        .controlSize(.small)
                        .disabled(reconnecting != nil)
                        .help("Sign in to this account again in your browser")
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
                HStack(spacing: 8) {
                    TextField("", text: $renamedTo)
                        .textFieldStyle(.roundedBorder)
                        .frame(maxWidth: 260)
                        .onSubmit { actions.rename(account, to: renamedTo, through: session) }
                        .accessibilityLabel("Display name")
                    Button("Save") { actions.rename(account, to: renamedTo, through: session) }
                        .disabled(renamedTo == account.displayName || actions.isBusy)
                }
            }
            field("LOCAL STORE") {
                Text(store.path)
                    .font(.system(.callout, design: .monospaced))
                    .textSelection(.enabled)
            }
            HStack(spacing: 8) {
                Button(actions.running == .testing ? "Testing…" : "Test connection") {
                    Task { await actions.test(account, through: session) }
                }
                Button(reindexTitle) {
                    Task { await actions.reindex(account, through: session) }
                }
                Button("Remove account…") { actions.askToRemove(account) }
            }
            .disabled(actions.isBusy)
            if actions.missingCredential {
                // The Partial state: a row that exists with nothing in the
                // keyring to sign in with. It asks for a password rather
                // than a retry, which is a different offer from "the server
                // said no".
                Label(
                    "This account has no password in your Keychain yet.",
                    systemImage: "key.slash"
                )
                .font(.callout)
            }
            if let outcome = actions.outcome {
                // What happened, where it was asked for. A connection test
                // whose answer appears somewhere else is a test nobody reads.
                Label(outcome, systemImage: actions.failed ? "xmark.circle" : "checkmark.circle")
                    .font(.callout)
                    .foregroundStyle(actions.failed ? Color.primary : Color.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
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

    // -- Filters (#1156) ----------------------------------------------------

    /// The saved searches, and what each one runs.
    ///
    /// The key is the identity (#292) and is deliberately not editable here:
    /// renaming writes `name`, so a filter cannot be orphaned by being moved
    /// to a new key. What the row shows is the name; what it edits is the
    /// query, the label, and whether the sidebar carries it.
    @ViewBuilder private var filters: some View {
        if let current = store.filters {
            VStack(alignment: .leading, spacing: 0) {
                if current.isEmpty {
                    ContentUnavailableView {
                        Label("No filters", systemImage: "line.3.horizontal.decrease.circle")
                    } description: {
                        Text(
                            "A filter is a saved search. Add one below, or write "
                                + "`[filters.<name>]` in the config file."
                        )
                    }
                    .frame(maxHeight: .infinity)
                } else {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(current, id: \.key) { filter in
                                filterRow(filter)
                                if filter.key != current.last?.key { Divider() }
                            }
                        }
                    }
                }
                Divider()
                HStack(spacing: PostioTokens.space2) {
                    TextField("Name", text: $newFilterKey)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 140)
                    TextField("is:unread", text: $newFilterQuery)
                        .textFieldStyle(.roundedBorder)
                    Button("Add") {
                        store.addFilter(key: newFilterKey, query: newFilterQuery)
                        if store.failure == nil {
                            newFilterKey = ""
                            newFilterQuery = ""
                        }
                    }
                    .disabled(
                        newFilterKey.trimmingCharacters(in: .whitespaces).isEmpty
                            || newFilterQuery.trimmingCharacters(in: .whitespaces).isEmpty
                    )
                }
                .padding(.vertical, PostioTokens.space2)
            }
        } else {
            unreadable
        }
    }

    @ViewBuilder private func filterRow(_ filter: FilterFfi) -> some View {
        VStack(alignment: .leading, spacing: PostioTokens.space2) {
            HStack {
                TextField(
                    filter.key,
                    text: Binding(
                        get: { filter.name },
                        set: { value in
                            var next = filter
                            next.name = value
                            store.applyFilter(next)
                        })
                )
                .textFieldStyle(.plain)
                .font(.body.weight(.medium))
                Spacer()
                Toggle(
                    "In the sidebar",
                    isOn: Binding(
                        get: { filter.pinned },
                        set: { value in
                            var next = filter
                            next.pinned = value
                            store.applyFilter(next)
                        })
                )
                .toggleStyle(.checkbox)
                Button("Remove") { store.removeFilter(filter.key) }
                    .buttonStyle(.link)
            }
            TextField(
                "is:unread",
                text: Binding(
                    get: { filter.query },
                    set: { value in
                        var next = filter
                        next.query = value
                        store.applyFilter(next)
                    })
            )
            .textFieldStyle(.roundedBorder)
            .font(.system(.callout, design: .monospaced))
        }
        .padding(.vertical, PostioTokens.space3)
    }

    // -- Privacy (#1156) ----------------------------------------------------

    /// What Postio will not do without being asked, and what it has been
    /// asked.
    ///
    /// The promises are stated rather than configurable, because they are not
    /// settings: remote images blocked, read receipts never sent
    /// automatically, no telemetry, JavaScript and network off in the reader.
    /// A switch for any of them would be a switch for turning the product's
    /// own claims off.
    ///
    /// What *is* here is the list of grants, because a permission the user
    /// cannot see is one they cannot withdraw.
    @ViewBuilder private var privacy: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space6) {
            field("POSTIO WILL NOT") {
                VStack(alignment: .leading, spacing: PostioTokens.space2) {
                    ForEach(Self.promises, id: \.self) { promise in
                        Label(promise, systemImage: "checkmark.shield")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            Divider()
            field("REMOTE IMAGES ALLOWED FROM") {
                if grants.isEmpty {
                    Text("Nothing yet. Images stay blocked until you allow a sender.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                } else {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(grants, id: \.subject) { grant in
                            HStack {
                                Text(grant.subject)
                                if grant.wholeDomain {
                                    // Two very different amounts of trust; a
                                    // list that drew them alike would
                                    // understate one of them.
                                    Text("everyone at this domain")
                                        .font(.footnote)
                                        .foregroundStyle(.secondary)
                                }
                                Spacer(minLength: PostioTokens.space4)
                                Button("Revoke") {
                                    session?.revokeRemoteImages(grant.subject)
                                    grants = session?.remoteImageGrants() ?? []
                                }
                                .buttonStyle(.link)
                            }
                            .padding(.vertical, 4)
                            .accessibilityElement(children: .combine)
                            if grant.subject != grants.last?.subject { Divider() }
                        }
                    }
                }
            }
            Spacer(minLength: 0)
        }
        .onAppear { grants = session?.remoteImageGrants() ?? [] }
    }

    /// The promises, as `docs/PRODUCT.md` states them.
    private static let promises = [
        "Load remote images until you allow the sender",
        "Send a read receipt automatically",
        "Run JavaScript or reach the network in the reading pane",
        "Fetch anything speculatively, or send any telemetry",
    ]

    // -- Keyboard (#1156) ---------------------------------------------------

    /// Every command and the key in force for it, grouped as the menus are.
    ///
    /// Read-only for now, and honestly so: rebinding is `[keys]`, the pane
    /// says where, and the Config file pane is one click away. A picker that
    /// *looked* editable and wrote nothing would be worse than a table that
    /// admits what it is — canvas 3d's rule that a state names its reason.
    ///
    /// The bindings come from the session rather than from `defaultBinding`,
    /// so a rebound key shows the key the user actually has. With no session
    /// the defaults stand in, because settings are a file and being unable to
    /// read mail is not being unable to read your own keyboard.
    @ViewBuilder private var keyboard: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: PostioTokens.space6) {
                ForEach(MenuPlan.build(bindings: bindingsInForce), id: \.title) { menu in
                    VStack(alignment: .leading, spacing: PostioTokens.space2) {
                        Text(menu.title.uppercased())
                            .font(.system(size: 10, weight: .semibold))
                            .kerning(0.6)
                            .foregroundStyle(.secondary)
                        ForEach(menu.items, id: \.command) { item in
                            HStack(alignment: .firstTextBaseline) {
                                Text(item.title)
                                Spacer(minLength: PostioTokens.space4)
                                Text(item.shortcut ?? "—")
                                    .font(.system(.callout, design: .monospaced))
                                    .foregroundStyle(item.shortcut == nil ? .tertiary : .secondary)
                            }
                            .accessibilityElement(children: .combine)
                        }
                    }
                }
            }
            .padding(.trailing, PostioTokens.space4)
        }
    }

    /// What each command is bound to now.
    private func bindingsInForce(_ command: String) -> [String] {
        session?.bindings(for: command) ?? []
    }

    // -- Sync & storage (#1156) ---------------------------------------------

    @ViewBuilder private var syncing: some View {
        if let current = store.syncing {
            VStack(alignment: .leading, spacing: 20) {
                HStack(alignment: .top, spacing: 32) {
                    field("CHECK FOR MAIL") {
                        VStack(alignment: .leading, spacing: PostioTokens.space2) {
                            Picker("", selection: syncBinding(current, \.checkForMail)) {
                                Text("Push").tag(CheckForMailFfi.idle)
                                Text("Poll").tag(CheckForMailFfi.poll)
                                Text("Manual").tag(CheckForMailFfi.manual)
                            }
                            .pickerStyle(.segmented)
                            .labelsHidden()
                            .fixedSize()
                            // What the choice costs, in the unit being chosen
                            // between — the sentence is the boundary's, so
                            // GTK says the same one.
                            Text(
                                current.checkForMail == .poll
                                    ? "Every \(settingsHumanizeInterval(seconds: current.pollIntervalSecs))"
                                    : "Folders without push still poll every "
                                        + settingsHumanizeInterval(seconds: current.pollIntervalSecs)
                            )
                            .font(.system(.footnote, design: .monospaced))
                            .foregroundStyle(.secondary)
                        }
                    }
                    field("POLL EVERY") {
                        Picker("", selection: syncBinding(current, \.pollIntervalSecs)) {
                            Text("1 min").tag(UInt64(60))
                            Text("5 min").tag(UInt64(300))
                            Text("15 min").tag(UInt64(900))
                            Text("1 hour").tag(UInt64(3600))
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                    }
                    Spacer(minLength: 0)
                }
                Divider()
                HStack(alignment: .top, spacing: 32) {
                    field("DOWNLOAD BODIES") {
                        Picker("", selection: syncBinding(current, \.bodyFetch)) {
                            Text("When opened").tag(BodyFetchFfi.lazy)
                            Text("Ahead of time").tag(BodyFetchFfi.eager)
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                    }
                    field("DOWNLOAD ATTACHMENTS") {
                        Picker("", selection: syncBinding(current, \.attachmentFetch)) {
                            Text("When opened").tag(AttachmentFetchFfi.onOpen)
                            Text("Ahead of time").tag(AttachmentFetchFfi.eager)
                            Text("Never").tag(AttachmentFetchFfi.never)
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                    }
                    Spacer(minLength: 0)
                }
                Divider()
                field("ON STARTUP") {
                    VStack(alignment: .leading, spacing: 8) {
                        Toggle("Sync as soon as Postio opens", isOn: syncBinding(current, \.syncOnStartup))
                        Toggle("Notify me about new mail", isOn: syncBinding(current, \.notify))
                    }
                    .toggleStyle(.checkbox)
                }
                Spacer(minLength: 0)
            }
        } else {
            unreadable
        }
    }

    private func syncBinding<T>(
        _ current: SyncingFfi,
        _ field: WritableKeyPath<SyncingFfi, T>
    ) -> Binding<T> {
        Binding(
            get: { current[keyPath: field] },
            set: { value in store.applySyncing { $0[keyPath: field] = value } }
        )
    }

    // -- Config file: the whole thing, as text (#1156) ----------------------

    /// The raw editor, and the escape hatch every unbuilt pane depends on.
    ///
    /// Canvas 3f: *"Settings is still the config file — no second store, no
    /// OK/Cancel"*. So there is no Save here either. What there is instead is
    /// the footer, which already says whether the file parses and where it
    /// stopped — the only thing that could tell you a keystroke went wrong.
    ///
    /// Monospaced and not proportional, because it is TOML and alignment
    /// carries meaning in it. `autocorrection` and the smart-quote
    /// substitutions are off for the reason every code editor turns them
    /// off: a curly quote in a TOML string is a parse error somebody did not
    /// type.
    @ViewBuilder private var configFile: some View {
        TextEditor(text: configBinding)
            .font(.system(.body, design: .monospaced))
            .autocorrectionDisabled()
            .textEditorStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(Color(nsColor: .textBackgroundColor))
            .overlay(
                RoundedRectangle(cornerRadius: PostioTokens.radiusMd)
                    .stroke(Color.secondary.opacity(0.25), lineWidth: 1)
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .accessibilityLabel("The configuration file")
    }

    /// Writes on every keystroke, like every other control in this window.
    ///
    /// Invalid TOML is written too: this is a text editor over a file
    /// somebody is part-way through fixing, and refusing to save until it
    /// parses would make it useless for its one job. The footer says what is
    /// wrong, and a running Postio keeps the last good configuration.
    private var configBinding: Binding<String> {
        Binding(
            get: { store.text },
            set: { store.write(text: $0) }
        )
    }

    // -- Composing (#1288) --------------------------------------------------

    @ViewBuilder private var composing: some View {
        if let current = store.composing {
            VStack(alignment: .leading, spacing: 20) {
                HStack(alignment: .top, spacing: 32) {
                    field("SIGNATURE ON A REPLY") {
                        Picker("", selection: composeBinding(current, \.signatureOnReply)) {
                            Text("Above the quote").tag(SignaturePlacementFfi.aboveQuote)
                            Text("Below it").tag(SignaturePlacementFfi.belowQuote)
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                    }
                    field("SIGNATURE ON A FORWARD") {
                        Picker("", selection: composeBinding(current, \.signatureOnForward)) {
                            Text("Above the quote").tag(SignaturePlacementFfi.aboveQuote)
                            Text("Below it").tag(SignaturePlacementFfi.belowQuote)
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                    }
                    Spacer(minLength: 0)
                }
                Divider()
                field("EDIT ELSEWHERE") {
                    VStack(alignment: .leading, spacing: 8) {
                        // A text field rather than a picker: the list of
                        // applications that can edit text is open, which is
                        // exactly the case ADR 0029 reserves a free field for.
                        TextField("", text: editorBinding(current))
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 260)
                        Text(editorAdvice(current.editor))
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 0)
            }
        } else {
            unreadable
        }
    }

    /// What to say under the editor field: which application will open the
    /// draft, or why the one named cannot.
    ///
    /// The judgement is the boundary's, not this view's — `postio_ui::handoff`
    /// decides, so GTK says the same thing about the same name.
    private func editorAdvice(_ configured: String) -> String {
        switch settingsHandoffTarget(
            configured: configured,
            isApplication: ComposeHandoff.isApplication(configured)
        ) {
        case .platformDefault:
            "Empty: the draft opens in whatever this Mac opens a text file with."
        case .application(let name):
            "Drafts open in \(name)."
        case .needsTerminal(_, let advice):
            advice
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

    /// For a section this build does not know.
    ///
    /// All eight of canvas 3f's panes are drawn now, so nothing reaches this
    /// from the shipped nav. It stays because the nav is `postio_ui`'s and a
    /// newer core can name a section this build has never heard of — which
    /// should say so rather than draw an empty pane.
    private var unbuilt: some View {
        ContentUnavailableView {
            Label("Not in this build", systemImage: "gearshape")
        } description: {
            Text(
                "This section is newer than this build of Postio. It is still "
                    + "editable in the file — the Config file pane has all of it."
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
            Text(PostioPath.abbreviated(store.path))
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

    /// The same binding, over the `[compose]` table.
    private func composeBinding<T>(
        _ current: ComposingFfi,
        _ field: WritableKeyPath<ComposingFfi, T>
    ) -> Binding<T> {
        Binding(
            get: { current[keyPath: field] },
            set: { value in store.applyComposing { $0[keyPath: field] = value } }
        )
    }

    /// The editor field, which writes on every keystroke like every other
    /// control here — there is no Save in this window (canvas 3f).
    private func editorBinding(_ current: ComposingFfi) -> Binding<String> {
        Binding(
            get: { current.editor },
            set: { value in store.applyComposing { $0.editor = value } }
        )
    }
}
