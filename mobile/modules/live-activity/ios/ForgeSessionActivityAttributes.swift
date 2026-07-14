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
import ActivityKit

struct ForgeSessionActivityAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        var busy: Bool
        var waiting: Bool
        var costUsd: Double
        var contextTokens: Int

        enum CodingKeys: String, CodingKey {
            case busy, waiting
            case costUsd = "cost_usd"
            case contextTokens = "context_tokens"
        }
    }

    var sessionId: String
    var title: String
}
