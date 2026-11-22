// SPDX-License-Identifier: AGPL-3.0-or-later
import Combine
import SwiftUI

/// The night log of a station on this listener (FR-SL-4): wake-ups grouped
/// by night (noon to noon, newest night first), with the number of
/// wake-ups, the longest quiet stretch and each wake-up's time and length.
/// Re-read when the tile's `log_revision` changes and once a minute.
struct NightLogScreen: View {
    let id: String
    @EnvironmentObject private var model: Nestwatch
    @Environment(\.palette) private var c
    @State private var entries: [WakeUpEntry] = []
    @State private var now = nowUnixMs()
    @State private var confirmClear = false
    private let tick = Timer.publish(every: 60, on: .main, in: .common).autoconnect()

    var body: some View {
        let tile = model.listenerState?.stations.first { $0.id == id }
        let name = tile.map { $0.name.isEmpty ? tr("unnamed") : $0.name } ?? tr("unnamed")
        let nights = nightGroups(entries, nowMs: now)
        Page(tr("night_log")) {
            Caption(name)
            if nights.isEmpty {
                Card { Text(tr("night_log_empty")).font(NWFont.body).foregroundColor(c.text2).fixedSize(horizontal: false, vertical: true) }
            }
            ForEach(nights, id: \.start) { night in NightCard(night: night, now: now) }
            Caption(tr("night_log_footer"))
            if !entries.isEmpty {
                DangerButton(tr("clear_log")) { confirmClear = true }
            }
        }
        .onAppear(perform: reload)
        .onChange(of: tile?.logRevision) { _ in reload() }
        .onReceive(tick) { _ in reload() }
        .alert(tr("clear_log_confirm", name), isPresented: $confirmClear) {
            Button(tr("clear"), role: .destructive) {
                model.withListener { try $0.clearNightLog(stationId: id) }
                reload()
            }
            Button(tr("cancel"), role: .cancel) {}
        }
    }

    private func reload() {
        now = nowUnixMs()
        #if DEBUG
        if let sample = debugNightLog() {
            entries = sample
            return
        }
        #endif
        guard let l = model.listenerEngine else { return }
        entries = (try? l.nightLog(stationId: id)) ?? entries
    }
}

private struct NightCard: View {
    let night: NightGroup
    let now: UInt64
    @Environment(\.palette) private var c

    var body: some View {
        Card {
            SectionTitle(tr("night_of", nightDateText(night.start)))
            Caption(summary)
            ForEach(Array(night.entries.reversed().enumerated()), id: \.offset) { _, e in
                EntryRow(e: e, now: now)
            }
        }
    }

    private var summary: String {
        var bits = [trp("wake_ups", night.entries.count)]
        if let q = night.longestQuietMs { bits.append(tr("longest_quiet", durationText(ms: q))) }
        return bits.joined(separator: " · ")
    }
}

private struct EntryRow: View {
    let e: WakeUpEntry
    let now: UInt64
    @Environment(\.palette) private var c

    var body: some View {
        let ongoing = e.endUnixMs == nil
        HStack(spacing: 8) {
            Text(line(ongoing)).font(NWFont.body).foregroundColor(e.notTheBaby ? c.text3 : c.text1)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 8)
            if e.alarmed { Pill(text: tr("badge_alarm"), kind: .sound) }
            if e.notTheBaby { Pill(text: tr("not_the_baby"), kind: .neutral).opacity(0.75) }
        }
        .frame(minHeight: 32)
        .accessibilityElement(children: .combine)
    }

    private func line(_ ongoing: Bool) -> String {
        var bits = [timeText(unixMs: e.startUnixMs), durationText(ms: wakeUpDurationMs(e, nowMs: now))]
        if ongoing { bits.append(tr("going_on")) }
        return bits.joined(separator: " · ")
    }
}

#if DEBUG
/// `NESTWATCH_DEBUG_NIGHT_LOG=1`: sample wake-ups for screenshots of the
/// night log without a station (debug builds only).
func debugNightLog() -> [WakeUpEntry]? {
    guard ProcessInfo.processInfo.environment["NESTWATCH_DEBUG_NIGHT_LOG"] != nil else { return nil }
    let cal = Calendar.current
    let today = cal.startOfDay(for: Date())
    func at(_ dayOffset: Int, _ h: Int, _ m: Int) -> UInt64 {
        let d = cal.date(byAdding: .day, value: dayOffset, to: today)!
        return UInt64(cal.date(bySettingHour: h, minute: m, second: 0, of: d)!.timeIntervalSince1970 * 1000)
    }
    func w(_ start: UInt64, mins: UInt64, alarmed: Bool = false, notBaby: Bool = false) -> WakeUpEntry {
        WakeUpEntry(startUnixMs: start, endUnixMs: start + mins * 60_000, loudMs: mins * 30_000, peakDbfs: -32,
                    alerted: true, alarmed: alarmed, notTheBaby: notBaby)
    }
    return [
        w(at(-2, 22, 40), mins: 4), w(at(-1, 2, 14), mins: 6, alarmed: true), w(at(-1, 5, 50), mins: 12),
        w(at(-1, 21, 5), mins: 2, notBaby: true), w(at(0, 1, 30), mins: 9), w(at(0, 4, 2), mins: 3, alarmed: true),
    ]
}
#endif
