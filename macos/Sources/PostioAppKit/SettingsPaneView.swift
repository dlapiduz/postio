import PostioFFI
import SwiftUI
import PostioKit

/// Sections, order, headings and labels come from `postio_ui::settings`, so the two frontends
/// cannot drift.
/// Every Picker is `.segmented` explicitly: SwiftUI defaults to a dropdown on macOS, and ADR 0029
/// wants segmented for closed sets.
public struct SettingsPaneView: View {
    @Bindable private var store: SettingsStore
    private let accounts: [AccountFfi]

    private let mailboxes: [MailboxFfi]

    /// `nil` when the store never opened; the pane still draws, since settings are a file.
    private let session: PostioSession?

    private let contacts: ContactsSettingsModel?
    private let section: SettingsSectionRequest

    /// Held outside this view: the seven verbs in `Context::Accounts` act on it, and a command
    /// cannot reach an `@State` (`SettingsAccounts`).
    private let accountCursor: SettingsAccounts
    private let repair: AccountRepair
    /// A pane that edited its own copy would draw what it believes rather than what was written.
    private let reloadAccounts: (() -> Void)?
    private let run: ((String) -> Void)?
    @State private var adding: AddAccountModel?
    /// Owned by the application: a re-index reports through the event stream, and a window that
    /// owned it would have to be open when each report landed.
    private let actions: AccountActions
    @State private var renamedTo = ""
    @State private var grants: [GrantFfi] = []
    @State private var activations: [UnsubscribeActivationFfi] = []
    @State private var agentActivity: AgentActivityFfi?
    @State private var newFilterKey = ""
    @State private var newFilterQuery = ""

    public init(
        store: SettingsStore,
        accounts: [AccountFfi] = [],
        mailboxes: [MailboxFfi] = [],
        actions: AccountActions = AccountActions(),
        accountCursor: SettingsAccounts = SettingsAccounts(),
        repair: AccountRepair = AccountRepair(),
        reloadAccounts: (() -> Void)? = nil,
        session: PostioSession? = nil,
        contacts: ContactsSettingsModel? = nil,
        section: SettingsSectionRequest = SettingsSectionRequest(),
        run: ((String) -> Void)? = nil
    ) {
        self.store = store
        self.accounts = accounts
        self.mailboxes = mailboxes
        self.actions = actions
        self.accountCursor = accountCursor
        self.repair = repair
        self.reloadAccounts = reloadAccounts
        self.session = session
        self.contacts = contacts
        self.section = section
        self.run = run
    }

    public var body: some View {
        content
            // By token, not value: two accounts added in a row are two sheets, and `onChange` on
            // the wish alone would open only the first.
            .onChange(of: accountCursor.wishToken) { _, _ in grant(accountCursor.take()) }
            .onAppear {
                grant(accountCursor.take())
                if let key = section.take() { store.selected = key }
            }
            .onChange(of: section.token) { _, _ in
                if let key = section.take() { store.selected = key }
            }
            // Never pre-filled: the old password is what stopped working.
            .alert(
                "New password",
                isPresented: Binding(
                    get: { repair.asking != nil },
                    set: { if !$0 { repair.cancel() } }
                )
            ) {
                SecureField("Password", text: Binding(
                    get: { repair.typed }, set: { repair.typed = $0 }
                ))
                Button("Save") {
                    guard let id = repair.asking,
                          let account = accounts.first(where: { $0.id == id })
                    else { return }
                    Task { await repair.save(account, through: session) }
                }
                Button("Cancel", role: .cancel) { repair.cancel() }
            } message: {
                Text(
                    """
                    It goes straight into your Keychain and is never written \
                    to a file. If your provider calls this an app password, \
                    that is the one it wants.
                    """
                )
            }
    }

    private var content: some View {
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
        // Fixed rather than resizable: a nav dragged to nothing is a bug report waiting.
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
            // The whole row is the target: a plain button hit-tests only what it draws, and an
            // unselected row draws a clear background.
            .contentShape(RoundedRectangle(cornerRadius: 6))
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 8)
        .accessibilityAddTraits(selected ? [.isSelected] : [])
    }

    /// A symbolic icon name is not an SF Symbol, and the shared crate should carry neither.
    private func symbol(for key: String) -> String {
        switch key {
        case "accounts": return "person.crop.circle"
        case "saved_searches": return "line.3.horizontal.decrease.circle"
        case "compose": return "square.and.pencil"
        case "focus": return "tray.and.arrow.down"
        case "keys": return "keyboard"
        case "sync": return "arrow.triangle.2.circlepath"
        case "privacy": return "lock.shield"
        case "mcp": return "terminal"
        case "contacts": return "person.2"
        default: return "doc.plaintext"
        }
    }

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

    @ViewBuilder private var contactsPane: some View {
        if let contacts {
            ContactsSettingsView(model: contacts)
        } else {
            unbuilt
        }
    }

    @ViewBuilder private var pane: some View {
        switch store.selected {
        case "focus": filteringPane
        case "compose": composing
        case "accounts": accountsPane
        case "sync": syncing
        case "keys": keyboard
        case "privacy": privacy
        case "mcp": agents
        case "contacts": contactsPane
        case "saved_searches": filters
        case "": configFile
        default: unbuilt
        }
    }

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
            HStack(spacing: 6) {
                Button {
                    adding = AddAccountModel()
                } label: {
                    Image(systemName: "plus")
                }
                .help("Add an account")
                .accessibilityLabel("Add an account")
                Button {
                    if let account = accountCursor.focused(in: accounts) {
                        actions.askToRemove(account)
                    }
                } label: {
                    Image(systemName: "minus")
                }
                .disabled(accountCursor.cursor == nil || actions.isBusy)
                .help("Remove the selected account")
                .accessibilityLabel("Remove the selected account")
                Spacer()
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
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
                // Only when the sheet wrote an account: cancelling must not announce one, and
                // skipping this on a real add leaves the row unsynced until relaunch (#1299).
                if added { actions.added() }
                adding = nil
            }
        }
    }

    private var reindexTitle: String {
        guard actions.running == .reindexing else { return "Re-index store" }
        guard let progress = actions.progressLabel else { return "Re-indexing…" }
        return "Re-indexing… \(progress)"
    }

    /// Says what a settings verb refused beside the control, where the person who pressed it is
    /// looking.
    private func complain(_ said: String?) {
        guard let said else { return }
        actions.said(said, failed: true)
    }

    private func grant(_ wish: SettingsAccounts.Wish?) {
        // Both wishes are about accounts, so that is the pane to be on.
        if wish != nil { store.selected = "accounts" }
        switch wish {
        case .add:
            adding = AddAccountModel()
        case let .updateCredential(id):
            // The row's own button takes the same route (`AccountRepair`), so a command and a
            // button cannot repair an account two ways.
            guard let account = accounts.first(where: { $0.id == id }) else { return }
            Task { await repair.begin(account, through: session) }
        case nil:
            break
        }
    }

    @ViewBuilder private func accountRow(_ account: AccountFfi) -> some View {
        let open = accountCursor.cursor == account.id
        VStack(alignment: .leading, spacing: 0) {
            Button {
                accountCursor.put(cursor: open ? nil : account.id)
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
                    // Inline, beside the account. Which repair is the boundary's answer, not a
                    // guess from the provider's name: a password where consent is wanted, or a
                    // browser for an app password, sends the person somewhere with no field to use.
                    if let offer = repairOffer(account), let label = offer.button {
                        Button(label) {
                            Task { await repair.begin(account, through: session) }
                        }
                        .controlSize(.small)
                        .disabled(repair.running != nil)
                        .help(
                            account.repair == .password
                                ? "Store a new password for this account"
                                : "Sign in to this account again in your browser"
                        )
                        .accessibilityHint(offer.reason)
                    }
                }
                .padding(.vertical, 10)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(open ? [.isSelected] : [])

            if open {
                accountForm(account)
            }
        }
    }

    private func repairOffer(_ account: AccountFfi) -> AccountRepair.Offer? {
        AccountRepair.offer(
            for: account,
            found: actions.trouble(for: account.id),
            running: repair.running == account.id
        )
    }

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
            field("SYNCING") {
                Toggle(
                    "Check this account for new mail",
                    isOn: Binding(
                        get: { account.enabled },
                        set: { wanted in
                            complain(session?.setAccountEnabled(account.id, wanted))
                            reloadAccounts?()
                        }
                    )
                )
                .toggleStyle(.switch)
                .disabled(actions.isBusy)
            }
            field("DEFAULT ACCOUNT") {
                // "Default", never "primary" or "main" (#960): they imply this account is more the
                // user's than another. It cannot be cleared; marking another reverses it.
                if account.isDefault {
                    Text("New messages come from this account.")
                        .foregroundStyle(.secondary)
                } else {
                    Button("Make this the default") {
                        complain(session?.setDefaultAccount(account.id))
                        reloadAccounts?()
                    }
                    .disabled(actions.isBusy)
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
            if let offer = repairOffer(account) {
                Label(offer.reason, systemImage: "key.slash")
                    .font(.callout)
            }
            if let outcome = actions.outcome {
                Label(outcome, systemImage: actions.failed ? "xmark.circle" : "checkmark.circle")
                    .font(.callout)
                    .foregroundStyle(actions.failed ? Color.primary : Color.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let outcome = repair.outcome {
                // A repair that reported nothing is pressed again: the row's warning does not clear
                // until the next sync proves the credential.
                Label(outcome, systemImage: repair.failed ? "xmark.circle" : "checkmark.circle")
                    .font(.callout)
                    .foregroundStyle(repair.failed ? Color.primary : Color.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.leading, 42)
        .padding(.bottom, 14)
    }

    /// Every word is `postio_ui::filtering`'s through `settings_filtering`; the switch and
    /// take-backs write through `focus_edit`, so the rest of the file stays as it was.
    @ViewBuilder private var filteringPane: some View {
        if let page = store.filtering {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    VStack(alignment: .leading, spacing: PostioTokens.space1) {
                        Toggle(
                            page.switch,
                            isOn: Binding(
                                get: { page.on },
                                set: { store.applyFiltering(on: $0) }
                            )
                        )
                        .toggleStyle(.switch)
                        Text(page.state)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Divider()
                    field(page.filtered.uppercased()) {
                        VStack(alignment: .leading, spacing: PostioTokens.space2) {
                            HStack(spacing: PostioTokens.space3) {
                                if let today = page.today {
                                    Text(today).font(.body.weight(.medium))
                                }
                                Button {
                                    run?("go_to_filtered")
                                } label: {
                                    HStack(spacing: 6) {
                                        Text(page.open)
                                        if let cap = KeyCapSpelling.cap(page.openKey) {
                                            KeyCap(cap)
                                        }
                                    }
                                }
                                .disabled(run == nil)
                            }
                            Text(page.kept)
                                .font(.callout)
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                            if !page.keys.isEmpty {
                                HStack(spacing: PostioTokens.space3) {
                                    ForEach(page.keys, id: \.label) { hint in
                                        HStack(spacing: 4) {
                                            KeyCap(KeyCapSpelling.cap(hint.key) ?? hint.key)
                                            Text(hint.label)
                                                .font(.callout)
                                                .foregroundStyle(.secondary)
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Divider()
                    field(page.neverHeading.uppercased()) {
                        VStack(alignment: .leading, spacing: PostioTokens.space2) {
                            Text(page.guards)
                                .font(.callout)
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                            filteringList(page.never, empty: page.neverEmpty, name: page.neverHeading)
                        }
                    }
                    Divider()
                    field(page.stoppedHeading.uppercased()) {
                        filteringList(page.stopped, empty: page.stoppedEmpty, name: page.stoppedHeading)
                    }
                    if let control = store.afterUnsubscribe {
                        Divider()
                        afterUnsubscribeControl(control)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        } else {
            unreadable
        }
    }

    @ViewBuilder private func afterUnsubscribeControl(_ control: AfterUnsubscribeFfi) -> some View {
        field(control.heading.uppercased()) {
            VStack(alignment: .leading, spacing: PostioTokens.space2) {
                Picker(
                    control.heading,
                    selection: Binding(
                        get: { control.delete },
                        set: { store.applyAfterUnsubscribe(delete: $0) }
                    )
                ) {
                    ForEach(Array(control.choices.enumerated()), id: \.offset) { index, label in
                        Text(label).tag(index == 1)
                    }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .equalSegments(control.choices.count, each: 76)
                Text(control.note)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    @ViewBuilder private func filteringList(
        _ entries: [FilteringEntryFfi], empty: String, name: String
    ) -> some View {
        if entries.isEmpty {
            Text(empty).font(.callout).foregroundStyle(.tertiary)
        } else {
            VStack(alignment: .leading, spacing: PostioTokens.space1) {
                ForEach(Array(entries.enumerated()), id: \.offset) { _, entry in
                    HStack(spacing: PostioTokens.space3) {
                        Text(entry.says)
                            .foregroundStyle(entry.acts ? Color.primary : Color.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .fixedSize(horizontal: false, vertical: true)
                        Button(entry.undoLabel) { store.takeBack(entry.undo) }
                            .buttonStyle(.link)
                    }
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(name)
        }
    }

    /// The key is the identity (#292) and not editable here: renaming writes `name`, so a filter
    /// cannot be orphaned by moving to a new key.
    @ViewBuilder private var filters: some View {
        if let current = store.filters {
            VStack(alignment: .leading, spacing: 0) {
                if current.isEmpty {
                    ContentUnavailableView {
                        Label("No saved searches", systemImage: "line.3.horizontal.decrease.circle")
                    } description: {
                        // One literal so the table name is set as code: a concatenation is a plain
                        // String and showed its backticks.
                        Text("Add one below, or write `[saved_searches.<name>]` in the config file.")
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

    /// The promises are stated, not configurable: a switch for any of them would turn the product's
    /// own claims off.
    /// The grants are listed because a permission the user cannot see is one they cannot withdraw.
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
            Divider()
            field("UNSUBSCRIBED FROM") {
                // No Revoke beside these: the request has left. This is a record of what Postio did
                // on your behalf, so "only on deliberate activation" is reviewable.
                if activations.isEmpty {
                    Text("Nothing yet. Unsubscribing only ever happens when you ask for it.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                } else {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(activations, id: \.label) { activation in
                            HStack {
                                Text(activation.listIdentifier)
                                Spacer(minLength: PostioTokens.space4)
                                Text(activation.when)
                                    .font(.system(.footnote, design: .monospaced))
                                    .foregroundStyle(.secondary)
                            }
                            .padding(.vertical, 4)
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel(activation.label)
                            if activation.label != activations.last?.label { Divider() }
                        }
                    }
                }
            }
            Spacer(minLength: 0)
        }
        .onAppear {
            grants = session?.remoteImageGrants() ?? []
            activations = session?.unsubscribeActivations() ?? []
        }
    }

    private static let promises = [
        "Load remote images until you allow the sender",
        "Send a read receipt automatically",
        "Run JavaScript or reach the network in the reading pane",
        "Fetch anything speculatively, or send any telemetry",
    ]

    /// Every word is `postio_ui::agent_access`'s and every edit goes through the boundary's
    /// patchers. Revoking removes the account's `[mcp]` table only, so it bites on the agent's next
    /// call; the note says a running call finishes.
    @ViewBuilder private var agents: some View {
        let words = settingsAgentWords()
        ScrollView {
            VStack(alignment: .leading, spacing: PostioTokens.space6) {
                if let rows = store.agentSharing {
                    field(words.sharedHeading.uppercased()) {
                        if rows.isEmpty {
                            Text(words.nothingShared)
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        } else {
                            VStack(alignment: .leading, spacing: 0) {
                                ForEach(rows, id: \.address) { row in
                                    HStack(alignment: .firstTextBaseline) {
                                        VStack(alignment: .leading, spacing: 2) {
                                            Text(row.address)
                                            Text("\(row.folders) \u{00B7} \(row.abilities)")
                                                .font(.footnote)
                                                .foregroundStyle(.secondary)
                                        }
                                        Spacer(minLength: PostioTokens.space4)
                                        Button(words.revokeOne) {
                                            store.revokeAgent(row.address)
                                            refreshAgentActivity()
                                        }
                                        .buttonStyle(.link)
                                        .accessibilityLabel("\(words.revokeOne) \(row.address)")
                                    }
                                    .padding(.vertical, 4)
                                    if row.address != rows.last?.address { Divider() }
                                }
                            }
                            HStack {
                                Text(words.revokeNote)
                                    .font(.footnote)
                                    .foregroundStyle(.secondary)
                                Spacer(minLength: PostioTokens.space4)
                                Button(words.revokeAll) { store.revokeAllAgents() }
                                    .buttonStyle(.link)
                            }
                        }
                    }
                    if let entry = AgentEntryPoint.path(executable: Bundle.main.executableURL) {
                        Divider()
                        field(words.entryHeading.uppercased()) {
                            Text(entry)
                                .font(.system(.callout, design: .monospaced))
                                .textSelection(.enabled)
                        }
                    }
                    Divider()
                    field(words.activityHeading.uppercased()) {
                        if let activity = agentActivity {
                            Text(activity.summary)
                                .font(.callout)
                                .foregroundStyle(.secondary)
                            VStack(alignment: .leading, spacing: 0) {
                                ForEach(Array(activity.calls.enumerated()), id: \.offset) { _, call in
                                    HStack(alignment: .firstTextBaseline) {
                                        Text(call.line)
                                            .foregroundStyle(call.refused ? .secondary : .primary)
                                        Spacer(minLength: PostioTokens.space4)
                                        Text(call.when)
                                            .font(.system(.footnote, design: .monospaced))
                                            .foregroundStyle(.secondary)
                                    }
                                    .padding(.vertical, 4)
                                    .accessibilityElement(children: .combine)
                                    Divider()
                                }
                            }
                        }
                    }
                } else {
                    unreadable
                }
            }
        }
        .onAppear { refreshAgentActivity() }
    }

    private func refreshAgentActivity() {
        guard let session else { return }
        Task {
            let activity = await Task.detached { session.agentActivity() }.value
            agentActivity = activity
        }
    }

    /// Read-only: rebinding is `[keys]` and the pane says where; a picker that looked editable and
    /// wrote nothing would be worse.
    /// Bindings come from the session so a rebound key shows; with no session the defaults stand
    /// in.
    @ViewBuilder private var keyboard: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: PostioTokens.space6) {
                ForEach(MenuPlan.build(bindings: bindingsInForce), id: \.title) { menu in
                    VStack(alignment: .leading, spacing: PostioTokens.space2) {
                        Text(menu.title.uppercased())
                            .font(.system(size: 10, weight: .semibold))
                            .kerning(0.6)
                            .foregroundStyle(.secondary)
                        ForEach(menu.items, id: \.title) { item in
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

    private func bindingsInForce(_ command: String) -> [String] {
        session?.bindings(for: command) ?? []
    }

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
                            .equalSegments(3, each: 80)
                            Text(
                                current.checkForMail == .poll
                                    ? "Every \(settingsHumanizeInterval(seconds: current.pollIntervalSecs))"
                                    : "Folders without push still poll every "
                                        + settingsHumanizeInterval(seconds: current.pollIntervalSecs)
                            )
                            .font(.footnote)
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
                        .equalSegments(4, each: 72)
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
                        .equalSegments(2, each: 116)
                    }
                    field("DOWNLOAD ATTACHMENTS") {
                        Picker("", selection: syncBinding(current, \.attachmentFetch)) {
                            Text("When opened").tag(AttachmentFetchFfi.onOpen)
                            Text("Ahead of time").tag(AttachmentFetchFfi.eager)
                            Text("Never").tag(AttachmentFetchFfi.never)
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .equalSegments(3, each: 116)
                    }
                    Spacer(minLength: 0)
                }
                Divider()
                field("ON STARTUP") {
                    VStack(alignment: .leading, spacing: 8) {
                        Toggle("Sync as soon as Postio opens", isOn: syncBinding(current, \.syncOnStartup))
                        Toggle("Notify me about new mail", isOn: syncBinding(current, \.notify))
                        Toggle("Show the unread count on the Dock icon", isOn: syncBinding(current, \.badge))
                    }
                    .toggleStyle(.checkbox)
                }
                Divider()
                let words = settingsAutoEmptyWords()
                field(words[0].uppercased()) {
                    VStack(alignment: .leading, spacing: PostioTokens.space2) {
                        HStack(alignment: .firstTextBaseline, spacing: 12) {
                            Text("Junk").frame(width: 40, alignment: .leading)
                            autoEmptyPicker(syncBinding(current, \.emptyJunkAfterDays))
                        }
                        HStack(alignment: .firstTextBaseline, spacing: 12) {
                            Text("Trash").frame(width: 40, alignment: .leading)
                            autoEmptyPicker(syncBinding(current, \.emptyTrashAfterDays))
                        }
                        Text(words[1])
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 0)
            }
        } else {
            unreadable
        }
    }

    private func autoEmptyPicker(_ days: Binding<UInt32>) -> some View {
        let choices = settingsAutoEmptyChoices()
        return Picker("", selection: days) {
            ForEach(choices, id: \.days) { choice in
                Text(choice.label).tag(choice.days)
            }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .equalSegments(choices.count, each: 76)
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

    /// No Save: settings are the config file, with no second store. The footer says whether it
    /// parses. Monospaced because alignment carries meaning in TOML, a point under the panes' text
    /// since monospaced reads larger; autocorrection and smart quotes are off because a curly quote
    /// in a TOML string is a parse error.
    @ViewBuilder private var configFile: some View {
        TextEditor(text: configBinding)
            .font(.system(.subheadline, design: .monospaced))
            .lineSpacing(3)
            .autocorrectionDisabled()
            .textEditorStyle(.plain)
            .scrollContentBackground(.hidden)
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .background(Color(nsColor: .textBackgroundColor))
            .overlay(
                RoundedRectangle(cornerRadius: PostioTokens.radiusMd)
                    .stroke(Color.secondary.opacity(0.25), lineWidth: 1)
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .accessibilityLabel("The configuration file")
    }

    /// Invalid TOML is written too: refusing to save until it parses would make the editor useless
    /// for fixing it.
    /// The footer says what is wrong and a running Postio keeps the last good configuration.
    private var configBinding: Binding<String> {
        Binding(
            get: { store.text },
            set: { store.write(text: $0) }
        )
    }

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

    private func editorAdvice(_ configured: String) -> String {
        switch settingsHandoffTarget(
            configured: configured,
            found: ComposeHandoff.found(configured)
        ) {
        case .platformDefault:
            "Empty: the draft opens in whatever this Mac opens a text file with."
        case .application(let name):
            "Drafts open in \(name)."
        case .needsTerminal(_, let advice), .missing(_, let advice):
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

    /// A newer core can name a section this build has never heard of; say so rather than draw an
    /// empty pane.
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

    /// Applied to whatever the file says at the moment of the click, never to the copy this view
    /// was drawn from.
    private func composeBinding<T>(
        _ current: ComposingFfi,
        _ field: WritableKeyPath<ComposingFfi, T>
    ) -> Binding<T> {
        Binding(
            get: { current[keyPath: field] },
            set: { value in store.applyComposing { $0[keyPath: field] = value } }
        )
    }

    private func editorBinding(_ current: ComposingFfi) -> Binding<String> {
        Binding(
            get: { current.editor },
            set: { value in store.applyComposing { $0.editor = value } }
        )
    }
}

extension View {
    /// `.fixedSize()` gave every segment its own label's width, so the row read as uneven; one
    /// width per picker, wide enough for its longest label, gives equal segments.
    fileprivate func equalSegments(_ count: Int, each: CGFloat) -> some View {
        frame(width: CGFloat(count) * each)
    }
}
