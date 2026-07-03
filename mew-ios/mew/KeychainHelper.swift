import Foundation
import Security

/// Loads and stores the phone's persistent iroh secret key in the iOS Keychain.
/// The key must not iCloud-sync (per spec: "device private keys never leave devices").
enum KeychainHelper {
    private static let service = "ai.mew.mew"
    private static let account = "iroh-secret-key"

    static func loadOrCreateSecretKey() -> Data {
        if let existing = load() {
            return existing
        }
        // Generate a new 32-byte key
        var bytes = [UInt8](repeating: 0, count: 32)
        let status = SecRandomCopyBytes(kSecRandomDefault, 32, &bytes)
        guard status == errSecSuccess else {
            fatalError("Failed to generate secret key: \(status)")
        }
        let key = Data(bytes)
        save(key)
        return key
    }

    private static func load() -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess else { return nil }
        return item as? Data
    }

    private static func save(_ key: Data) {
        let attributes: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecValueData as String: key,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        SecItemAdd(attributes as CFDictionary, nil)
    }
}
