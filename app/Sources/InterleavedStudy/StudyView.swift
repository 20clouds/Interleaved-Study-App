// Study: choose a pool and a timer, then study timed blocks or Pomodoros, rate, quiz, and accept or override the
// next suggestion. All timing lives in the engine, so switching views never loses it.
import SwiftUI

struct StudyView: View {
    @Environment(AppModel.self) private var app
    var body: some View {
        if let s = app.session { SessionStage(s: s) } else { StartPanel() }
    }
}

// MARK: - Start

private enum PoolKind: String, CaseIterable, Identifiable {
    case global, course, lesson, unit, topic, sandbox
    var id: String { rawValue }
    var label: String {
        switch self {
        case .global: "Everything"
        case .course: "Courses"
        case .lesson: "One lesson"
        case .unit: "One unit"
        case .topic: "One topic"
        case .sandbox: "Sandbox"
        }
    }
    var help: String {
        switch self {
        case .global: "All courses, mixed"
        case .course: "One or more, mixed"
        case .lesson: "Its topics, in a mixed order"
        case .unit: "Part of a lesson"
        case .topic: "Just this one"
        case .sandbox: "Hand-picked, off the schedule"
        }
    }
}

private struct StartPanel: View {
    @Environment(AppModel.self) private var app
    @State private var kind: PoolKind = .global
    @State private var courseIds: Set<String> = []
    @State private var docId = ""
    @State private var unit = ""
    @State private var ids: [String] = []
    @State private var filter = ""
    @State private var preview: [Suggestion]?

    var body: some View {
        if let d = app.data, let settings = app.settings {
            if d.topics.isEmpty {
                EmptyState(title: "No topics to study yet", message: "Import lessons in the Library first.") {
                    Button("Open the Library") { app.screen = .library }
                }
            } else {
                Page {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("What do you want to study?").font(Typeset.display)
                        Text("Pick a pool and a timer. The app mixes topics from different lessons and courses, puts overdue reviews first and gives close deadlines priority.")
                            .font(.system(size: 15)).foregroundStyle(Ink.ink2).frame(maxWidth: 640, alignment: .leading)
                    }
                    HStack(alignment: .top, spacing: 48) {
                        VStack(alignment: .leading, spacing: 32) {
                            poolSection(d)
                            timerSection(settings)
                            HStack(spacing: 12) {
                                Button { Task { await start() } } label: { HStack(spacing: 8) { Label("Start with the Top Suggestion", systemImage: "play.fill"); KeyHint(key: "S") } }
                                    .buttonStyle(.primaryBig)
                                    .disabled(pool == nil || (preview ?? []).isEmpty)
                                Text(pool == nil ? "Choose what goes in the pool" : label).font(Typeset.small).foregroundStyle(Ink.ink2)
                            }
                        }
                        .frame(maxWidth: 520, alignment: .leading)
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Suggested order").font(Typeset.heading)
                            Text("Click any topic to start with it instead.").font(Typeset.small).foregroundStyle(Ink.ink2)
                            SuggestionLanes(items: preview, limit: 12) { id in Task { await start(id) } }
                        }
                        .frame(maxWidth: .infinity, alignment: .topLeading)
                    }
                }
                .onAppear(perform: fromScope)
                .task(id: PreviewKey(pool: pool, version: app.dataVersion)) {
                    guard let pool else { preview = []; return }
                    preview = nil
                    preview = (try? await Engine.call("sessionPreview", pool, pool.isSandbox)) ?? []
                }
                .task(id: app.quickstart) {
                    guard app.quickstart > app.quickstartHandled else { return }
                    app.quickstartHandled = app.quickstart
                    await start()
                }
            }
        }
    }

    private struct PreviewKey: Hashable { let pool: Pool?; let version: Int }

    /// The Library's scope picks the pool the page opens with.
    private func fromScope() {
        guard let d = app.data else { return }
        docId = d.docs.first?.id ?? ""
        switch app.scope {
        case .all: break
        case .course(let id): kind = .course; courseIds = [id]
        case .lesson(let id): kind = .lesson; docId = id
        case .unit(let id, let u): kind = .unit; docId = id; unit = u
        }
    }

    private var units: [String] { app.units(of: docId) }

    private var pool: Pool? {
        switch kind {
        case .global: .global
        case .course: courseIds.isEmpty ? nil : .course(app.data?.courses.map(\.id).filter(courseIds.contains) ?? [])
        case .lesson: docId.isEmpty ? nil : .lesson(docId: docId)
        case .unit: docId.isEmpty || !units.contains(unit) ? nil : .unit(docId: docId, unit: unit)
        case .topic: ids.isEmpty ? nil : .topic([ids[0]])
        case .sandbox: ids.isEmpty ? nil : .sandbox(ids)
        }
    }

    private var label: String {
        switch kind {
        case .global: "Everything"
        case .course: (app.data?.courses ?? []).filter { courseIds.contains($0.id) }.map(\.name).joined(separator: ", ")
        case .lesson: app.docs[docId]?.title ?? "Lesson"
        case .unit: unit
        case .topic: ids.first.flatMap { app.topics[$0]?.name } ?? "Topic"
        case .sandbox: "Sandbox of \(ids.count) topics"
        }
    }

    private func start(_ topicId: String? = nil) async {
        guard let pool else { return }
        await app.act { try await Engine.run("sessionStart", ["mode": app.mode, "pool": pool, "poolLabel": label, "topicId": topicId] as [String: Any?]) }
    }

    // MARK: Pool

    private func poolSection(_ d: DataSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Pool").font(Typeset.heading)
            Picker("Pool", selection: $kind) {
                ForEach(PoolKind.allCases) { k in
                    VStack(alignment: .leading, spacing: 0) {
                        Text(k.label)
                        Text(k.help).font(Typeset.small).foregroundStyle(Ink.ink2)
                    }
                    .tag(k)
                }
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()
            .onChange(of: kind) { if kind == .unit && !units.contains(unit) { unit = units.first ?? "" } }

            switch kind {
            case .course:
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(d.courses) { c in
                        Toggle(isOn: Binding(get: { courseIds.contains(c.id) }, set: { on in if on { courseIds.insert(c.id) } else { courseIds.remove(c.id) } })) {
                            HStack(spacing: 6) { Dot(color: app.color(c.id)); Text(c.name) }
                        }
                    }
                }
            case .lesson, .unit:
                Picker("Lesson", selection: $docId) {
                    ForEach(d.courses) { c in
                        Section(c.name) { ForEach(app.lessons(of: c.id)) { Text($0.title).tag($0.id) } }
                    }
                }
                .frame(maxWidth: 360)
                .onChange(of: docId) { if kind == .unit { unit = units.first ?? "" } }
                if kind == .unit {
                    Picker("Unit", selection: $unit) { ForEach(units, id: \.self) { Text($0).tag($0) } }.frame(maxWidth: 360)
                }
            case .topic, .sandbox:
                TopicPicker(ids: $ids, multiple: kind == .sandbox, filter: $filter)
                if kind == .sandbox {
                    Text("Ratings in a sandbox stay separate. At the end you choose to save them to the schedule or discard them.")
                        .font(Typeset.small).foregroundStyle(Ink.ink2)
                }
            case .global:
                EmptyView()
            }
        }
    }

    // MARK: Timer

    private func timerSection(_ s: Settings) -> some View {
        @Bindable var app = app
        return VStack(alignment: .leading, spacing: 12) {
            Text("Timer").font(Typeset.heading)
            Picker("Mode", selection: $app.mode) {
                Text("Timed topics").tag("timed")
                Text("Interleaved Pomodoro").tag("pomodoro")
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: 360)
            if app.mode == "timed" {
                HStack(spacing: 6) {
                    Text("Each topic gets")
                    NumberField(value: s.study.minutes, range: 1...240) { app.saveSettings(["study": ["minutes": $0]]) }
                    Text("minutes.")
                }
                Toggle("Use each topic’s suggested time instead", isOn: Binding(get: { s.study.useSuggested }, set: { app.saveSettings(["study": ["useSuggested": $0]]) }))
            } else {
                PomodoroFields(pomo: s.pomo)
            }
        }
    }
}

/// The Pomodoro timings as one sentence of fields, shared by Study and Settings.
struct PomodoroFields: View {
    @Environment(AppModel.self) private var app
    let pomo: Settings.Pomo
    var body: some View {
        let save = { (k: String, v: Any) in app.saveSettings(["pomo": [k: v]]) }
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Text("Work"); NumberField(value: pomo.work, range: 1...240) { save("work", $0) }
                Text("min, break"); NumberField(value: pomo.brk, range: 1...120) { save("brk", $0) }
                Text("min,")
            }
            HStack(spacing: 6) {
                Text("a long break of"); NumberField(value: pomo.long, range: 1...240) { save("long", $0) }
                Text("min every"); NumberField(value: pomo.cycles, range: 1...20) { save("cycles", $0) }
                Text("blocks.")
            }
            HStack(spacing: 6) {
                Picker("Topic length", selection: Binding(get: { pomo.perTopic }, set: { save("perTopic", $0) })) {
                    Text("1 Pomodoro per topic").tag("one")
                    Text("N Pomodoros per topic").tag("n")
                    Text("Custom minutes per topic").tag("minutes")
                }
                .labelsHidden()
                .fixedSize()
                if pomo.perTopic == "n" { NumberField(value: pomo.n, range: 1...20) { save("n", $0) } }
                if pomo.perTopic == "minutes" { NumberField(value: pomo.minutes, range: 1...600) { save("minutes", $0) }; Text("min per topic") }
            }
        }
    }
}

/// A small number field that saves on Return or when it loses focus, clamped to its range.
struct NumberField: View {
    let value: Double
    var range: ClosedRange<Double> = 0...1_000_000
    var width: CGFloat = 56
    let onSave: (Double) -> Void
    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        TextField("", text: $text)
            .textFieldStyle(.roundedBorder)
            .multilineTextAlignment(.trailing)
            .monospacedDigit()
            .frame(width: width)
            .focused($focused)
            .onAppear { text = Self.show(value) }
            .onChange(of: value) { text = Self.show(value) }
            .onSubmit(commit)
            .onChange(of: focused) { if !focused { commit() } }
    }

    private static func show(_ v: Double) -> String { v == v.rounded() ? String(Int(v)) : String(v) }

    /// An empty or non-numeric entry is not a value: show what is stored again and save nothing.
    private func commit() {
        guard let n = Double(text.trimmingCharacters(in: .whitespaces)) else { text = Self.show(value); return }
        let c = min(range.upperBound, max(range.lowerBound, n))
        text = Self.show(c)
        if c != value { onSave(c) }
    }
}

/// Topics to pick from, filtered by name: one (radio) or several (checkboxes).
struct TopicPicker: View {
    @Environment(AppModel.self) private var app
    @Binding var ids: [String]
    let multiple: Bool
    @Binding var filter: String

    var body: some View {
        let list = (app.data?.topics ?? []).filter { filter.isEmpty || $0.name.localizedCaseInsensitiveContains(filter) }.prefix(300)
        VStack(alignment: .leading, spacing: 6) {
            TextField("Filter topics", text: $filter).textFieldStyle(.roundedBorder)
            List(Array(list)) { t in
                Toggle(isOn: Binding(get: { ids.contains(t.id) }, set: { on in
                    if !multiple { ids = on ? [t.id] : [] } else if on { ids.append(t.id) } else { ids.removeAll { $0 == t.id } }
                })) {
                    HStack(spacing: 8) {
                        Text(t.name).font(Typeset.topic)
                        DeckLabel(doc: app.docs[t.docId])
                    }
                }
                .toggleStyle(.checkbox)
            }
            .listStyle(.bordered)
            .frame(height: min(220, CGFloat(max(list.count, 1)) * 26 + 12))
        }
    }
}

/// A pool's suggested order as the interleave line; clicking a topic picks it.
struct SuggestionLanes: View {
    @Environment(AppModel.self) private var app
    let items: [Suggestion]?
    var limit = 12
    var busy = false
    var ground: Color = Ink.paper
    let onPick: (String) -> Void

    var body: some View {
        if let items {
            if items.isEmpty {
                Text("No topics in this pool.").foregroundStyle(Ink.ink2)
            } else {
                Lanes(items: items.prefix(limit).enumerated().map { i, s in
                    let t = app.topics[s.topicId]
                    return LaneItem(id: s.topicId, courseId: t?.courseId ?? "", state: i == 0 ? .next : .later, name: s.name, deck: t.flatMap { app.docs[$0.docId] },
                                    meta: s.reason, help: busy ? "Rate or skip the rating first" : "Study this topic", action: { onPick(s.topicId) }, disabled: busy)
                }, label: "Suggested order", ground: ground)
            }
        } else {
            ProgressView().controlSize(.small)
        }
    }
}

// MARK: - Session

private struct SessionStage: View {
    @Environment(AppModel.self) private var app
    let s: SessionState
    @State private var quiz = false
    @State private var others: [Suggestion]?

    var body: some View {
        HStack(spacing: 0) {
            ScrollView {
                stage.padding(48).frame(maxWidth: 820, alignment: .leading).frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Ink.paper)
            Divider()
            ScrollView { side.padding(24) }
                .frame(width: 340)
                .background(Ink.panel)
        }
        .onChange(of: s.topicId) { quiz = false }
        .onChange(of: s.phase) { quiz = false }
        .task(id: OthersKey(phase: s.phase, topic: s.topicId, pool: s.pool, version: app.dataVersion)) {
            guard s.phase != "work" else { others = nil; return }
            others = (try? await Engine.call("sessionPreview", s.pool, s.sandbox)) ?? []
        }
    }

    private struct OthersKey: Hashable { let phase: String; let topic: String?; let pool: Pool; let version: Int }

    private var topic: Topic? { s.topicId.flatMap { app.topics[$0] } }
    private var proposalCourse: String? { s.proposal.flatMap { app.topics[$0.topicId]?.courseId } }

    // MARK: Stage

    @ViewBuilder private var stage: some View {
        VStack(alignment: .leading, spacing: 28) {
            if let m = s.message {
                Text(m).padding(12).frame(maxWidth: .infinity, alignment: .leading).background(RoundedRectangle(cornerRadius: 8).fill(Ink.due.opacity(0.08))).foregroundStyle(Ink.due)
            }
            if s.phase == "done" { sandboxDone } else {
                head
                if s.timing { clock }
                if s.phase == "rate" { rating }
                if s.phase == "next" || s.phase == "break" { upNext }
            }
        }
    }

    private var sandboxDone: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Sandbox finished").font(Typeset.display)
            Text("\(s.sandboxReviews.count) block(s) recorded in this sandbox, \(fmtMin(s.sandboxReviews.reduce(0) { $0 + $1.minutes })) in total. They have not touched your schedule.")
                .font(.system(size: 15)).foregroundStyle(Ink.ink2)
            HStack(spacing: 12) {
                Button("Save to Schedule") { app.perform("Saved to schedule") { try await Engine.run("sessionSandbox", true) } }.buttonStyle(.primaryBig)
                Button("Discard") { app.perform("Sandbox discarded") { try await Engine.run("sessionSandbox", false) } }.controlSize(.large)
            }
        }
    }

    @ViewBuilder private var head: some View {
        let d = topic.flatMap { app.docs[$0.docId] }
        VStack(alignment: .leading, spacing: 10) {
            switch s.phase {
            case "next":
                Text("Finished: \(s.blocks.last(where: { $0.state != "current" })?.name ?? s.topicName)").font(Typeset.small).foregroundStyle(Ink.ink2)
            case "break":
                Text((s.breakKind == "long" ? "Long break" : "Break") + (s.blockIndex < s.blockPlan.count - 1 ? ", then back to \(s.topicName) (block \(s.blockIndex + 2) of \(s.blocksForTopic))" : ""))
                    .font(Typeset.small).foregroundStyle(Ink.ink2)
            default:
                HStack(spacing: 6) {
                    Dot(color: app.color(topic?.courseId))
                    Text((topic.flatMap { app.courses[$0.courseId]?.name } ?? "") + (d.map { " / \($0.title)" } ?? ""))
                }
                .font(Typeset.small).foregroundStyle(Ink.ink2)
            }
            Text(s.phase == "break" ? "Stand up, look away, drink water." : s.phase == "next" ? "Choose the next topic" : s.topicName)
                .font(Typeset.serif(34, weight: .medium))
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if s.phase == "work" || s.phase == "rate", let d {
                (Text(Image(systemName: "doc")) + Text(" Study from ") + Text(d.deckName).bold()).foregroundStyle(Ink.ink2)
            }
        }
    }

    private var clock: some View {
        VStack(alignment: .leading, spacing: 14) {
            TimelineView(.periodic(from: .now, by: 0.25)) { ctx in
                Text(fmtClock(s.left(ctx.date)))
                    .font(Typeset.clock)
                    .tracking(-2.6)
                    .foregroundStyle(s.paused ? Ink.ink3 : Ink.ink)
                    .accessibilityLabel("\(fmtClock(s.left(ctx.date))) left")
            }
            Text(s.paused ? "Paused" : s.phase == "work" ? (s.mode == "pomodoro" ? "Pomodoro \(s.blockIndex + 1) of \(s.blocksForTopic)" : "Timed block") : s.breakKind == "long" ? "Long break" : "Short break")
                .foregroundStyle(Ink.ink2)
            HStack(spacing: 10) {
                Button { app.togglePause() } label: {
                    HStack(spacing: 8) { Label(s.paused ? "Resume" : "Pause", systemImage: s.paused ? "play.fill" : "pause.fill"); KeyHint(key: "space") }
                }
                .buttonStyle(.primaryBig)
                if s.phase == "work" {
                    Button { app.perform { try await Engine.run("sessionEndEarly") } } label: { Label("Done, Rate Now", systemImage: "checkmark") }.controlSize(.large)
                }
                Button { app.perform { try await Engine.run("sessionExtend", 5) } } label: { Label("5 min", systemImage: "plus") }.buttonStyle(.borderless)
                if s.phase == "work" {
                    Button { app.perform { try await Engine.run("sessionSkip") } } label: { HStack(spacing: 6) { Label("Skip Topic", systemImage: "forward.end"); KeyHint(key: "N") } }
                        .buttonStyle(.borderless).help("Records time spent only")
                } else {
                    Button { app.perform { try await Engine.run("sessionSkipBreak") } } label: { HStack(spacing: 6) { Label("Skip Break", systemImage: "forward.end"); KeyHint(key: "N") } }
                        .buttonStyle(.borderless)
                }
            }
        }
    }

    private var rating: some View {
        @Bindable var app = app
        return VStack(alignment: .leading, spacing: 14) {
            Text("How well do you know it now?").font(Typeset.heading)
            HStack(spacing: 8) {
                ForEach(Array(Rating.allCases.enumerated()), id: \.element) { i, r in
                    Button { app.pending.rating = r } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack { Text(r.label).bold(); Spacer(); KeyHint(key: "\(i + 1)") }
                            Text(r.help).font(Typeset.small).foregroundStyle(Ink.ink2).multilineTextAlignment(.leading)
                        }
                        .padding(12)
                        .frame(maxWidth: .infinity, minHeight: 72, alignment: .topLeading)
                        .background(RoundedRectangle(cornerRadius: 8).fill(app.pending.rating == r ? Ink.sel : Ink.raised))
                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(app.pending.rating == r ? Ink.ink : Ink.line2, lineWidth: app.pending.rating == r ? 1.5 : 0.5))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(app.pending.rating == r ? .isSelected : [])
                }
            }
            if let q = app.pending.quiz {
                (Text("Quiz score ") + Text("\(Int((q * 100).rounded()))%").bold() + Text(", counts double")).font(Typeset.small)
            } else {
                Button { quiz.toggle() } label: { Label("Quick Quiz", systemImage: "questionmark.circle") }
            }
            if quiz, app.pending.quiz == nil, let id = s.topicId {
                QuizPanel(topicId: id) { app.pending.quiz = $0; quiz = false }
                    .padding(16)
                    .background(RoundedRectangle(cornerRadius: 8).fill(Ink.raised))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(Ink.line, lineWidth: 0.5))
            }
            TextField("Optional note for this review", text: $app.pending.note, axis: .vertical)
                .font(Typeset.serif(14))
                .lineLimit(2...4)
                .textFieldStyle(.roundedBorder)
            HStack(spacing: 10) {
                Button { app.rate() } label: { HStack(spacing: 8) { Text("Save Rating"); KeyHint(key: "Return") } }
                    .buttonStyle(.primaryBig).disabled(app.pending.rating == nil)
                Button("Skip Rating") { app.perform { try await Engine.run("sessionRate", ["rating": nil] as [String: Any?]) } }
                    .controlSize(.large).help("Records the time without a rating")
                Button { app.perform { try await Engine.run("sessionExtend", 5) } } label: { Label("Keep Going 5 More Min", systemImage: "plus") }
            }
        }
    }

    private var upNext: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let p = s.proposal {
                HStack(spacing: 6) {
                    Dot(color: app.color(proposalCourse))
                    Text("Up next" + (proposalCourse.flatMap { app.courses[$0]?.name }.map { ", \($0)" } ?? ""))
                }
                .font(Typeset.small).foregroundStyle(Ink.ink2)
                Text(p.name).font(Typeset.serif(22, weight: .medium))
                HStack(spacing: 10) {
                    DeckLabel(doc: app.topics[p.topicId].flatMap { app.docs[$0.docId] })
                    Text(p.reason).font(Typeset.small).foregroundStyle(Ink.ink2)
                }
                HStack(spacing: 10) {
                    let label = HStack(spacing: 8) { Label(s.phase == "break" ? "Start It Now" : "Start", systemImage: "play.fill"); KeyHint(key: "Return") }
                    if s.phase == "break" {
                        Button { app.perform { try await Engine.run("sessionAccept", p.topicId) } } label: { label }.controlSize(.large)
                    } else {
                        Button { app.perform { try await Engine.run("sessionAccept", p.topicId) } } label: { label }.buttonStyle(.primaryBig)
                    }
                    Button { app.perform { try await Engine.run("sessionSkipSuggestion") } } label: { HStack(spacing: 6) { Label("Something Else", systemImage: "forward.end"); KeyHint(key: "X") } }
                        .buttonStyle(.borderless).help("Suggest a different topic instead")
                }
                .padding(.top, 4)
            } else {
                Text("Nothing left to suggest in this pool. Pick a topic on the right or end the session.").foregroundStyle(Ink.ink2)
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 8).fill(Ink.raised))
        .overlay(alignment: .leading) { Rectangle().fill(app.color(proposalCourse)).frame(width: 3).clipShape(RoundedRectangle(cornerRadius: 1.5)) }
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Ink.line, lineWidth: 0.5))
    }

    // MARK: Side

    private var side: some View {
        let done = s.blocks.filter { $0.state != "current" }
        let cur = s.blocks.first { $0.state == "current" }
        var items: [LaneItem] = done.enumerated().map { i, b in
            LaneItem(id: "d\(i)", courseId: b.courseId, state: b.state == "skipped" ? .skipped : .done, name: b.name,
                     meta: "\(b.state == "skipped" ? "Skipped" : b.rating?.label ?? "Not rated"), \(b.minutes < 1 ? "<1m" : fmtMin(b.minutes))")
        }
        if let cur, s.phase != "next" {
            items.append(LaneItem(id: "cur", courseId: cur.courseId, state: .current, name: cur.name, meta: s.phase == "rate" ? "Rate it now" : s.phase == "break" ? "On a break" : "Now"))
        }
        if let p = s.proposal, s.phase != "work", let pc = proposalCourse {
            items.append(LaneItem(id: "next", courseId: pc, state: .next, name: p.name, meta: "Up next"))
        }
        return VStack(alignment: .leading, spacing: 24) {
            VStack(alignment: .leading, spacing: 4) {
                Text((s.mode == "pomodoro" ? "Interleaved Pomodoro" : "Timed topics") + (s.sandbox ? ", sandbox" : "")).font(Typeset.small).foregroundStyle(Ink.ink2)
                Text(s.poolLabel).font(Typeset.title)
            }
            if s.phase == "work", let t = topic, !t.summary.isEmpty || !t.terms.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text("About this topic").font(Typeset.heading)
                    if !t.summary.isEmpty { Text(t.summary).font(Typeset.serif(13)).foregroundStyle(Ink.ink).textSelection(.enabled) }
                    if !t.terms.isEmpty { Chips(items: t.terms) }
                }
            }
            VStack(alignment: .leading, spacing: 8) {
                Text("This session").font(Typeset.heading)
                if items.isEmpty { Text("No finished blocks yet.").font(Typeset.small).foregroundStyle(Ink.ink2) }
                else { Lanes(items: items, label: "Blocks in this session", legend: false, ground: Ink.panel, rowHeight: 44) }
            }
            if s.phase != "work" && s.phase != "done" {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Or choose any topic in the pool").font(Typeset.heading)
                    SuggestionLanes(items: others, limit: 8, busy: s.phase == "rate", ground: Ink.panel) { id in app.perform { try await Engine.run("sessionAccept", id) } }
                    if s.phase == "rate" { Text("Rate or skip the rating first.").font(Typeset.small).foregroundStyle(Ink.ink2) }
                }
            }
            if s.phase != "done" {
                Button(role: .destructive) { app.endSession() } label: { Label("End Session", systemImage: "stop.fill") }
                    .buttonStyle(.borderless)
                    .foregroundStyle(Ink.due)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// Key terms and prerequisites as quiet chips that wrap.
struct Chips: View {
    let items: [String]
    var body: some View {
        FlowLayout(spacing: 6) {
            ForEach(items, id: \.self) { x in
                Text(x).font(Typeset.small).padding(.horizontal, 8).padding(.vertical, 3)
                    .background(RoundedRectangle(cornerRadius: 5).fill(Ink.sel))
            }
        }
    }
}

/// Lays children out left to right, wrapping onto new lines.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = rows(proposal.width ?? .infinity, subviews)
        return CGSize(width: proposal.width ?? rows.map(\.width).max() ?? 0, height: rows.reduce(0) { $0 + $1.height } + spacing * CGFloat(max(0, rows.count - 1)))
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in rows(bounds.width, subviews) {
            var x = bounds.minX
            for i in row.items {
                let size = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: y), proposal: .init(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }
    private func rows(_ width: CGFloat, _ subviews: Subviews) -> [(items: [Int], width: CGFloat, height: CGFloat)] {
        var out: [(items: [Int], width: CGFloat, height: CGFloat)] = []
        var cur: [Int] = [], w: CGFloat = 0, h: CGFloat = 0
        for (i, v) in subviews.enumerated() {
            let s = v.sizeThatFits(.unspecified)
            if !cur.isEmpty && w + spacing + s.width > width { out.append((cur, w, h)); cur = []; w = 0; h = 0 }
            w += (cur.isEmpty ? 0 : spacing) + s.width
            h = max(h, s.height)
            cur.append(i)
        }
        if !cur.isEmpty { out.append((cur, w, h)) }
        return out
    }
}
