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
// registers the faces in the MAIN app bundle's UIAppFonts only — a widget extension is a separate
// bundle and cannot see them. So this target carries its own copies: the five .ttf files sitting
// beside this source file are swept into the extension's Copy Bundle Resources phase by the
// target's PBXFileSystemSynchronizedRootGroup (they are not listed in its membershipExceptions),
// and Info.plist here declares them in UIAppFonts. `ForgeTypeface` then resolves them by exact
// PostScript name — falling back to San Francisco if, for any reason, a face fails to register.
// See ForgeTypeface below for the fallback contract.
import SwiftUI
import UIKit

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

    /// Body / label text — Geist.
    static func sans(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        ForgeTypeface.resolve(
            ForgeTypeface.sansFace(for: weight),
            available: ForgeTypeface.sansAvailable,
            size: size,
            weight: weight
        )
    }

    /// Technical/status text (counters, percentages, money, timers) — Geist Mono, per the design's
    /// rule that everything numeric or machine-ish is set in the mono face.
    static func mono(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        ForgeTypeface.resolve(
            ForgeTypeface.monoFace(for: weight),
            available: ForgeTypeface.monoAvailable,
            size: size,
            weight: weight
        )
    }
}

/// Resolves the bundled Geist faces, with San Francisco as a hard fallback.
///
/// The strings below are PostScript names (name-table ID 6), read out of the actual .ttf files
/// rather than assumed from their filenames:
///   Geist-Regular · Geist-Medium · Geist-SemiBold · GeistMono-Regular · GeistMono-Medium
///
/// Only those five faces ship in this target (the app bundles Geist-Bold and GeistMono-SemiBold
/// too, but nothing here asks for them and an extension pays for every byte it embeds), so heavier
/// weights clamp down to the heaviest face present: sans .bold/.heavy/.black → Geist-SemiBold,
/// mono .semibold and up → GeistMono-Medium. No call site currently requests those.
///
/// FALLBACK: if a family fails to register — a resource that didn't get copied, a plist that lost
/// UIAppFonts — every helper returns `.system(size:weight:)` instead. A widget in SF is a cosmetic
/// regression; a widget that renders nothing is a bug report. `Font.custom` alone would also fall
/// back, but it would silently drop the requested weight, so the availability probe is explicit.
private enum ForgeTypeface {
    /// One probe per family; the faces of a family are registered together or not at all.
    /// `static let` gives a lazy, once-only, thread-safe evaluation.
    static let sansAvailable: Bool = UIFont(name: "Geist-Regular", size: 12) != nil
    static let monoAvailable: Bool = UIFont(name: "GeistMono-Regular", size: 12) != nil

    static func sansFace(for weight: Font.Weight) -> String {
        if weight == .medium { return "Geist-Medium" }
        if weight == .semibold || weight == .bold || weight == .heavy || weight == .black {
            return "Geist-SemiBold"
        }
        return "Geist-Regular"
    }

    static func monoFace(for weight: Font.Weight) -> String {
        if weight == .medium || weight == .semibold || weight == .bold
            || weight == .heavy || weight == .black
        {
            return "GeistMono-Medium"
        }
        return "GeistMono-Regular"
    }

    /// `fixedSize:` (not `size:`) deliberately: `Font.custom(_:size:)` scales with Dynamic Type
    /// whereas the `.system(size:)` calls this replaces did not, and these widget/Live Activity
    /// layouts are laid out to the point. `fixedSize:` keeps the previous metrics exactly.
    static func resolve(_ postScriptName: String, available: Bool, size: CGFloat, weight: Font.Weight) -> Font {
        guard available else { return .system(size: size, weight: weight) }
        return .custom(postScriptName, fixedSize: size)
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
