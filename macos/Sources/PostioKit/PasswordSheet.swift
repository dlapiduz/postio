import PostioFFI
import SwiftUI

/// "Update password…" from the sign-in banner (screen 19): a sheet on the
/// main window asking for the account's new password, which goes straight
/// into the Keychain through the engine's credential store
/// (`AccountRepair.save`, `repair_credential`).
///
/// The same `AccountRepair` route the settings window's row takes, so the
/// banner and the row cannot store a password two ways. Never pre-filled:
/// the old password is what stopped working, and Postio does not have it.
public struct PasswordSheet: View {
    let account: AccountFfi
    let repair: AccountRepair
    let session: PostioSession?

    public init(account: AccountFfi, repair: AccountRepair, session: PostioSession?) {
        self.account = account
        self.repair = repair
        self.session = session
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("New password").font(.system(size: 15, weight: .bold))
            Text(
                """
                For \(account.address). It goes straight into your Keychain and \
                is never written to a file. If your provider calls this an app \
                password, that is the one it wants.
                """
            )
            .font(.system(size: 13))
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            SecureField("Password", text: Binding(get: { repair.typed }, set: { repair.typed = $0 }))
                .textFieldStyle(.roundedBorder)
                .onSubmit(save)
            if repair.failed, let outcome = repair.outcome {
                Label(outcome, systemImage: "xmark.circle")
                    .font(.system(size: 12))
                    .foregroundStyle(.primary)
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { repair.cancel() }
                    .keyboardShortcut(.cancelAction)
                Button(repair.running == account.id ? "Saving\u{2026}" : "Save", action: save)
                    .keyboardShortcut(.defaultAction)
                    .disabled(repair.typed.isEmpty || repair.running != nil)
            }
        }
        .padding(20)
        .frame(width: 420)
    }

    private func save() {
        Task { await repair.save(account, through: session) }
    }
}
