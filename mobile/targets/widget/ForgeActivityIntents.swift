// Allow/Deny buttons on the Hearth "NEEDS YOU" Live Activity card and Dynamic Island expanded
// view (mobile.dc.html's "Mobile Live Activity" / "Mobile Dynamic Island" screens) resolve
// without opening the app — that's what `LiveActivityIntent` (iOS 17+) is for. They POST the
// same decision the in-app UI sends (`answer()` in mobile/src/lib/api.ts, backed by
// `POST /<token>/api/answer` in crates/forge-cli/src/serve.rs). `attributes.baseUrl` already
// carries that daemon token as a path segment, so no extra credential lookup is needed here —
// see ForgeSessionActivityAttributes.swift's header comment.
import ActivityKit
import AppIntents
import OSLog

@available(iOS 17.0, *)
struct ForgeAllowIntent: LiveActivityIntent {
    static var title: LocalizedStringResource = "Allow"

    @Parameter(title: "Session ID")
    var sessionId: String

    @Parameter(title: "Base URL")
    var baseUrl: String

    @Parameter(title: "Sequence")
    var seq: Int

    func perform() async throws -> some IntentResult {
        await ForgeActivityDecision.send(sessionId: sessionId, baseUrl: baseUrl, seq: seq, allow: true)
        return .result()
    }
}

@available(iOS 17.0, *)
struct ForgeDenyIntent: LiveActivityIntent {
    static var title: LocalizedStringResource = "Deny"

    @Parameter(title: "Session ID")
    var sessionId: String

    @Parameter(title: "Base URL")
    var baseUrl: String

    @Parameter(title: "Sequence")
    var seq: Int

    func perform() async throws -> some IntentResult {
        await ForgeActivityDecision.send(sessionId: sessionId, baseUrl: baseUrl, seq: seq, allow: false)
        return .result()
    }
}

@available(iOS 16.1, *)
private enum ForgeActivityDecision {
    private static let logger = Logger(subsystem: "dev.adulari.forge", category: "LiveActivityIntent")

    static func send(sessionId: String, baseUrl: String, seq: Int, allow: Bool) async {
        guard !baseUrl.isEmpty else {
            logger.error("Missing base URL for session \(sessionId, privacy: .public) decision")
            return
        }
        let path = baseUrl.hasSuffix("/") ? "\(baseUrl)api/answer" : "\(baseUrl)/api/answer"
        guard let url = URL(string: path) else {
            logger.error("Invalid base URL for session \(sessionId, privacy: .public) decision")
            return
        }

        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try? JSONSerialization.data(withJSONObject: [
            "session": sessionId,
            "seq": seq,
            "allow": allow,
        ])

        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
                logger.error("Decision POST for session \(sessionId, privacy: .public) failed")
                return
            }
        } catch {
            logger.error("Decision POST errored for session \(sessionId, privacy: .public): \(error.localizedDescription, privacy: .public)")
            return
        }

        applyOptimisticState(sessionId: sessionId)
    }

    /// Flips `waiting` off locally so the card updates instantly instead of waiting on the next
    /// APNs push — the daemon's own push (once it processes the decision) remains the source of
    /// truth and will correct this if anything raced.
    private static func applyOptimisticState(sessionId: String) {
        guard let activity = Activity<ForgeSessionActivityAttributes>.activities.first(where: {
            $0.attributes.sessionId == sessionId
        }) else { return }
        var state = activity.content.state
        state.waiting = false
        state.question = nil
        state.promptSeq = nil
        Task { await activity.update(.init(state: state, staleDate: nil)) }
    }
}
