// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI

/// The Nestwatch tokens (design/nestwatch.css, design/README.md §3, §6):
/// `--ink-0..3`, text levels, honey (accent), lavender, sage, coral and
/// amber, for the night and day themes and the station's dimmed mode.
struct Palette: Equatable {
    var ink0: Color
    var ink1: Color
    var ink2: Color
    var ink3: Color
    var text1: Color
    var text2: Color
    var text3: Color
    var honey: Color
    var lavender: Color
    var sage: Color
    var coral: Color
    var amber: Color
    var dark: Bool

    var honeySoft: Color { honey.opacity(dark ? 0.14 : 0.12) }
    var lavenderSoft: Color { lavender.opacity(dark ? 0.16 : 0.12) }
    var sageSoft: Color { sage.opacity(dark ? 0.14 : 0.12) }
    var coralSoft: Color { coral.opacity(dark ? 0.16 : 0.12) }
    var amberSoft: Color { amber.opacity(dark ? 0.14 : 0.12) }
    /// Text on a filled honey or lavender button.
    var onAccent: Color { dark ? ink0 : .white }

    static let night = Palette(
        ink0: Color(hex: 0x0F1017), ink1: Color(hex: 0x171923), ink2: Color(hex: 0x212433), ink3: Color(hex: 0x2D3145),
        text1: Color(hex: 0xF3EFE8), text2: Color(hex: 0xB9B4C8), text3: Color(hex: 0x8A86A0),
        honey: Color(hex: 0xF4BD7E), lavender: Color(hex: 0xB3A6FF), sage: Color(hex: 0x93D4B4),
        coral: Color(hex: 0xFF8F80), amber: Color(hex: 0xFFD36B), dark: true
    )

    static let day = Palette(
        ink0: Color(hex: 0xFAF6EF), ink1: Color(hex: 0xFFFFFF), ink2: Color(hex: 0xF1ECE3), ink3: Color(hex: 0xE2DBCF),
        text1: Color(hex: 0x22202B), text2: Color(hex: 0x57536A), text3: Color(hex: 0x7D7890),
        honey: Color(hex: 0xB8681C), lavender: Color(hex: 0x5B4BD1), sage: Color(hex: 0x2F8A62),
        coral: Color(hex: 0xD0473A), amber: Color(hex: 0x9A6A00), dark: false
    )

    /// Station night mode (`.dimmed`): near-black with desaturated warm text.
    static let dimmed: Palette = {
        var p = night
        p.ink0 = Color(hex: 0x000000); p.ink1 = Color(hex: 0x07070A); p.ink2 = Color(hex: 0x0D0D12); p.ink3 = Color(hex: 0x16161D)
        p.text1 = Color(hex: 0x6D5A44); p.text2 = Color(hex: 0x4E4234); p.text3 = Color(hex: 0x3A3128)
        p.honey = Color(hex: 0x7A5A36); p.sage = Color(hex: 0x3F5E4F); p.coral = Color(hex: 0x7C4A43)
        p.lavender = Color(hex: 0x4F4A7A); p.amber = Color(hex: 0x7A5A36)
        return p
    }()

    /// The palette for a theme choice and the system appearance.
    static func resolve(_ choice: ThemeChoice, system: ColorScheme, dimmed: Bool) -> Palette {
        if dimmed { return .dimmed }
        switch choice {
        case .night: return .night
        case .day: return .day
        case .system: return system == .dark ? .night : .day
        }
    }
}

extension Color {
    init(hex: UInt32) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: 1
        )
    }
}

private struct PaletteKey: EnvironmentKey {
    static let defaultValue = Palette.night
}

extension EnvironmentValues {
    var palette: Palette {
        get { self[PaletteKey.self] }
        set { self[PaletteKey.self] = newValue }
    }
}

/// Type scale 12 / 14 / 16 / 20 / 28 / 44 with the rounded system font
/// (design/README.md §3.2, §6).
enum NWFont {
    static let display = Font.system(size: 44, weight: .semibold, design: .rounded)
    static let headline = Font.system(size: 28, weight: .semibold, design: .rounded)
    static let title = Font.system(size: 20, weight: .semibold, design: .rounded)
    static let subtitle = Font.system(size: 16, weight: .semibold, design: .rounded)
    static let body = Font.system(size: 16, design: .rounded)
    static let bodySmall = Font.system(size: 14, design: .rounded)
    static let caption = Font.system(size: 12, design: .rounded)
    static let label = Font.system(size: 12, weight: .medium, design: .rounded)
    /// Monospace for codes and numbers, used with wide tracking.
    static func mono(_ size: CGFloat = 16) -> Font { .system(size: size, weight: .semibold, design: .monospaced) }
}

/// Radii 10 / 16 / 24 (design/README.md §3.3).
enum Radius {
    static let small: CGFloat = 10
    static let medium: CGFloat = 16
    static let large: CGFloat = 24
}
