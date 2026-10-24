import Foundation
import Combine

@MainActor
public final class ServerProfilesManager: ObservableObject {
    private static let profilesKey = "saved_server_profiles"
    private static let activeIdKey = "active_server_profile_id"

    @Published public var profiles: [ServerProfile] = []
    @Published public var activeProfileId: UUID?

    public init() {
        loadProfiles()
        FilesIntegration.refresh(profiles: profiles)
        VaultSync.shared.attach(self)
        Task { VaultSync.shared.sync() }
    }

    public var activeProfile: ServerProfile? {
        if let id = activeProfileId, let found = profiles.first(where: { $0.id == id }) {
            return found
        }
        return profiles.first(where: { $0.isDefault }) ?? profiles.first
    }

    public func profile(for id: UUID?) -> ServerProfile? {
        guard let id = id else { return activeProfile }
        return profiles.first(where: { $0.id == id }) ?? activeProfile
    }

    public func addProfile(
        name: String,
        host: String,
        port: UInt16 = 22,
        username: String,
        isDefault: Bool = false,
        keyId: String? = nil
    ) -> ServerProfile {
        let shouldBeDefault = isDefault || profiles.isEmpty
        if shouldBeDefault {
            for i in 0..<profiles.count {
                profiles[i].isDefault = false
            }
        }

        let newProfile = ServerProfile(
            name: name.trimmingCharacters(in: .whitespacesAndNewlines),
            host: host.trimmingCharacters(in: .whitespacesAndNewlines),
            port: port,
            username: username.trimmingCharacters(in: .whitespacesAndNewlines),
            isDefault: shouldBeDefault,
            keyId: keyId,
            updatedAt: Date()
        )

        profiles.append(newProfile)
        if activeProfileId == nil || shouldBeDefault {
            activeProfileId = newProfile.id
        }

        saveProfiles()
        return newProfile
    }

    /// Servers from a backup (feature 56): a server with an id already here is kept as it is, or
    /// replaced when `overwrite` is set; new ones are added. Nothing is ever removed. Returns how many
    /// were added or replaced.
    @discardableResult
    public func restore(_ incoming: [ServerProfile], overwrite: Bool) -> Int {
        var changed = 0
        for p in incoming {
            var q = p
            q.isDefault = false
            if let i = profiles.firstIndex(where: { $0.id == p.id }) {
                guard overwrite else { continue }
                q.isDefault = profiles[i].isDefault
                q.keyId = profiles[i].keyId
                profiles[i] = q
            } else {
                profiles.append(q)
            }
            changed += 1
        }
        if !profiles.isEmpty, !profiles.contains(where: { $0.isDefault }) { profiles[0].isDefault = true }
        if changed > 0 { saveProfiles() }
        return changed
    }

    public func updateProfile(_ updated: ServerProfile) {
        guard let index = profiles.firstIndex(where: { $0.id == updated.id }) else { return }

        if updated.isDefault {
            for i in 0..<profiles.count {
                profiles[i].isDefault = false
            }
        }

        profiles[index] = updated
        profiles[index].updatedAt = Date()
        saveProfiles()
    }

    /// Pins the host key seen on the first successful connection (trust on first use). A key that
    /// is already pinned is never replaced here; that only happens by forgetting it explicitly.
    public func pinHostKey(serverId: UUID, fingerprint: String) {
        guard let index = profiles.firstIndex(where: { $0.id == serverId }),
              profiles[index].hostKeyFingerprint == nil else { return }
        profiles[index].hostKeyFingerprint = fingerprint
        profiles[index].updatedAt = Date()
        saveProfiles()
    }

    public func deleteProfile(id: UUID) {
        VaultSync.shared.recordDeletion(of: id)
        profiles.removeAll(where: { $0.id == id })
        if activeProfileId == id {
            activeProfileId = profiles.first?.id
        }
        if !profiles.isEmpty && !profiles.contains(where: { $0.isDefault }) {
            profiles[0].isDefault = true
        }
        saveProfiles()
    }

    public func setDefault(id: UUID) {
        for i in 0..<profiles.count {
            profiles[i].isDefault = (profiles[i].id == id)
        }
        activeProfileId = id
        saveProfiles()
    }

    private func loadProfiles() {
        if let data = UserDefaults.standard.data(forKey: Self.profilesKey),
           let decoded = try? JSONDecoder().decode([ServerProfile].self, from: data),
           !decoded.isEmpty {
            self.profiles = decoded
            if let savedIdStr = UserDefaults.standard.string(forKey: Self.activeIdKey),
               let savedId = UUID(uuidString: savedIdStr),
               decoded.contains(where: { $0.id == savedId }) {
                self.activeProfileId = savedId
            } else {
                self.activeProfileId = decoded.first(where: { $0.isDefault })?.id ?? decoded.first?.id
            }
            return
        }

        // Migrate legacy single-host configuration from UserDefaults if present
        let legacyHost = UserDefaults.standard.string(forKey: "ssh_host") ?? ""
        let legacyUser = UserDefaults.standard.string(forKey: "ssh_username") ?? ""
        let legacyPort = UserDefaults.standard.integer(forKey: "ssh_port")
        let port: UInt16 = legacyPort > 0 ? UInt16(legacyPort) : 22

        if !legacyHost.isEmpty && !legacyUser.isEmpty {
            let defaultProfile = ServerProfile(
                name: "Primary Host",
                host: legacyHost,
                port: port,
                username: legacyUser,
                isDefault: true
            )
            self.profiles = [defaultProfile]
            self.activeProfileId = defaultProfile.id
            saveProfiles()
        }
    }

    /// Replaces the list with what a vault merge produced. Saves without stamping or syncing again.
    func applySynced(_ merged: [ServerProfile]) {
        profiles = merged
        if let id = activeProfileId, !merged.contains(where: { $0.id == id }) {
            activeProfileId = merged.first(where: { $0.isDefault })?.id ?? merged.first?.id
        }
        if !profiles.isEmpty && !profiles.contains(where: { $0.isDefault }) {
            profiles[0].isDefault = true
        }
        persist()
    }

    private func saveProfiles() {
        persist()
        Task { VaultSync.shared.sync() }
    }

    private func persist() {
        if let encoded = try? JSONEncoder().encode(profiles) {
            UserDefaults.standard.set(encoded, forKey: Self.profilesKey)
        }
        if let activeId = activeProfileId {
            UserDefaults.standard.set(activeId.uuidString, forKey: Self.activeIdKey)
        }
        FilesIntegration.refresh(profiles: profiles)
    }
}
