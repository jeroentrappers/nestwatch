// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI

/// A status pill: always a word and a dot, never colour alone (NFR-A11Y-1).
/// `.sound` pulses unless Reduce Motion is on.
struct Pill: View {
    let text: String
    let kind: PillKind
    @Environment(\.palette) private var c
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var pulse = false

    var body: some View {
        let (fg, bg) = colors
        HStack(spacing: 6) {
            Circle().fill(fg).frame(width: 8, height: 8)
                .opacity(kind == .sound && pulse ? 0.35 : 1)
            Text(text).font(NWFont.label).foregroundColor(fg).lineLimit(1)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .background(Capsule().fill(bg))
        .onAppear {
            guard kind == .sound, !reduceMotion else { return }
            withAnimation(.easeInOut(duration: 0.6).repeatForever(autoreverses: true)) { pulse = true }
        }
        .accessibilityElement(children: .combine)
    }

    private var colors: (Color, Color) {
        switch kind {
        case .calm: return (c.sage, c.sageSoft)
        case .sound: return (c.coral, c.coralSoft)
        case .warn: return (c.amber, c.amberSoft)
        case .voice: return (c.lavender, c.lavenderSoft)
        case .cam: return (c.honey, c.honeySoft)
        case .neutral: return (c.text2, c.ink2)
        }
    }
}

/// A card (`.card`); `alert` adds the coral halo while there is sound.
struct Card<Content: View>: View {
    var alert = false
    @ViewBuilder let content: () -> Content
    @Environment(\.palette) private var c

    var body: some View {
        VStack(alignment: .leading, spacing: 12, content: content)
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: Radius.large).fill(c.ink1))
            .overlay(
                RoundedRectangle(cornerRadius: Radius.large)
                    .strokeBorder(alert ? c.coral.opacity(0.7) : c.ink3, lineWidth: alert ? 2 : 1)
            )
    }
}

struct Caption: View {
    let text: String
    var color: Color?
    @Environment(\.palette) private var c

    init(_ text: String, color: Color? = nil) {
        self.text = text
        self.color = color
    }

    var body: some View {
        Text(text).font(NWFont.caption).foregroundColor(color ?? c.text3).fixedSize(horizontal: false, vertical: true)
    }
}

struct SectionTitle: View {
    let text: String
    @Environment(\.palette) private var c
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text).font(NWFont.subtitle).foregroundColor(c.text1).accessibilityAddTraits(.isHeader)
    }
}

/// The logo (design/logo.svg): a crescent moon cradling two sound waves.
struct Logo: View {
    var size: CGFloat = 48

    var body: some View {
        Canvas { ctx, sz in
            let k = sz.width / 64
            var moon = ctx
            moon.clip(to: Path(ellipseIn: CGRect(x: 20 * k, y: 4 * k, width: 40 * k, height: 40 * k)), options: .inverse)
            moon.fill(Path(ellipseIn: CGRect(x: 6 * k, y: 8 * k, width: 48 * k, height: 48 * k)), with: .color(Color(hex: 0xF4BD7E)))
            let stroke = StrokeStyle(lineWidth: 3.2 * k, lineCap: .round)
            let wave = Color(hex: 0xB3A6FF)
            var w1 = Path()
            w1.move(to: CGPoint(x: 40 * k, y: 38 * k))
            w1.addCurve(to: CGPoint(x: 40 * k, y: 27 * k), control1: CGPoint(x: 43.5 * k, y: 35.4 * k), control2: CGPoint(x: 43.5 * k, y: 29.6 * k))
            ctx.stroke(w1, with: .color(wave), style: stroke)
            var w2 = Path()
            w2.move(to: CGPoint(x: 46 * k, y: 43 * k))
            w2.addCurve(to: CGPoint(x: 46 * k, y: 21 * k), control1: CGPoint(x: 53 * k, y: 37.8 * k), control2: CGPoint(x: 53 * k, y: 26.2 * k))
            ctx.stroke(w2, with: .color(wave.opacity(0.6)), style: stroke)
        }
        .frame(width: size, height: size)
        .accessibilityLabel(Text(tr("app_name")))
    }
}

/// Primary action button (`.btn.primary`), at least 48 pt high.
struct PrimaryButton: View {
    let title: String
    var enabled = true
    let action: () -> Void
    @Environment(\.palette) private var c

    init(_ title: String, enabled: Bool = true, action: @escaping () -> Void) {
        self.title = title
        self.enabled = enabled
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Text(title).font(NWFont.subtitle).frame(maxWidth: .infinity, minHeight: 48)
                .foregroundColor(enabled ? c.onAccent : c.text3)
                .background(RoundedRectangle(cornerRadius: Radius.medium).fill(enabled ? c.honey : c.ink2))
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
    }
}

/// Secondary button (`.btn`).
struct SecondaryButton: View {
    let title: String
    var enabled = true
    let action: () -> Void
    @Environment(\.palette) private var c

    init(_ title: String, enabled: Bool = true, action: @escaping () -> Void) {
        self.title = title
        self.enabled = enabled
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Text(title).font(NWFont.body).lineLimit(1).minimumScaleFactor(0.8)
                .frame(maxWidth: .infinity, minHeight: 48)
                .padding(.horizontal, 8)
                .foregroundColor(enabled ? c.text1 : c.text3)
                .background(RoundedRectangle(cornerRadius: Radius.medium).fill(c.ink2))
                .overlay(RoundedRectangle(cornerRadius: Radius.medium).strokeBorder(c.ink3))
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
    }
}

/// Danger button (`.btn.danger`).
struct DangerButton: View {
    let title: String
    let action: () -> Void
    @Environment(\.palette) private var c

    init(_ title: String, action: @escaping () -> Void) {
        self.title = title
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Text(title).font(NWFont.body).lineLimit(1).minimumScaleFactor(0.8)
                .frame(maxWidth: .infinity, minHeight: 48)
                .padding(.horizontal, 8)
                .foregroundColor(c.coral)
                .background(RoundedRectangle(cornerRadius: Radius.medium).fill(c.coralSoft))
        }
        .buttonStyle(.plain)
    }
}

/// A text-only button in a colour.
struct LinkButton: View {
    let title: String
    var color: Color?
    let action: () -> Void
    @Environment(\.palette) private var c

    init(_ title: String, color: Color? = nil, action: @escaping () -> Void) {
        self.title = title
        self.color = color
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Text(title).font(NWFont.bodySmall).foregroundColor(color ?? c.text2).frame(minHeight: 44)
        }
        .buttonStyle(.plain)
    }
}

/// A header icon button with a spoken label (≥ 44 pt target).
struct IconButton: View {
    let symbol: String
    let label: String
    let action: () -> Void
    @Environment(\.palette) private var c

    init(_ symbol: String, label: String, action: @escaping () -> Void) {
        self.symbol = symbol
        self.label = label
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol).font(.system(size: 19)).foregroundColor(c.text2).frame(width: 44, height: 44)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(label))
    }
}

/// The station's level ring (`.ring`): the arc fills with the level, the
/// tick marks the strictest listener threshold (UI-2), the centre shows the
/// number and "quiet" or "sound".
struct LevelRing: View {
    let levelDb: Float
    let thresholdDb: Float?
    let sound: Bool
    var size: CGFloat = 220
    var showNumbers = true
    @Environment(\.palette) private var c

    var body: some View {
        let f = CGFloat(levelPosition(levelDb))
        let word = tr(sound ? "ring_sound" : "ring_quiet")
        let db = "\(Int(levelDb.isFinite ? levelDb.rounded() : -90))"
        ZStack {
            Canvas { ctx, sz in draw(ctx, sz, f) }
            .animation(.linear(duration: 0.16), value: f)
            if showNumbers {
                VStack(spacing: 2) {
                    Text(db).font(NWFont.display).foregroundColor(c.text1).monospacedDigit()
                    Text("\(tr("db")) · \(word)").font(NWFont.caption).foregroundColor(sound ? c.coral : c.text2)
                }
            }
        }
        .frame(width: size, height: size)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text("\(tr("ring_label")): \(db) \(tr("db")), \(word)"))
    }

    private func draw(_ ctx: GraphicsContext, _ sz: CGSize, _ f: CGFloat) {
        let stroke: CGFloat = sz.width * 0.07
        let r: CGFloat = sz.width / 2 - stroke / 2 - 4
        let centre = CGPoint(x: sz.width / 2, y: sz.height / 2)
        func arc(_ to: CGFloat) -> Path {
            var p = Path()
            let end: Double = 135 + 270 * Double(to)
            p.addArc(center: centre, radius: r, startAngle: .degrees(135), endAngle: .degrees(end), clockwise: false)
            return p
        }
        let style = StrokeStyle(lineWidth: stroke, lineCap: .round)
        ctx.stroke(arc(1), with: .color(c.ink3), style: style)
        if f > 0.001 { ctx.stroke(arc(f), with: .color(sound ? c.coral : c.sage), style: style) }
        guard let t = thresholdDb else { return }
        let a: Double = (135 + 270 * Double(levelPosition(t))) * .pi / 180
        let dx = CGFloat(cos(a)), dy = CGFloat(sin(a))
        var tick = Path()
        tick.move(to: CGPoint(x: centre.x + (r - stroke) * dx, y: centre.y + (r - stroke) * dy))
        tick.addLine(to: CGPoint(x: centre.x + (r + stroke) * dx, y: centre.y + (r + stroke) * dy))
        ctx.stroke(tick, with: .color(c.text1), style: StrokeStyle(lineWidth: 4, lineCap: .round))
    }
}

/// A listener tile's level meter (`.meter`) with this listener's threshold mark.
struct LevelMeter: View {
    let levelDb: Float?
    let thresholdDb: Float?
    let sound: Bool
    @Environment(\.palette) private var c

    var body: some View {
        Canvas { ctx, sz in
            let r = sz.height / 2
            ctx.fill(Path(roundedRect: CGRect(origin: .zero, size: sz), cornerRadius: r), with: .color(c.ink3))
            if let l = levelDb {
                let w = sz.width * CGFloat(levelPosition(l))
                ctx.fill(Path(roundedRect: CGRect(x: 0, y: 0, width: w, height: sz.height), cornerRadius: r), with: .color(sound ? c.coral : c.sage))
            }
            if let t = thresholdDb {
                let x = sz.width * CGFloat(levelPosition(t))
                var p = Path()
                p.move(to: CGPoint(x: x, y: -2))
                p.addLine(to: CGPoint(x: x, y: sz.height + 2))
                ctx.stroke(p, with: .color(c.text1), lineWidth: 3)
            }
        }
        .frame(height: 12)
    }
}

/// A QR code drawn from the core's module matrix with a light quiet zone.
struct QrImage: View {
    let qr: QrMatrix
    let label: String

    var body: some View {
        Canvas { ctx, sz in
            let n = Int(qr.size)
            guard n > 0 else { return }
            let cell = min(sz.width, sz.height) / CGFloat(n)
            let m = [UInt8](qr.modules)
            var p = Path()
            for y in 0..<n {
                var x = 0
                while x < n {
                    if m[y * n + x] == 1 {
                        let start = x
                        while x < n && m[y * n + x] == 1 { x += 1 }
                        p.addRect(CGRect(x: CGFloat(start) * cell, y: CGFloat(y) * cell, width: CGFloat(x - start) * cell + 0.5, height: cell + 0.5))
                    } else {
                        x += 1
                    }
                }
            }
            ctx.fill(p, with: .color(Color(hex: 0x111111)))
        }
        .aspectRatio(1, contentMode: .fit)
        .padding(12)
        .background(RoundedRectangle(cornerRadius: Radius.medium).fill(Color.white))
        .accessibilityLabel(Text(label))
    }
}

/// The calibration chart (`.calibrate`): the last minute of room level with
/// a threshold line the user drags (FR-AU-2, UI-5). The slider below it
/// offers the same control to VoiceOver.
struct CalibrationChart: View {
    let points: [(x: Float, db: Float)]
    let thresholdDb: Float
    let activated: Bool
    let onThreshold: (Float) -> Void
    @Environment(\.palette) private var c

    var body: some View {
        GeometryReader { geo in
            Canvas { ctx, sz in
                let h = sz.height, w = sz.width
                func y(_ db: Float) -> CGFloat { h * CGFloat(1 - levelPosition(db)) }
                let ty = y(thresholdDb)
                if activated { ctx.fill(Path(CGRect(x: 0, y: 0, width: w, height: ty)), with: .color(c.coralSoft)) }
                if points.count >= 2 {
                    var p = Path()
                    for (i, pt) in points.enumerated() {
                        let q = CGPoint(x: CGFloat(pt.x) * w, y: y(pt.db))
                        if i == 0 { p.move(to: q) } else { p.addLine(to: q) }
                    }
                    ctx.stroke(p, with: .color(c.sage), style: StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
                }
                var line = Path()
                line.move(to: CGPoint(x: 0, y: ty))
                line.addLine(to: CGPoint(x: w, y: ty))
                ctx.stroke(line, with: .color(c.honey), style: StrokeStyle(lineWidth: 2.5, dash: activated ? [] : [6, 5]))
                ctx.fill(Path(ellipseIn: CGRect(x: w - 22, y: ty - 6, width: 12, height: 12)), with: .color(c.honey))
            }
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { v in
                onThreshold(positionToDb(Float(1 - v.location.y / geo.size.height)))
            })
        }
        .frame(height: 140)
        .background(RoundedRectangle(cornerRadius: Radius.medium).fill(c.ink2))
        .clipShape(RoundedRectangle(cornerRadius: Radius.medium))
        .accessibilityHidden(true)
    }
}

/// Hold to talk (`.btn.talk`): the largest control on a tile. Pressing
/// requests talk-back, releasing ends it; VoiceOver toggles it.
struct HoldToTalk: View {
    let active: Bool
    let enabled: Bool
    let label: String
    let onTalk: (Bool) -> Void
    @Environment(\.palette) private var c
    @State private var pressed = false

    var body: some View {
        let bg = !enabled ? c.ink2 : (active ? c.lavender : c.lavenderSoft)
        let fg = !enabled ? c.text3 : (active ? c.onAccent : c.lavender)
        Text(label).font(NWFont.subtitle).foregroundColor(fg)
            .frame(maxWidth: .infinity, minHeight: 64)
            .background(RoundedRectangle(cornerRadius: Radius.medium).fill(bg))
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { _ in
                        guard enabled, !pressed else { return }
                        pressed = true
                        onTalk(true)
                    }
                    .onEnded { _ in
                        guard pressed else { return }
                        pressed = false
                        onTalk(false)
                    }
            )
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Text(label))
            .accessibilityValue(Text(tr(active ? "talk_state_on" : "talk_state_off")))
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { if enabled { onTalk(!active) } }
    }
}

/// A rounded square with a mono value, e.g. the verification number.
struct CodeBox: View {
    let text: String
    var size: CGFloat = 20
    @Environment(\.palette) private var c

    var body: some View {
        Text(text).font(NWFont.mono(size)).tracking(3).foregroundColor(c.text1)
            .padding(.horizontal, 16).padding(.vertical, 12)
            .background(RoundedRectangle(cornerRadius: Radius.medium).fill(c.ink2))
            .textSelection(.enabled)
    }
}

/// A labelled switch row (≥ 44 pt target).
struct SwitchRow: View {
    let label: String
    let isOn: Bool
    var enabled = true
    let onChange: (Bool) -> Void
    @Environment(\.palette) private var c

    var body: some View {
        Toggle(isOn: Binding(get: { isOn }, set: onChange)) {
            Text(label).font(NWFont.body).foregroundColor(c.text1)
        }
        .toggleStyle(SwitchToggleStyle(tint: c.lavender))
        .disabled(!enabled)
        .frame(minHeight: 44)
    }
}

/// A segmented control (`.segmented`).
struct Segmented<T: Hashable>: View {
    let options: [(T, String)]
    let selected: T
    let onSelect: (T) -> Void
    @Environment(\.palette) private var c

    var body: some View {
        HStack(spacing: 0) {
            ForEach(Array(options.enumerated()), id: \.offset) { i, o in
                let on = o.0 == selected
                Button { onSelect(o.0) } label: {
                    Text(o.1).font(NWFont.bodySmall).lineLimit(1).minimumScaleFactor(0.7)
                        .foregroundColor(on ? c.text1 : c.text2)
                        .frame(maxWidth: .infinity, minHeight: 40)
                        .background(on ? c.lavenderSoft : Color.clear)
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(on ? .isSelected : [])
                if i < options.count - 1 { Rectangle().fill(c.ink3).frame(width: 1) }
            }
        }
        .background(c.ink2)
        .clipShape(RoundedRectangle(cornerRadius: Radius.small))
        .overlay(RoundedRectangle(cornerRadius: Radius.small).strokeBorder(c.ink3))
    }
}

/// A text field in the Nestwatch style with a label and a hint line.
struct Field: View {
    let label: String
    @Binding var text: String
    var hint: String?
    var error = false
    var mono = false
    var keyboard: UIKeyboardType = .default
    var capitalization: TextInputAutocapitalization = .never
    var onSubmit: () -> Void = {}
    @Environment(\.palette) private var c

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(label).font(NWFont.caption).foregroundColor(c.text2)
            TextField("", text: $text)
                .font(mono ? NWFont.mono(17) : NWFont.body)
                .foregroundColor(c.text1)
                .keyboardType(keyboard)
                .textInputAutocapitalization(capitalization)
                .disableAutocorrection(true)
                .submitLabel(.done)
                .onSubmit(onSubmit)
                .padding(.horizontal, 12)
                .frame(minHeight: 44)
                .background(RoundedRectangle(cornerRadius: Radius.small).fill(c.ink2))
                .overlay(RoundedRectangle(cornerRadius: Radius.small).strokeBorder(error ? c.coral : c.ink3))
                .accessibilityLabel(Text(label))
            if let hint {
                Text(hint).font(NWFont.caption).foregroundColor(error ? c.coral : c.text3).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// Pills that wrap onto more rows when the width runs out.
struct PillRow: View {
    let pills: [(String, PillKind)]

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 140), spacing: 8, alignment: .leading)], alignment: .leading, spacing: 8) {
            ForEach(Array(pills.enumerated()), id: \.offset) { _, p in Pill(text: p.0, kind: p.1) }
        }
    }
}
