// Reads the device's notification key from the shared keychain group. The main app writes it with
// expo-secure-store (mobile/src/lib/push/push.ios.ts), which stores a UTF-8 JSON string as a
// generic password whose service, account and generic attribute are the values below; keep them in
// sync with PUSH_KEY_SERVICE / PUSH_KEY_NAME in mobile/src/lib/push/pushE2e.ts.
//
// No access group is named in the query: a keychain search covers every group in this process's
// own entitlements, and the extension's only group is the shared one, so a team-prefix mismatch
// cannot make the lookup silently miss.
import Foundation
import Security

struct ForgePushKey {
    let privateKey: Data
    let auth: Data

    private static let service = "forge.push.e2e"
    private static let name = "forge-push-e2e-v1"

    private struct Stored: Decodable {
        let v: Int
        let d: String
        let a: String
    }

    static func load() -> ForgePushKey? {
        let encodedName = Data(name.utf8)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrGeneric as String: encodedName,
            kSecAttrAccount as String: encodedName,
            kSecMatchLimit as String: kSecMatchLimitOne,
            kSecReturnData as String: kCFBooleanTrue as Any,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let stored = try? JSONDecoder().decode(Stored.self, from: data),
              stored.v == 1,
              let privateKey = ForgePushCrypto.base64URLDecode(stored.d),
              let auth = ForgePushCrypto.base64URLDecode(stored.a)
        else {
            return nil
        }
        return ForgePushKey(privateKey: privateKey, auth: auth)
    }
}
