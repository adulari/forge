// Used by this widget extension target (ActivityConfiguration/Dynamic Island in
// ForgeSessionActivity.swift). NOT in `_shared/` — that convention only merges files into the
// main app target and this extension target, but mobile/modules/live-activity/ios's
// LiveActivityModule.swift compiles as its OWN separate CocoaPod (confirmed via a real EAS Build
// Xcode log: "cannot find type 'ForgeSessionActivityAttributes' in scope" in that pod, even with
// this file in `_shared/`). So that module keeps its own literal copy of this same struct
// instead — see mobile/modules/live-activity/ios/ForgeSessionActivityAttributes.swift, which
// must be changed in lockstep with this file.
//
// `ContentState`'s field names are a hand-kept-in-sync wire contract with
// crates/forge-cli/src/apns.rs's `LiveActivityContentState` — that Rust struct's own doc comment
// points back here too. Change one side, change all three.
//
// `baseUrl`/`agentLabel` live on `Attributes` (set once at `Activity.request()`, per session) —
// `baseUrl` already carries the daemon's auth token as a path segment (see
// mobile/src/lib/api.ts's `request()`), which is what lets `ForgeActivityIntents.swift`'s
// Allow/Deny buttons POST a decision straight from the widget extension without a separate
// credential store. `question`/`promptSeq`/`tasksDone`/`tasksTotal`/`stateSinceEpoch` on
// `ContentState` back the Hearth "NEEDS YOU" and forging cards — see mobile.dc.html's "Mobile
// Live Activity" / "Mobile Dynamic Island" screens.
import ActivityKit

struct ForgeSessionActivityAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        var busy: Bool
        var waiting: Bool
        var costUsd: Double
        var contextTokens: Int
        var contextLimit: Int
        var question: String?
        var promptSeq: Int?
        var tasksDone: Int?
        var tasksTotal: Int?
        var stateSinceEpoch: Double?

        enum CodingKeys: String, CodingKey {
            case busy, waiting, question
            case costUsd = "cost_usd"
            case contextTokens = "context_tokens"
            case contextLimit = "context_limit"
            case promptSeq = "prompt_seq"
            case tasksDone = "tasks_done"
            case tasksTotal = "tasks_total"
            case stateSinceEpoch = "state_since"
        }
    }

    var sessionId: String
    var title: String
    var baseUrl: String
    var agentLabel: String
}
