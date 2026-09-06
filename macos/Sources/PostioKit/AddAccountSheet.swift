import PostioFFI
import SwiftUI

/// Adding an account: a sheet dropping from the Settings window's title bar
/// (#1279, canvas screen 27).
///
/// A sheet rather than a window, because it is a thing you are doing *to*
/// the settings window and cannot meaningfully leave half-done in the
/// background. Three steps, counted in the corner, `Cancel` and `Continue`
/// bottom right — `Esc` cancels and `⏎` continues, which is what a Mac user
/// will try before reading anything.
public struct AddAccountSheet: View {
    private let session: PostioSession?
    @Bindable private var model: AddAccountModel
    private let done: () -> Void

    @FocusState private var focusedAddress: Bool

    public init(
        session: PostioSession?,
        model: AddAccountModel,
        done: @escaping () -> Void
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

    // -- step 1: who you are, and how to reach them --------------------------

    private var address: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space4) {
            labelled("Email address") {
                TextField("", text: $model.address)
                    .textFieldStyle(.roundedBorder)
                    .focused($focusedAddress)
                    .accessibilityLabel("Email address")
            }
            // The verdict, from the preset table rather than from a branch
            // per provider: Postio is not built for any one of them.
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
            Text(
                """
                Consent happens in your browser and returns on a loopback port. \
                The refresh token goes to the login Keychain — never to config.toml. \
                Postio asks for your mail and nothing else: not your contacts, \
                not your calendar, not your files.
                """
            )
            .font(.callout)
            .foregroundStyle(.secondary)
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

    // -- step 2: the credential ---------------------------------------------

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
            VStack(alignment: .leading, spacing: PostioTokens.space3) {
                Text("Signing in opens your browser")
                    .font(.headline)
                Text(
                    """
                    Postio never shows you a sign-in form of its own: a password \
                    typed into a mail client is a password that client could \
                    keep. Consent happens in Safari and returns on a loopback \
                    port, and only a refresh token comes back — into the login \
                    Keychain.
                    """
                )
                .foregroundStyle(.secondary)
            }
        case .localStore:
            Text("Choose the store on the next step.")
                .foregroundStyle(.secondary)
        }
    }

    // -- step 3: where the mail lands ---------------------------------------

    private var store: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space4) {
            labelled("Store path") {
                TextField("~/mail", text: $model.storePath)
                    .textFieldStyle(.roundedBorder)
            }
            labelled("Format") {
                Picker("", selection: $model.format) {
                    ForEach(AddAccountModel.Format.allCases) { format in
                        Text(format.rawValue).tag(format)
                    }
                }
                .pickerStyle(.segmented)
                .fixedSize()
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

    // -- the frame ----------------------------------------------------------

    private var buttons: some View {
        HStack {
            if model.step != .address {
                Button("Back") { model.back() }
            }
            Spacer()
            Button("Cancel", role: .cancel) { done() }
                .keyboardShortcut(.cancelAction)
            Button(model.step == .store ? "Start sync" : "Continue") {
                if model.step == .store {
                    if model.finish(through: session) { done() }
                } else {
                    model.next()
                }
            }
            .keyboardShortcut(.defaultAction)
            .buttonStyle(.borderedProminent)
            .disabled(!model.canContinue)
        }
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
