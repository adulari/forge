// Opens a notice the daemon sealed to this device: RFC 8291 (Web Push) aes128gcm, the same
// construction as crates/forge-cli/src/push.rs and mobile/src/lib/push/pushE2e.ts. The three are
// checked against one cross-language vector (protocol/fixtures/push-notice-vector.json) by the
// Rust and TypeScript tests; this file has no test of its own because the repo has no Swift test
// target, so keep it a line-for-line mirror of `openSealed` in pushE2e.ts.
//
// Every failure returns nil, and the caller then leaves the generic alert untouched.
import CryptoKit
import Foundation

struct ForgeSealedNotice {
    let title: String
    let message: String
    let sessionId: String
    let seq: Int
    let kind: String
}

enum ForgePushCrypto {
    private struct Plaintext: Decodable {
        let v: Int
        let title: String
        let message: String
        let session_id: String?
        let seq: Int?
        let kind: String?
    }

    static func base64URLDecode(_ value: String) -> Data? {
        var text = value.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        if text.count % 4 == 1 { return nil }
        while text.count % 4 != 0 { text.append("=") }
        return Data(base64Encoded: text)
    }

    static func open(blob: String, key: ForgePushKey) -> ForgeSealedNotice? {
        guard let message = base64URLDecode(blob) else { return nil }
        // salt(16) || record size(4) || key id length(1) || key id (sender public key, 65) || ciphertext
        guard message.count >= 16 + 4 + 1 + 65 + 17, message[message.startIndex + 20] == 65 else { return nil }
        let salt = message.subdata(in: message.startIndex..<message.startIndex + 16)
        let senderPublic = message.subdata(in: message.startIndex + 21..<message.startIndex + 86)
        let sealed = message.subdata(in: message.startIndex + 86..<message.endIndex)
        let tagStart = sealed.count - 16

        do {
            let privateKey = try P256.KeyAgreement.PrivateKey(rawRepresentation: key.privateKey)
            let sender = try P256.KeyAgreement.PublicKey(x963Representation: senderPublic)
            let secret = try privateKey.sharedSecretFromKeyAgreement(with: sender)

            var info = Data("WebPush: info\0".utf8)
            info.append(privateKey.publicKey.x963Representation)
            info.append(senderPublic)
            // HKDF-Extract(salt: auth secret, ikm: ECDH) then Expand(info), 32 bytes (RFC 8291 3.4).
            let ikm = secret.hkdfDerivedSymmetricKey(
                using: SHA256.self, salt: key.auth, sharedInfo: info, outputByteCount: 32)

            let cek = HKDF<SHA256>.deriveKey(
                inputKeyMaterial: ikm, salt: salt,
                info: Data("Content-Encoding: aes128gcm\0".utf8), outputByteCount: 16)
            let nonceKey = HKDF<SHA256>.deriveKey(
                inputKeyMaterial: ikm, salt: salt,
                info: Data("Content-Encoding: nonce\0".utf8), outputByteCount: 12)
            let nonce = try AES.GCM.Nonce(data: nonceKey.withUnsafeBytes { Data($0) })

            let box = try AES.GCM.SealedBox(
                nonce: nonce,
                ciphertext: sealed.subdata(in: sealed.startIndex..<sealed.startIndex + tagStart),
                tag: sealed.subdata(in: sealed.startIndex + tagStart..<sealed.endIndex))
            var record = try AES.GCM.open(box, using: cek)

            while record.last == 0 { record.removeLast() }
            guard !record.isEmpty, record.removeLast() == 0x02 else { return nil }

            let plain = try JSONDecoder().decode(Plaintext.self, from: record)
            guard plain.v == 1 else { return nil }
            return ForgeSealedNotice(
                title: plain.title, message: plain.message,
                sessionId: plain.session_id ?? "", seq: plain.seq ?? 0, kind: plain.kind ?? "")
        } catch {
            return nil
        }
    }
}
