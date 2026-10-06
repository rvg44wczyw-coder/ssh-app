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
        isDefault: Bool = false
    ) -> ServerProfile {
        var shouldBeDefault = isDefault || profiles.isEmpty
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
            isDefault: shouldBeDefault
        )

        profiles.append(newProfile)
        if activeProfileId == nil || shouldBeDefault {
            activeProfileId = newProfile.id
        }

        saveProfiles()
        return newProfile
    }

    public func updateProfile(_ updated: ServerProfile) {
        guard let index = profiles.firstIndex(where: { $0.id == updated.id }) else { return }

        if updated.isDefault {
            for i in 0..<profiles.count {
                profiles[i].isDefault = false
            }
        }

        profiles[index] = updated
        saveProfiles()
    }

    public func deleteProfile(id: UUID) {
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

    private func saveProfiles() {
        if let encoded = try? JSONEncoder().encode(profiles) {
            UserDefaults.standard.set(encoded, forKey: Self.profilesKey)
        }
        if let activeId = activeProfileId {
            UserDefaults.standard.set(activeId.uuidString, forKey: Self.activeIdKey)
        }
    }
}
