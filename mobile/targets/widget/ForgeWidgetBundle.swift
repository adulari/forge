// Entry point for the WidgetKit extension — both the Home Screen widget and the Live Activity
// live in this one extension bundle (Apple's own convention, not a Forge choice: a Live
// Activity is declared as a `Widget` alongside ordinary widgets, there's no separate target
// type for it).
import SwiftUI
import WidgetKit

@main
struct ForgeWidgetBundle: WidgetBundle {
    var body: some Widget {
        ForgeSessionsWidget()
        if #available(iOS 16.1, *) {
            ForgeSessionActivityWidget()
        }
    }
}
