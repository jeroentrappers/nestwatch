// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import Security

/// The engines' storage (device key, trust lists, settings) as Keychain
/// generic-password items (security.md §5, PLATFORM-3, IOS-2): service
/// `org.nestwatch.Nestwatch`, account = the value's name, accessible after
/// first unlock and never synchronised or migrated to another device, so
/// keys stay out of backups. Values are never logged.
final class KeychainStorage: Storage, @unchecked Sendable {
    static let service = "org.nestwatch.Nestwatch"
    private let service: String
    private let lock = NSLock()

    init(service: String = KeychainStorage.service, defaults: UserDefaults = .standard) {
        self.service = service
        // Keychain items outlive an uninstall; user defaults do not. A fresh
        // install starts with a fresh identity, as on the other platforms.
        let marker = "keychain-owned"
        if !defaults.bool(forKey: marker) {
            removeAll()
            defaults.set(true, forKey: marker)
        }
    }

    private func base(_ key: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
        ]
    }

    private func failure(_ what: String, _ status: OSStatus) -> StorageFailure {
        .Backend(reason: "keychain \(what): \(status)")
    }

    func get(key: String) throws -> Data? {
        lock.lock()
        defer { lock.unlock() }
        var q = base(key)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &out)
        switch status {
        case errSecSuccess: return out as? Data ?? Data()
        case errSecItemNotFound: return nil
        default: throw failure("read", status)
        }
    }

    func put(key: String, value: Data) throws {
        lock.lock()
        defer { lock.unlock() }
        let attrs: [String: Any] = [
            kSecValueData as String: value,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        var status = SecItemUpdate(base(key) as CFDictionary, attrs as CFDictionary)
        if status == errSecItemNotFound {
            var add = base(key)
            add.merge(attrs) { _, new in new }
            add[kSecAttrSynchronizable as String] = false
            status = SecItemAdd(add as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw failure("write", status) }
    }

    func delete(key: String) throws {
        lock.lock()
        defer { lock.unlock() }
        let status = SecItemDelete(base(key) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw failure("delete", status) }
    }

    /// Removes every value of this service.
    func removeAll() {
        let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service]
        SecItemDelete(q as CFDictionary)
    }
}
