// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// A localised string from `Localizable.strings`, formatted with `args`.
func tr(_ key: String, _ args: CVarArg...) -> String {
    let format = NSLocalizedString(key, comment: "")
    return args.isEmpty ? format : String(format: format, arguments: args)
}

/// A plural string from `Localizable.stringsdict`, with the plural rules
/// of the language (one, few, many, other ...) applied to `n`.
func trp(_ key: String, _ n: Int) -> String {
    String.localizedStringWithFormat(NSLocalizedString(key, comment: ""), n)
}

/// The locale for dates and durations: the app's language (the system
/// language when the app has it, else English) with the user's region.
let appLocale: Locale = {
    let lang = Bundle.main.preferredLocalizations.first ?? "en"
    guard !lang.contains("-"), let region = Locale.current.regionCode else { return Locale(identifier: lang) }
    return Locale(identifier: "\(lang)_\(region)")
}()

/// A duration such as "4 min" or "1 hr, 20 min" in the user's language;
/// under a minute "less than a minute" (FR-SL-3).
func durationText(ms: UInt64, locale: Locale = appLocale) -> String {
    let mins = ms / 60_000
    guard mins >= 1 else { return tr("less_than_minute") }
    let f = DateComponentsFormatter()
    var cal = Calendar.current
    cal.locale = locale
    f.calendar = cal
    f.unitsStyle = .short
    f.allowedUnits = mins >= 24 * 60 ? [.day, .hour] : [.hour, .minute]
    f.maximumUnitCount = 2
    return f.string(from: TimeInterval(mins * 60)) ?? "\(mins)"
}

/// A short duration choice such as "30 sec" or "2 min" (alarm delay).
func shortDurationText(seconds: Int, locale: Locale = appLocale) -> String {
    let f = DateComponentsFormatter()
    var cal = Calendar.current
    cal.locale = locale
    f.calendar = cal
    f.unitsStyle = .short
    f.allowedUnits = seconds % 60 == 0 ? [.minute] : [.second]
    return f.string(from: TimeInterval(seconds)) ?? "\(seconds)"
}

/// A clock time such as "03:40" or "3:40 AM", as the locale writes it.
func timeText(unixMs: UInt64, locale: Locale = appLocale) -> String {
    let f = DateFormatter()
    f.locale = locale
    f.dateStyle = .none
    f.timeStyle = .short
    return f.string(from: Date(timeIntervalSince1970: TimeInterval(unixMs) / 1000))
}

/// A night's date such as "Sat 21 Nov" or "Sat, Nov 21".
func nightDateText(_ date: Date, locale: Locale = appLocale) -> String {
    let f = DateFormatter()
    f.locale = locale
    f.setLocalizedDateFormatFromTemplate("EEEdMMM")
    return f.string(from: date)
}

/// Unix milliseconds of now.
func nowUnixMs() -> UInt64 { UInt64(Date().timeIntervalSince1970 * 1000) }

/// `m:ss` for countdowns.
func clockText(ms: Int64) -> String {
    let s = max(ms / 1000, 0)
    return String(format: "%d:%02d", s / 60, s % 60)
}

/// "just now", "5 min ago", ...
func agoText(unixMs: UInt64, nowMs: UInt64 = nowUnixMs()) -> String {
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
