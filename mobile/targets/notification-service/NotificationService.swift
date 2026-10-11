// Replaces the generic lock-screen alert with the real, end-to-end sealed text.
//
// The daemon sends a generic alert plus an opaque `enc` blob (hosted relay and Apple only ever
// see that blob). If this device holds the key and the blob opens, the title and body are swapped
// in; in every other case (no key, app lock on, a build without the key, garbage, running out of
// time) the content goes out exactly as received, so the worst outcome is the generic alert.
import UserNotifications

final class NotificationService: UNNotificationServiceExtension {
    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttempt: UNMutableNotificationContent?

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        self.contentHandler = contentHandler
        guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else {
            contentHandler(request.content)
            return
        }
        bestAttempt = content

        if let blob = request.content.userInfo["enc"] as? String,
           let key = ForgePushKey.load(),
           let notice = ForgePushCrypto.open(blob: blob, key: key) {
            content.title = notice.title.isEmpty ? "Forge" : notice.title
            content.body = notice.message
            if !notice.sessionId.isEmpty { content.threadIdentifier = notice.sessionId }
            var info = content.userInfo
            info["enc"] = nil
            info["forgeDecrypted"] = true
            info["forgeSessionId"] = notice.sessionId
            info["forgeSeq"] = notice.seq
            info["forgeKind"] = notice.kind
            content.userInfo = info
        }
        contentHandler(content)
    }

    override func serviceExtensionTimeWillExpire() {
        if let contentHandler, let bestAttempt {
            contentHandler(bestAttempt)
        }
    }
}
