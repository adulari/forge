// Bridges ActivityKit's Activity<ForgeSessionActivityAttributes> lifecycle (request/update/end)
// to JS. This module compiles as its own CocoaPod, so its ActivityAttributes source must remain
// byte-for-byte identical to the widget extension's copy.
import ActivityKit
import ExpoModulesCore
import OSLog

public class LiveActivityModule: Module {
    private let logger = Logger(subsystem: "dev.adulari.forge", category: "LiveActivity")

    public func definition() -> ModuleDefinition {
        Name("LiveActivity")
        Events("pushToken")

        Function("isSupported") { () -> Bool in
            if #available(iOS 16.1, *) {
                return ActivityAuthorizationInfo().areActivitiesEnabled
            }
            return false
        }

        AsyncFunction("start") {
            (sessionId: String, title: String, busy: Bool, waiting: Bool, costUsd: Double, contextTokens: Int, contextLimit: Int) throws -> [String: String?] in
            guard #available(iOS 16.1, *) else {
                return ["activityId": nil, "pushToken": nil]
            }
            guard ActivityAuthorizationInfo().areActivitiesEnabled else {
                self.logger.notice("Live Activities are disabled for session \(sessionId, privacy: .public)")
                return ["activityId": nil, "pushToken": nil]
            }

            if let existing = Activity<ForgeSessionActivityAttributes>.activities.first(where: {
                $0.attributes.sessionId == sessionId
            }) {
                self.observePushTokens(for: existing)
                return ["activityId": existing.id, "pushToken": nil]
            }

            let attributes = ForgeSessionActivityAttributes(sessionId: sessionId, title: title)
            let state = ForgeSessionActivityAttributes.ContentState(
                busy: busy, waiting: waiting, costUsd: costUsd, contextTokens: contextTokens, contextLimit: contextLimit
            )
            do {
                let activity = try Activity.request(
                    attributes: attributes,
                    content: .init(state: state, staleDate: nil),
                    pushType: .token
                )
                self.logger.info("Started Live Activity \(activity.id, privacy: .public) for session \(sessionId, privacy: .public)")
                self.observePushTokens(for: activity)
                return ["activityId": activity.id, "pushToken": nil]
            } catch {
                self.logger.error("Failed to start Live Activity for session \(sessionId, privacy: .public): \(error.localizedDescription, privacy: .public)")
                throw error
            }
        }

        AsyncFunction("update") {
            (activityId: String, busy: Bool, waiting: Bool, costUsd: Double, contextTokens: Int, contextLimit: Int) async throws in
            guard #available(iOS 16.1, *) else { return }
            guard let activity = Activity<ForgeSessionActivityAttributes>.activities.first(where: { $0.id == activityId }) else {
                throw LiveActivityError.activityNotFound(activityId)
            }
            let state = ForgeSessionActivityAttributes.ContentState(
                busy: busy, waiting: waiting, costUsd: costUsd, contextTokens: contextTokens, contextLimit: contextLimit
            )
            await activity.update(.init(state: state, staleDate: nil))
        }

        AsyncFunction("end") { (activityId: String) async throws in
            guard #available(iOS 16.1, *) else { return }
            guard let activity = Activity<ForgeSessionActivityAttributes>.activities.first(where: { $0.id == activityId }) else {
                return
            }
            await activity.end(nil, dismissalPolicy: .immediate)
            self.logger.info("Ended Live Activity \(activityId, privacy: .public)")
        }
    }

    @available(iOS 16.1, *)
    private func observePushTokens(for activity: Activity<ForgeSessionActivityAttributes>) {
        Task { [weak self] in
            for await data in activity.pushTokenUpdates {
                let token = data.map { String(format: "%02x", $0) }.joined()
                self?.logger.info("Received Live Activity push token for session \(activity.attributes.sessionId, privacy: .public)")
                self?.sendEvent("pushToken", ["sessionId": activity.attributes.sessionId, "pushToken": token])
            }
        }
    }
}

private enum LiveActivityError: LocalizedError {
    case activityNotFound(String)

    var errorDescription: String? {
        switch self {
        case let .activityNotFound(id):
            return "Live Activity \(id) is no longer active"
        }
    }
}
