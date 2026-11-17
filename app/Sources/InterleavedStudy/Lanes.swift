// The interleave line: each course is a lane, and the study order is one thread that steps between lanes. Used on
// Today (the order coming up), in a session (what was done, what is next) and on the Study start page. Drawn from
// data; nothing animates. See DESIGN.md, "Signature motif".
import SwiftUI

enum LaneState { case done, skipped, current, next, later }

struct LaneItem: Identifiable {
    let id: String
    let courseId: String
    var state: LaneState = .later
    var name: String
    var deck: Doc? = nil
    var meta: String = ""
    var aside: String = ""
    var help: String = ""
    var action: (() -> Void)? = nil
    var disabled = false
}

struct Lanes: View {
    @Environment(AppModel.self) private var app
    let items: [LaneItem]
    var label: String
    var legend = true
    /// The ground under the dots, so their halo matches what they sit on.
    var ground: Color = Ink.paper
    var rowHeight: CGFloat = 48

    private let step: CGFloat = 14, pad: CGFloat = 7

    /// The courses the line passes through, in the library's course order.
    private var lanes: [String] {
        let used = Set(items.map(\.courseId))
        let known = (app.data?.courses ?? []).map(\.id).filter(used.contains)
        var seen = Set(known)
        return known + items.map(\.courseId).filter { seen.insert($0).inserted }
    }

    var body: some View {
        let lanes = lanes
        let x = { (cid: String) in pad + CGFloat(max(0, lanes.firstIndex(of: cid) ?? 0)) * step }
        let width = pad * 2 + CGFloat(max(0, lanes.count - 1)) * step
        VStack(alignment: .leading, spacing: 10) {
            if legend && lanes.count > 1 {
                HStack(spacing: 14) {
                    ForEach(lanes, id: \.self) { id in
                        HStack(spacing: 6) { Dot(color: app.color(id)); Text(app.courses[id]?.name ?? "") }
                    }
                }
                .font(Typeset.small).foregroundStyle(Ink.ink2)
                .accessibilityHidden(true)
            }
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(items.enumerated()), id: \.element.id) { i, it in
                    row(it, prev: i > 0 ? x(items[i - 1].courseId) : nil, cur: x(it.courseId), last: i == items.count - 1, lanes: lanes.map(x), width: width)
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(label)
        }
    }

    private func row(_ it: LaneItem, prev: CGFloat?, cur: CGFloat, last: Bool, lanes: [CGFloat], width: CGFloat) -> some View {
        let c = app.color(it.courseId)
        return HStack(alignment: .center, spacing: 14) {
            Canvas { ctx, size in
                let h = size.height, mid = h / 2
                for lx in lanes { ctx.stroke(Path { $0.move(to: .init(x: lx, y: 0)); $0.addLine(to: .init(x: lx, y: h)) }, with: .color(Ink.line), lineWidth: 1) }
                if let prev {
                    ctx.stroke(Path {
                        $0.move(to: .init(x: prev, y: 0))
                        $0.addCurve(to: .init(x: cur, y: mid), control1: .init(x: prev, y: h * 0.3), control2: .init(x: cur, y: h * 0.2))
                    }, with: .color(c), style: .init(lineWidth: 2, lineCap: .round))
                }
                if !last { ctx.stroke(Path { $0.move(to: .init(x: cur, y: mid)); $0.addLine(to: .init(x: cur, y: h)) }, with: .color(c), style: .init(lineWidth: 2, lineCap: .round)) }
            }
            .frame(width: width)
            .overlay(alignment: .leading) { dot(it.state, c).offset(x: cur - 6) }
            .accessibilityHidden(true)

            rowBody(it)
        }
        .frame(minHeight: rowHeight)
        // The canvas takes the row's height from its text instead of growing to fill the page.
        .fixedSize(horizontal: false, vertical: true)
    }

    @ViewBuilder private func dot(_ st: LaneState, _ c: Color) -> some View {
        let halo = Circle().fill(ground).frame(width: 14, height: 14)
        switch st {
        case .later: halo.overlay(Circle().strokeBorder(c, lineWidth: 2).frame(width: 10, height: 10)).frame(width: 12, height: 12)
        case .skipped: halo.overlay(Circle().strokeBorder(Ink.ink3, lineWidth: 1.5).frame(width: 10, height: 10)).frame(width: 12, height: 12)
        case .next: halo.overlay(Circle().fill(c).frame(width: 10, height: 10)).overlay(Circle().stroke(c.opacity(0.35), lineWidth: 2).frame(width: 16, height: 16)).frame(width: 12, height: 12)
        case .current: halo.overlay(Circle().fill(c).frame(width: 12, height: 12)).overlay(Circle().stroke(c, lineWidth: 2).frame(width: 18, height: 18)).frame(width: 12, height: 12)
        case .done: halo.overlay(Circle().fill(c).frame(width: 10, height: 10)).frame(width: 12, height: 12)
        }
    }

    @ViewBuilder private func rowBody(_ it: LaneItem) -> some View {
        let content = HStack(alignment: .center, spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(it.name)
                    .font(Typeset.serif(14, weight: it.state == .next || it.state == .current ? .semibold : .regular))
                    .foregroundStyle(it.state == .skipped ? Ink.ink2 : Ink.ink)
                    .strikethrough(it.state == .skipped, color: Ink.ink3)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                if it.deck != nil || !it.meta.isEmpty {
                    HStack(spacing: 10) {
                        DeckLabel(doc: it.deck).frame(maxWidth: 220, alignment: .leading).fixedSize(horizontal: true, vertical: false)
                        Text(it.meta).lineLimit(1).truncationMode(.tail)
                    }
                    .font(Typeset.small).foregroundStyle(Ink.ink2)
                }
            }
            Spacer(minLength: 8)
            if !it.aside.isEmpty { Text(it.aside).font(Typeset.small.monospacedDigit()).foregroundStyle(Ink.ink2) }
        }
        .padding(.vertical, 6).padding(.horizontal, 8)
        .contentShape(Rectangle())

        if let action = it.action {
            Button(action: action) { content }
                .buttonStyle(HoverRow())
                .disabled(it.disabled)
                .opacity(it.disabled ? 0.6 : 1)
                .help(it.help)
        } else {
            content.help(it.help)
        }
    }
}
