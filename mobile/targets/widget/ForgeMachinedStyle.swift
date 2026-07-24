// Machined palette + marks for this WidgetKit extension (Live Activity, Dynamic Island and the
// Home Screen widget all live in this one target — see ForgeWidgetBundle.swift). Source of
// truth: docs/design/machined/"Forge Machined - Mobile.dc.html" L610-627 ("M Native Surfaces")
// and INVENTORY.md §05 "Native iOS".
//
// The hex values mirror `darkTokens` in mobile/src/theme/tokens.ts — there is no way to share
// the TS token file with a native target, so they are kept as the literal values from there,
// same hand-sync caveat as ForgeSharedData.swift's wire contract. Two extra values come from
// the design frame itself rather than the app tokens: `panel` (#101015, the native surface
// fill, = tokens' bg3) and `inkBody` (#DCDCE2, the body copy ink used on native cards).
//
// Machined supersedes Emberline/Hearth: the thermal identity is retired. No gradients, no heat
// edges, no glows, no shadows — flat fills, hairline borders, one ember accent. The radii these
// surfaces use (7pt Live Activity buttons, 14/17/22pt activity card + Dynamic Island, 20pt
// widget tile) are the design's deliberate native-platform exception to the app's 3-4pt radius.
//
// TYPEFACE: the app bundles Geist / Geist Mono via the expo-font config plugin, but that plugin
// registers the faces in the MAIN app bundle's UIAppFonts only. A widget extension is a separate
// bundle and cannot see them without adding the .ttf files to this target's resources — a target
// -config change that can't be validated without a real iOS build. So this target stays on San
// Francisco, using `design: .monospaced` (SF Mono) everywhere the design specifies Geist Mono.
import SwiftUI

enum ForgeMachined {
    static let bg = Color(hex: 0x09090B)
    /// Native surface fill (design's #101015 card / widget tile; tokens' `bg3`).
    static let panel = Color(hex: 0x101015)
    static let panelDeep = Color(hex: 0x0D0D11)

    static let ink = Color(hex: 0xF4F4F6)
    /// Body copy on a native card — the design's #DCDCE2, one step under `ink`.
    static let inkBody = Color(hex: 0xDCDCE2)
    static let ink2 = Color(hex: 0x9A9AA6)
    static let ink3 = Color(hex: 0x5F5F6B)

    static let accent = Color(hex: 0xFF8A3D)
    static let onAccent = Color(hex: 0x1A0E04)
    static let success = Color(hex: 0x5FB97D)
    static let danger = Color(hex: 0xE5605C)
    static let warn = Color(hex: 0xD9A94E)

    static let hairline = Color(hex: 0xF4F4F6, opacity: 0.07)
    static let borderSoft = Color(hex: 0xF4F4F6, opacity: 0.08)
    static let border = Color(hex: 0xF4F4F6, opacity: 0.10)
    static let borderStrong = Color(hex: 0xF4F4F6, opacity: 0.14)

    /// Technical/status text (counters, percentages, money, timers). See the TYPEFACE note above
    /// for why this is SF Mono rather than the app's Geist Mono.
    static func mono(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight, design: .monospaced)
    }
}

extension Color {
    init(hex: UInt32, opacity: Double = 1) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: opacity
        )
    }
}

/// The Machined spark — the four-point star in every frame's header. This is the design's own
/// SVG path (`M12 2l2.4 7.6L22 12l-7.6 2.4L12 22l-2.4-7.6L2 12l7.6-2.4z`) on its native 24×24
/// viewBox, rescaled to whatever frame it's given, so the native surfaces carry exactly the same
/// mark as the app. Drawn rather than an SF Symbol because no symbol matches this silhouette.
struct ForgeSparkShape: Shape {
    func path(in rect: CGRect) -> Path {
        let unit = min(rect.width, rect.height) / 24
        func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
            CGPoint(x: rect.minX + x * unit, y: rect.minY + y * unit)
        }
        var path = Path()
        path.move(to: point(12, 2))
        path.addLine(to: point(14.4, 9.6))
        path.addLine(to: point(22, 12))
        path.addLine(to: point(14.4, 14.4))
        path.addLine(to: point(12, 22))
        path.addLine(to: point(9.6, 14.4))
        path.addLine(to: point(2, 12))
        path.addLine(to: point(9.6, 9.6))
        path.closeSubpath()
        return path
    }
}

struct ForgeSparkMark: View {
    var size: CGFloat = 12
    var color: Color = ForgeMachined.accent

    var body: some View {
        ForgeSparkShape()
            .fill(color)
            .frame(width: size, height: size)
    }
}

/// Flat state dot. Machined retires the Emberline "emberdot pulse" and its halo — a dot is a
/// dot; the color alone carries the state (danger = needs you, ember = forging, ok = idle).
struct ForgeStatusDot: View {
    let color: Color
    var size: CGFloat = 5

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: size, height: size)
    }
}
