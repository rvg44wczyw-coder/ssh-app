import SwiftUI
import SshCoreBridge

/// Settings -> Servers: the list, the active server, add and edit.
struct ServersSettingsPage: View {
    @ObservedObject var serversManager: ServerProfilesManager
    @State private var serverToEdit: ServerProfile? = nil
    @State private var showAddServerSheet: Bool = false
    @State private var showPaywall: Bool = false
    @ObservedObject private var subs = SubscriptionManager.shared

    var body: some View {
        Form {
                Section(header: Text("SSH Servers"), footer: Text("Tap a server to set it as active. New terminal sessions connect to the active server.")) {
                    if serversManager.profiles.isEmpty {
                        Text("No servers configured")
                            .foregroundColor(.secondary)
                            .italic()
                    } else {
                        ForEach(serversManager.profiles) { server in
                            HStack {
                                VStack(alignment: .leading, spacing: 3) {
                                    HStack(spacing: 6) {
                                        Text(server.name)
                                            .font(.headline)
                                        if server.isDefault {
                                            Text("DEFAULT")
                                                .font(.system(size: 9, weight: .bold))
                                                .padding(.horizontal, 5)
                                                .padding(.vertical, 1)
                                                .foregroundColor(.blue)
                                                .glassCard(cornerRadius: 9, tint: (Color.blue.opacity(0.15)), interactive: true)
                                        }
                                    }
                                    Text(verbatim: "\(server.username)@\(server.host):\(server.port)")
                                        .font(.caption)
                                        .foregroundColor(.secondary)
                                }

                                Spacer()

                                if serversManager.activeProfileId == server.id {
                                    Image(systemName: "checkmark.circle.fill")
                                        .foregroundColor(.appAccent)
                                        .font(.title3)
                                }

                                Button(action: {
                                    serverToEdit = server
                                }) {
                                    Image(systemName: "pencil")
                                        .foregroundColor(.secondary)
                                }
                                .buttonStyle(BorderlessButtonStyle())
                                .padding(.leading, 8)
                            }
                            .contentShape(Rectangle())
                            .onTapGesture {
                                serversManager.activeProfileId = server.id
                            }
                        }
                        .onDelete { indexSet in
                            for index in indexSet {
                                let id = serversManager.profiles[index].id
                                serversManager.deleteProfile(id: id)
                            }
                        }
                    }

                    Button(action: {
                        if FreeTier.canAddServer(count: serversManager.profiles.count, isPro: subs.unlocked) {
                            showAddServerSheet = true
                        } else {
                            showPaywall = true
                        }
                    }) {
                        Label("Add Server...", systemImage: "plus.circle.fill")
                            .font(.subheadline.bold())
                    }
                }

        }
        .navigationTitle("Servers")
            .sheet(isPresented: $showPaywall) {
                PaywallView(reason: "The free version keeps up to \(FreeTier.maxServers) servers. Pro removes the limit.")
            }
            .sheet(isPresented: $showAddServerSheet) {
                AddEditServerSheetView(existingProfile: nil) { newProfile in
                    _ = serversManager.addProfile(
                        name: newProfile.name,
                        host: newProfile.host,
                        port: newProfile.port,
                        username: newProfile.username,
                        isDefault: newProfile.isDefault,
                        keyId: newProfile.keyId
                    )
                }
            }
            .sheet(item: $serverToEdit) { profile in
                AddEditServerSheetView(existingProfile: profile) { updated in
                    serversManager.updateProfile(updated)
                }
            }
    }
}
