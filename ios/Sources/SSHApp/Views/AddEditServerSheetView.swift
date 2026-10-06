import SwiftUI

public struct AddEditServerSheetView: View {
    @Environment(\.dismiss) private var dismiss

    private let existingProfile: ServerProfile?
    private let onSave: (ServerProfile) -> Void

    @State private var name: String = ""
    @State private var host: String = ""
    @State private var port: Int = 22
    @State private var username: String = ""
    @State private var isDefault: Bool = false
    @State private var errorMessage: String? = nil

    public init(
        existingProfile: ServerProfile? = nil,
        onSave: @escaping (ServerProfile) -> Void
    ) {
        self.existingProfile = existingProfile
        self.onSave = onSave
        _name = State(initialValue: existingProfile?.name ?? "")
        _host = State(initialValue: existingProfile?.host ?? "")
        _port = State(initialValue: Int(existingProfile?.port ?? 22))
        _username = State(initialValue: existingProfile?.username ?? "")
        _isDefault = State(initialValue: existingProfile?.isDefault ?? false)
    }

    public var body: some View {
        NavigationView {
            Form {
                Section(header: Text("Server Details")) {
                    TextField("Server Name (e.g. MacBook Pro)", text: $name)

                    TextField("Host / IP (e.g. 100.x.y.z)", text: $host)
                        #if os(iOS)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)
                        #endif

                    TextField("SSH Username", text: $username)
                        #if os(iOS)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)
                        #endif

                    HStack {
                        Text("Port")
                        Spacer()
                        TextField("22", value: $port, formatter: NumberFormatter())
                            #if os(iOS)
                            .keyboardType(.numberPad)
                            #endif
                            .multilineTextAlignment(.trailing)
                    }
                }

                Section {
                    Toggle("Default Server", isOn: $isDefault)
                }

                if let error = errorMessage {
                    Section {
                        Text(error)
                            .foregroundColor(.red)
                            .font(.caption)
                    }
                }
            }
            .navigationTitle(existingProfile == nil ? "Add Server" : "Edit Server")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        save()
                    }
                    .font(.headline)
                }
            }
        }
    }

    private func save() {
        let trimmedHost = host.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedUser = username.trimmingCharacters(in: .whitespacesAndNewlines)
        var trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)

        guard !trimmedHost.isEmpty else {
            errorMessage = "Host address is required."
            return
        }
        guard !trimmedUser.isEmpty else {
            errorMessage = "Username is required."
            return
        }
        if trimmedName.isEmpty {
            trimmedName = trimmedHost
        }

        let profile = ServerProfile(
            id: existingProfile?.id ?? UUID(),
            name: trimmedName,
            host: trimmedHost,
            port: UInt16(clamping: port),
            username: trimmedUser,
            isDefault: isDefault
        )

        onSave(profile)
        dismiss()
    }
}
