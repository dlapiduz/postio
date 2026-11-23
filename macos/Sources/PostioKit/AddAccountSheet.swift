import PostioFFI
import SwiftUI
import UniformTypeIdentifiers

public struct AddAccountSheet: View {
    private let session: PostioSession?
    @Bindable private var model: AddAccountModel
    /// Cancel and a successful add both close the sheet; the difference decides whether anything
    /// needs starting (#1299).
    private let done: (Bool) -> Void

    @FocusState private var focusedAddress: Bool
    @State private var choosingStore = false

    public init(
        session: PostioSession?,
        model: AddAccountModel,
        done: @escaping (Bool) -> Void
    ) {
        self.session = session
        self.model = model
        self.done = done
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space4) {
            HStack(alignment: .firstTextBaseline) {
                Text("Add an account")
                    .font(.title2.weight(.semibold))
                Spacer()
                Text(model.counter)
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Step \(model.counter)")
            }
            step
            if let problem = model.problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .foregroundStyle(.primary)
                    .padding(PostioTokens.space3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: PostioTokens.radiusMd))
            }
            Spacer(minLength: 0)
            buttons
        }
        .padding(PostioTokens.space6)
        .frame(width: 560, height: 480)
        .onAppear { focusedAddress = true }
    }

    @ViewBuilder
    private var step: some View {
        switch model.step {
        case .address: address
        case .credentials: credentials
        case .store: store
        }
    }

    private var address: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space4) {
            labelled("Email address") {
                TextField("", text: $model.address)
                    .textFieldStyle(.roundedBorder)
                    .focused($focusedAddress)
                    .accessibilityLabel("Email address")
            }
            // From the preset table, not a branch per provider.
            Label(model.verdict, systemImage: "checkmark.circle")
                .font(.callout)
                .padding(PostioTokens.space3)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    Color(nsColor: PostioTokens.colorAccent).opacity(0.10),
                    in: .rect(cornerRadius: PostioTokens.radiusMd)
                )
            VStack(spacing: PostioTokens.space2) {
                route(.outlook, "Outlook / Microsoft 365", "opens Safari · oauth2 + PKCE", "square.grid.2x2")
                route(.gmail, "Gmail / Google Workspace", "opens Safari · oauth2 + PKCE", "envelope")
                route(.imap, "IMAP / SMTP", "password in Keychain", "tray")
                route(.localStore, "Existing local store", "maildir, mbox or notmuch", "internaldrive")
            }
            Text(routeNote)
                .font(.callout)
                .foregroundStyle(.secondary)
        }
    }

    /// One sentence per route: the OAuth paragraph under "Existing local store" described a sign-in
    /// that route never makes.
    private var routeNote: String {
        switch model.route {
        case .outlook, .gmail:
            """
            Consent happens in your browser and returns on a loopback port. \
            The refresh token goes to the login Keychain — never to config.toml. \
            Postio asks for your mail and nothing else: not your contacts, \
            not your calendar, not your files.
            """
        case .imap:
            "The password goes to your login Keychain — never to config.toml."
        case .localStore:
            "Postio reads the folder where it is. Nothing is copied off this Mac, and nothing signs in."
        }
    }

    private func route(_ value: RouteFfi, _ title: String, _ detail: String, _ symbol: String) -> some View {
        Button {
            model.route = value
        } label: {
            HStack(spacing: PostioTokens.space3) {
                Image(systemName: symbol)
                    .frame(width: 22)
                VStack(alignment: .leading, spacing: 1) {
                    Text(title)
                    Text(detail)
                        .font(.system(.callout, design: .monospaced))
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
            .padding(PostioTokens.space3)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .overlay(
                RoundedRectangle(cornerRadius: PostioTokens.radiusMd)
                    .stroke(
                        model.route == value
                            ? Color(nsColor: PostioTokens.colorAccent)
                            : Color.secondary.opacity(0.25),
                        lineWidth: model.route == value ? 2 : 1
                    )
            )
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(model.route == value ? [.isSelected] : [])
    }

    @ViewBuilder
    private var credentials: some View {
        switch model.route {
        case .imap:
            VStack(alignment: .leading, spacing: PostioTokens.space4) {
                labelled("Password") {
                    SecureField("", text: $model.password)
                        .textFieldStyle(.roundedBorder)
                        .accessibilityLabel("Password")
                }
                HStack(spacing: PostioTokens.space3) {
                    labelled("Incoming (IMAP)") {
                        TextField("imap.example.com", text: $model.imapHost)
                            .textFieldStyle(.roundedBorder)
                    }
                    labelled("Port") {
                        TextField("", value: $model.imapPort, format: .number.grouping(.never))
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 70)
                    }
                }
                HStack(spacing: PostioTokens.space3) {
                    labelled("Outgoing (SMTP)") {
                        TextField("smtp.example.com", text: $model.smtpHost)
                            .textFieldStyle(.roundedBorder)
                    }
                    labelled("Port") {
                        TextField("", value: $model.smtpPort, format: .number.grouping(.never))
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 70)
                    }
                }
                Text("The password goes to your login Keychain, never to config.toml.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        case .outlook, .gmail:
            VStack(alignment: .leading, spacing: PostioTokens.space4) {
                Text("Signing in opens your browser")
                    .font(.headline)
                Text(
                    """
                    Postio never shows you a sign-in form of its own: a password \
                    typed into a mail client is a password that client could \
                    keep. Consent happens in your browser and returns on a \
                    loopback port, and only a refresh token comes back — into \
                    the login Keychain.
                    """
                )
                .foregroundStyle(.secondary)

                // What is asked for and what is not: only Postio can say what it deliberately left
                // out.
                let scopes = signInScopes(address: model.address)
                VStack(alignment: .leading, spacing: PostioTokens.space2) {
                    Label(scopes.askedFor, systemImage: "checkmark.circle")
                    Label(scopes.notAskedFor, systemImage: "minus.circle")
                        .foregroundStyle(.secondary)
                    if !scopes.requested.isEmpty {
                        Text(scopes.requested.joined(separator: "\n"))
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(.tertiary)
                            .textSelection(.enabled)
                    }
                }
                .font(.callout)

                labelled("Your OAuth client id") {
                    TextField("", text: $model.clientId)
                        .textFieldStyle(.roundedBorder)
                        .accessibilityLabel("Your OAuth client id")
                }
                Text(
                    """
                    Postio ships no client id: one inside an open-source \
                    application is one every user of it shares, and a provider \
                    that notices revokes it for all of them at once. Register \
                    one for yourself, or add this account with a password over \
                    IMAP instead.
                    """
                )
                .font(.callout)
                .foregroundStyle(.secondary)

                if model.signingIn {
                    HStack(spacing: PostioTokens.space3) {
                        ProgressView().controlSize(.small)
                        Text(model.signInMessage)
                            .font(.system(.callout, design: .monospaced))
                    }
                    .accessibilityElement(children: .combine)
                }
            }
        case .localStore:
            Text("Choose the store on the next step.")
                .foregroundStyle(.secondary)
        }
    }

    private var store: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space4) {
            labelled("Store path") {
                VStack(alignment: .leading, spacing: PostioTokens.space2) {
                    HStack(spacing: PostioTokens.space2) {
                        TextField("~/mail", text: $model.storePath)
                            .textFieldStyle(.roundedBorder)
                        Button("Choose…") { choosingStore = true }
                            .fileImporter(
                                isPresented: $choosingStore,
                                allowedContentTypes: [.folder]
                            ) { result in
                                if case let .success(url) = result {
                                    model.storePath = url.path
                                }
                            }
                    }
                    // Said where the field is: a directory picked by mistake must not become an
                    // account that looks empty (#1278).
                    if let problem = model.storeProblem(through: session) {
                        Text(problem)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            labelled("Format") {
                // Stated, not offered: a control listing formats Postio cannot open reads as a
                // choice until you make it (#1296).
                Text("Maildir")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            labelled("Sync window") {
                VStack(alignment: .leading, spacing: PostioTokens.space2) {
                    Picker("", selection: $model.syncDays) {
                        Text("30 days").tag(30)
                        Text("90 days").tag(90)
                        Text("1 year").tag(365)
                        Text("Everything").tag(0)
                    }
                    .pickerStyle(.segmented)
                    .fixedSize()
                    Text(model.estimate)
                        .font(.system(.callout, design: .monospaced))
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private var buttons: some View {
        HStack {
            if model.step != .address {
                Button("Back") { model.back() }
            }
            Spacer()
            Button("Cancel", role: .cancel) {
                model.cancelSignIn(through: session)
                done(false)
            }
            .keyboardShortcut(.cancelAction)
            Button(continueTitle) {
                if signsInHere {
                    Task { if await model.signIn(through: session) { done(true) } }
                } else if model.step == .store {
                    Task { if await model.finish(through: session) { done(true) } }
                } else {
                    model.next()
                }
            }
            .keyboardShortcut(.defaultAction)
            .buttonStyle(.borderedProminent)
            .disabled(!model.canContinue || model.signingIn || model.finishing)
        }
    }

    private var signsInHere: Bool {
        model.step == .credentials && (model.route == .outlook || model.route == .gmail)
    }

    private var continueTitle: String {
        if signsInHere { return model.signingIn ? "Signing in…" : "Sign in" }
        if model.finishing { return "Signing in…" }
        return model.step == .store ? "Start sync" : "Continue"
    }

    private func labelled<Content: View>(
        _ label: String,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: PostioTokens.space2) {
            Text(label)
                .font(.callout)
                .foregroundStyle(.secondary)
            content()
        }
    }
}
