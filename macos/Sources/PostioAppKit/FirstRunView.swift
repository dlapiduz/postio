import PostioFFI
import SwiftUI
import PostioKit

/// The first-run wizard, as canvas 09 draws it: the address, the password,
/// what was found for the domain, and `Connect`.
///
/// The window *is* this on a fresh install, rather than an empty three-pane
/// shell with a button in it: with no account there is nothing for the
/// panes to show, and the one useful thing to do is this. `Settings ›
/// Accounts › +` keeps the sheet, whose other routes -- a browser sign-in,
/// a folder of mail on this Mac -- this links to.
public struct FirstRunView: View {
    @Bindable private var model: FirstRunModel
    /// The sheet's routes: browser sign-in and a local store.
    private let otherWays: () -> Void
    /// The account is saved and the sync window chosen.
    private let finished: () -> Void

    private enum Field: Hashable {
        case name, address, password, imapHost, smtpHost, login
    }

    @FocusState private var focus: Field?

    public init(model: FirstRunModel, otherWays: @escaping () -> Void, finished: @escaping () -> Void) {
        self.model = model
        self.otherWays = otherWays
        self.finished = finished
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                header
                Divider()
                VStack(alignment: .leading, spacing: PostioTokens.space6) {
                    switch model.phase {
                    case .account, .connecting: account
                    case .syncWindow, .finished: syncWindow
                    }
                }
                .padding(PostioTokens.space8)
            }
            .background(Color(nsColor: AppSurface.background))
            // `--color-divider` is ink at 16%; the ink is whichever the
            // appearance draws text in, so it is written that way rather
            // than as the token's fixed dark ink.
            .overlay(Rectangle().strokeBorder(Color.primary.opacity(0.16), lineWidth: 1))
            .frame(maxWidth: 640)
            .padding(PostioTokens.space8)
            .frame(maxWidth: .infinity)
        }
        .background(Color(nsColor: AppSurface.sidebar))
        .onAppear { focus = .name }
        .onChange(of: focus) { left, _ in
            // Leaving the field is the deliberate step that looks it up --
            // never a keystroke, since each lookup goes to the network.
            if left == .address { Task { await model.lookUp() } }
        }
        .onChange(of: model.phase) { _, now in
            if now == .finished { finished() }
        }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            Text("ADD ACCOUNT")
                .font(.system(size: 15, weight: .semibold).width(.condensed))
                .tracking(2.5)
                .foregroundStyle(.secondary)
            Spacer()
            Text("step \(model.counter)")
                .font(.system(.callout, design: .monospaced))
                .foregroundStyle(.secondary)
                .accessibilityLabel("Step \(model.counter)")
        }
        .padding(.horizontal, PostioTokens.space8)
        .padding(.vertical, PostioTokens.space6)
    }

    // -- steps 1 and 2: who you are, and proving it ---------------------------

    private var account: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space6) {
            // First, as on the desktop (5639e0ff): the `From` header and the
            // sidebar's label. Optional -- blank sends as the bare address --
            // and Return moves on, since there is nothing to look up yet.
            labelled("Your name") {
                TextField("", text: $model.name, prompt: Text("Ada Lovelace"))
                    .textContentType(.name)
                    .focused($focus, equals: .name)
                    .onSubmit { focus = .address }
                    .accessibilityLabel("Your name")
            }
            labelled("Email address") {
                TextField("", text: $model.address, prompt: Text("you@example.com"))
                    .textContentType(.emailAddress)
                    .focused($focus, equals: .address)
                    .onSubmit { focus = .password }
                    .accessibilityLabel("Email address")
            }
            if model.signsInWithBrowser {
                browser
            } else {
                labelled("Password") {
                    SecureField("", text: $model.password)
                        .textContentType(.password)
                        .focused($focus, equals: .password)
                        .onSubmit { Task { await model.connect() } }
                        .accessibilityLabel("Password")
                }
            }
            settings
            if let problem = model.problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .padding(PostioTokens.space3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: PostioTokens.radiusSm))
            }
            HStack(spacing: PostioTokens.space6) {
                if !model.signsInWithBrowser {
                    Button {
                        Task { await model.connect() }
                    } label: {
                        if model.phase == .connecting {
                            HStack(spacing: PostioTokens.space2) {
                                ProgressView().controlSize(.small).tint(.white)
                                Text("Signing in…")
                            }
                        } else {
                            Text("Connect")
                        }
                    }
                    .buttonStyle(ReaderButtonStyle(.primary))
                    .keyboardShortcut(.defaultAction)
                    .disabled(!model.canConnect)
                    .help("Sign in, and only then save the account")
                }
                if !model.editing, model.discovered != nil, !model.signsInWithBrowser {
                    Button("Edit manually") { model.editing = true }
                        .buttonStyle(.plain)
                        .font(.system(size: 13, weight: .semibold).width(.condensed))
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Other ways to add an account…", action: otherWays)
                    .buttonStyle(.link)
                    .font(.callout)
            }
            .disabled(model.phase == .connecting)
        }
        .textFieldStyle(.roundedBorder)
    }

    /// The panel under the password: what was found, a guess to check, or
    /// the servers to type.
    @ViewBuilder
    private var settings: some View {
        if model.lookingUp {
            panel {
                HStack(spacing: PostioTokens.space3) {
                    ProgressView().controlSize(.small)
                    Text("Looking up \(domain)…")
                        .foregroundStyle(.secondary)
                }
            }
        } else if let card = model.discovered, !model.signsInWithBrowser {
            panel {
                VStack(alignment: .leading, spacing: PostioTokens.space3) {
                    HStack(alignment: .firstTextBaseline, spacing: PostioTokens.space3) {
                        Image(systemName: card.found ? "checkmark" : "questionmark.circle")
                            .foregroundStyle(Color(nsColor: PostioTokens.colorAccent))
                        Text(card.heading)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if model.editing {
                        manual
                    } else {
                        line("IMAP", card.imapLine)
                        line("SMTP", card.smtpLine)
                        line("Auth", "password · secret in Keychain")
                    }
                    if let note = card.note {
                        Text(note)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    if card.requiresAppPassword {
                        HStack(spacing: PostioTokens.space2) {
                            Text("This provider needs an app password, not your own.")
                                .font(.callout)
                            if let help = card.helpUrl, let url = URL(string: help) {
                                Link("Make one…", destination: url)
                                    .font(.callout)
                            }
                        }
                    }
                }
            }
        }
    }

    /// The servers, open for typing.
    private var manual: some View {
        Grid(alignment: .leading, horizontalSpacing: PostioTokens.space3, verticalSpacing: PostioTokens.space3) {
            server("IMAP", host: $model.imapHost, port: $model.imapPort, security: $model.imapSecurity, field: .imapHost)
            server("SMTP", host: $model.smtpHost, port: $model.smtpPort, security: $model.smtpSecurity, field: .smtpHost)
            GridRow {
                mono("Login")
                TextField("", text: $model.login)
                    .focused($focus, equals: .login)
                    .accessibilityLabel("Login")
                    .gridCellColumns(3)
            }
        }
    }

    private func server(
        _ name: String,
        host: Binding<String>,
        port: Binding<Int>,
        security: Binding<SecurityFfi>,
        field: Field
    ) -> some View {
        GridRow {
            mono(name)
            TextField("", text: host, prompt: Text("\(name.lowercased()).example.com"))
                .focused($focus, equals: field)
                .accessibilityLabel("\(name) server")
            TextField("", value: port, format: .number.grouping(.never))
                .frame(width: 64)
                .accessibilityLabel("\(name) port")
            Picker("", selection: security) {
                Text("TLS").tag(SecurityFfi.tls)
                Text("STARTTLS").tag(SecurityFfi.startTls)
            }
            .labelsHidden()
            .frame(width: 110)
            .accessibilityLabel("\(name) security")
        }
    }

    /// A provider that signs in through the browser: Postio never draws its
    /// login form, so the way in is the sheet's browser route.
    private var browser: some View {
        panel {
            VStack(alignment: .leading, spacing: PostioTokens.space3) {
                Text("\(model.discovered?.source ?? domain) signs in through your browser.")
                Text("Consent happens in your browser, against the provider's own page. Postio asks for your mail and nothing else.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Sign in with your browser…", action: otherWays)
                    .buttonStyle(ReaderButtonStyle(.primary))
            }
        }
    }

    // -- step 3: how far back --------------------------------------------------

    private var syncWindow: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space6) {
            HStack(alignment: .firstTextBaseline, spacing: PostioTokens.space3) {
                Image(systemName: "checkmark")
                    .foregroundStyle(Color(nsColor: PostioTokens.colorAccent))
                Text("\(model.address) is connected.")
            }
            Text("How far back should Postio fetch?")
                .font(.title3.weight(.semibold))
            Picker("", selection: $model.window) {
                ForEach(model.choices, id: \.window) { choice in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(choice.label)
                        Text(choice.estimate)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    .tag(choice.window)
                }
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()
            Text("Older mail stays on the server: nothing there is moved or deleted.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Button("Start sync") { model.startSync() }
                .buttonStyle(ReaderButtonStyle(.primary))
                .keyboardShortcut(.defaultAction)
        }
    }

    // -- pieces ----------------------------------------------------------------

    private var domain: String {
        model.address.split(separator: "@").last.map(String.init) ?? model.address
    }

    private func labelled<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: PostioTokens.space2) {
            Text(title)
                .foregroundStyle(.secondary)
            content()
                .controlSize(.large)
        }
    }

    /// The bracketed panel the canvas draws round what was found.
    private func panel<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        content()
            .padding(PostioTokens.space6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.primary.opacity(0.03))
            .overlay(Rectangle().strokeBorder(Color.primary.opacity(0.16), lineWidth: 1))
    }

    private func line(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: PostioTokens.space4) {
            mono(label)
            Text(value)
                .font(.system(.body, design: .monospaced))
                .textSelection(.enabled)
        }
    }

    private func mono(_ text: String) -> some View {
        Text(text)
            .font(.system(.body, design: .monospaced))
            .foregroundStyle(.secondary)
            .frame(width: 44, alignment: .leading)
    }
}
