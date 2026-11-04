// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// A localised string from `Localizable.strings`, formatted with `args`.
func tr(_ key: String, _ args: CVarArg...) -> String {
    let format = NSLocalizedString(key, comment: "")
    return args.isEmpty ? format : String(format: format, arguments: args)
}

/// A plural string: `<key>.one` for 1, `<key>.other` otherwise (English
/// rules; other languages add their own forms with a `.stringsdict`).
func trp(_ key: String, _ n: Int, _ args: CVarArg...) -> String {
    let format = NSLocalizedString(n == 1 ? "\(key).one" : "\(key).other", comment: "")
    return String(format: format, arguments: args.isEmpty ? [n] : args)
}

/// `m:ss` for countdowns.
func clockText(ms: Int64) -> String {
    let s = max(ms / 1000, 0)
    return String(format: "%d:%02d", s / 60, s % 60)
}

/// "just now", "5 min ago", ...
func agoText(unixMs: UInt64, nowMs: UInt64 = UInt64(Date().timeIntervalSince1970 * 1000)) -> String {
    let mins = Int(nowMs > unixMs ? (nowMs - unixMs) / 60_000 : 0)
    switch mins {
    case ..<1: return tr("ago_now")
    case ..<60: return tr("ago_minutes", mins)
    case ..<(24 * 60): return tr("ago_hours", mins / 60)
    case ..<(48 * 60): return tr("ago_yesterday")
    default: return trp("ago_days", mins / (24 * 60))
    }
}

/// The invitation inside a URL the app was opened with (`nestwatch://invite?…`,
/// FR-PAIR-1); nil for other URLs.
func invitation(from url: URL) -> String? {
    guard url.scheme?.lowercased() == "nestwatch", url.host?.lowercased() == "invite" else { return nil }
    return url.absoluteString
}
