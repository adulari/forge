// Bridges ActivityKit's Activity<ForgeSessionActivityAttributes> lifecycle (request/update/end)
// to JS — there is no JS-callable API for Live Activities anywhere else in this project's
// dependency tree, so this is a small first-party Expo Module rather than a third-party
// package (keeps this on ActivityKit's own supported surface, no dependency on an
// unmaintained-risk community wrapper). `ForgeSessionActivityAttributes` is shared by
// reference with the widget extension target (mobile/targets/widget/ForgeSessionActivity.swift)
// via the `_shared` folder convention @bacons/apple-targets documents.
import ActivityKit
import ExpoModulesCore

public class LiveActivityModule: Module {
    public func definition() -> ModuleDefinition {
        Name("LiveActivity")

        Function("isSupported") { () -> Bool in
            if #available(iOS 16.1, *) {
                return ActivityAuthorizationInfo().areActivitiesEnabled
            }
            return false
        }

        // Starts (or reuses, if one's already running for this session) a Live Activity, and
        // waits briefly for ActivityKit to hand back its first push token (needed for remote
        // updates while the app is backgrounded/locked). Returns `nil` for both fields if Live
        // Activities are disabled/unsupported, or the push token if a token never arrives within
        // the timeout (the activity still starts either way — local `.update()`/`.end()` calls
        // work regardless of whether a remote push token exists).
        AsyncFunction("start") {
            (sessionId: String, title: String, busy: Bool, waiting: Bool, costUsd: Double, contextTokens: Int) -> [String: String?] in
            guard #available(iOS 16.1, *), ActivityAuthorizationInfo().areActivitiesEnabled else {
                return ["activityId": nil, "pushToken": nil]
            }

            // Reuse an existing activity for this session rather than starting a duplicate —
            // the JS side calls `start` once per turn-begin, and a stale prior activity for the
            // same session (app relaunch mid-turn, etc.) should be adopted, not orphaned.
            if let existing = Activity<ForgeSessionActivityAttributes>.activities.first(where: {
                $0.attributes.sessionId == sessionId
            }) {
                return ["activityId": existing.id, "pushToken": nil]
            }

            let attributes = ForgeSessionActivityAttributes(sessionId: sessionId, title: title)
            let state = ForgeSessionActivityAttributes.ContentState(
                busy: busy, waiting: waiting, costUsd: costUsd, contextTokens: contextTokens
            )

            do {
                let activity = try Activity.request(
                    attributes: attributes,
                    content: .init(state: state, staleDate: nil),
                    pushType: .token
                )
                let token = await Self.firstPushToken(of: activity)
                return ["activityId": activity.id, "pushToken": token]
            } catch {
                return ["activityId": nil, "pushToken": nil]
            }
        }

        AsyncFunction("update") {
            (activityId: String, busy: Bool, waiting: Bool, costUsd: Double, contextTokens: Int) in
            guard #available(iOS 16.1, *) else { return }
            guard let activity = Activity<ForgeSessionActivityAttributes>.activities.first(where: { $0.id == activityId }) else {
                return
            }
            let state = ForgeSessionActivityAttributes.ContentState(
                busy: busy, waiting: waiting, costUsd: costUsd, contextTokens: contextTokens
            )
            await activity.update(.init(state: state, staleDate: nil))
        }

        AsyncFunction("end") { (activityId: String) in
            guard #available(iOS 16.1, *) else { return }
            guard let activity = Activity<ForgeSessionActivityAttributes>.activities.first(where: { $0.id == activityId }) else {
                return
            }
            await activity.end(nil, dismissalPolicy: .immediate)
        }
    }

    @available(iOS 16.1, *)
    private static func firstPushToken(of activity: Activity<ForgeSessionActivityAttributes>) async -> String? {
        // ActivityKit hands the push token back asynchronously, usually within a couple of
        // seconds — bounded wait so `start()` never hangs the JS caller if one never arrives
        // (e.g. simulator, or the user just denied notification permission).
        let timeout = Task<String?, Never> {
            try? await Task.sleep(nanoseconds: 5_000_000_000)
            return nil
        }
        let wait = Task<String?, Never> {
            for await data in activity.pushTokenUpdates {
                return data.map { String(format: "%02x", $0) }.joined()
            }
            return nil
        }
        defer { timeout.cancel(); wait.cancel() }
        return await withTaskGroup(of: String?.self) { group in
            group.addTask { await wait.value }
            group.addTask { await timeout.value }
            let first = await group.next() ?? nil
            group.cancelAll()
            return first
        }
    }
}
