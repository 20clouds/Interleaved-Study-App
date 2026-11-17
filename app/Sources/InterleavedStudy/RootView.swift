// The main window: a source-list sidebar (views, then courses and their lessons), the toolbar strip with the session
// clock, the rate-limit banner, file drops onto the window, and the one-key study shortcuts.
import AppKit
import SwiftUI

struct RootView: View {
    @Environment(AppModel.self) private var app
    @State private var dropping = false

    var body: some View {
        @Bindable var app = app
        NavigationSplitView {
            Sidebar()
                .navigationSplitViewColumnWidth(min: 200, ideal: 230, max: 320)
        } detail: {
            detail
                .navigationTitle(app.screen.title)
                .toolbar { ToolbarItem(placement: .primaryAction) { Ribbon() } }
                .safeAreaInset(edge: .top, spacing: 0) { RateLimitBanner() }
        }
        .overlay(alignment: .bottom) { ToastView() }
        .overlay { if dropping { DropTarget() } }
        .dropDestination(for: URL.self) { urls, _ in
            let paths = urls.filter(\.isFileURL).map(\.path)
            guard !paths.isEmpty else { return false }
            app.importer.drop(paths, app: app)
            return true
        } isTargeted: { dropping = $0 }
        .modifier(ImportSheets())
        .sheet(isPresented: Binding(get: { app.courseWizard != nil }, set: { if !$0 { app.courseWizard = nil } })) { NewCourseSheet() }
        .background(StudyKeys(app: app))
    }

    @ViewBuilder private var detail: some View {
        if let err = app.dataError, app.data == nil {
            EmptyState(title: "The data could not be loaded", message: err) {
                Button("Try Again") { Task { await app.reload() } }
            }
        } else if app.data == nil || app.settings == nil {
            ProgressView().controlSize(.small).frame(maxWidth: .infinity, maxHeight: .infinity).background(Ink.paper)
        } else {
            switch app.screen {
            case .today: TodayView()
            case .session: StudyView()
            case .library: LibraryView()
            case .plan: PlanView()
            case .progress: ProgressScreen()
            case .queue: QueueView()
            }
        }
    }
}

// MARK: - Sidebar

private enum SidebarItem: Hashable {
    case screen(Screen)
    case scope(Scope)
}

private struct SidebarNode: Identifiable {
    let id: SidebarItem
    let title: String
    var color: Color? = nil
    var count: (n: Int, due: Int)? = nil
    var children: [SidebarNode]? = nil
}

private struct Sidebar: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        List(selection: selection) {
            Section {
                ForEach([Screen.today, .session, .library, .plan, .progress]) { s in
                    Label(s.title, systemImage: s.symbol)
                        .badge(s == .today && app.dueCount > 0 ? app.dueCount : 0)
                        .tag(SidebarItem.screen(s))
                        .help(s.title + " (⌘" + String(s.key.character) + ")")
                }
            }
            if !(app.data?.courses.isEmpty ?? true) {
                Section("Courses") {
                    OutlineGroup(courseNodes, children: \.children) { node in
                        HStack(spacing: 8) {
                            if let c = node.color { Dot(color: c) }
                            Text(node.title).lineLimit(1)
                            Spacer()
                            if let c = node.count {
                                Text(c.due > 0 ? "\(c.due) due" : "\(c.n)")
                                    .font(Typeset.small.monospacedDigit())
                                    .foregroundStyle(c.due > 0 ? Ink.due : Ink.ink3)
                                    .help("\(c.due) due of \(c.n)")
                            }
                        }
                        .tag(node.id)
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .bottom, spacing: 0) { SidebarFoot() }
    }

    private var selection: Binding<SidebarItem?> {
        Binding(
            get: {
                guard app.screen == .library else { return .screen(app.screen) }
                return app.scope == .all ? .screen(.library) : .scope(app.scope)
            },
            set: { item in
                switch item {
                case .screen(let s)?:
                    app.screen = s
                    if s == .library { app.scope = .all }
                case .scope(let sc)?:
                    app.screen = .library
                    app.scope = sc
                case nil: break
                }
            })
    }

    /// Courses, their lessons, and a lesson's units when it has more than one.
    private var courseNodes: [SidebarNode] {
        guard let d = app.data else { return [] }
        let now = app.now
        func count(_ pred: (Topic) -> Bool) -> (Int, Int) {
            var n = 0, due = 0
            for t in d.topics where pred(t) { n += 1; if t.status(now) == .review { due += 1 } }
            return (n, due)
        }
        return d.courses.map { c in
            let lessons = app.lessons(of: c.id).map { doc -> SidebarNode in
                let units = app.units(of: doc.id)
                return SidebarNode(id: .scope(.lesson(doc.id)), title: doc.title, count: count { $0.docId == doc.id },
                                   children: units.count > 1 ? units.map { u in SidebarNode(id: .scope(.unit(docId: doc.id, unit: u)), title: u, count: count { $0.docId == doc.id && $0.unit == u }) } : nil)
            }
            return SidebarNode(id: .scope(.course(c.id)), title: c.name, color: app.color(c.id), count: count { $0.courseId == c.id }, children: lessons.isEmpty ? nil : lessons)
        }
    }
}

/// What the AI is doing right now, and Settings.
private struct SidebarFoot: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Divider().padding(.bottom, 6)
            HStack(spacing: 6) {
                Button { app.screen = .queue } label: {
                    HStack(spacing: 8) {
                        if let q = app.queue, q.running != nil, !q.userPaused { ProgressView().controlSize(.mini).frame(width: 16) }
                        else { Image(systemName: Screen.queue.symbol).frame(width: 16) }
                        VStack(alignment: .leading, spacing: 0) {
                            Text("AI Queue")
                            if let state { Text(state).font(Typeset.small).foregroundStyle(warn ? Ink.due : Ink.ink2) }
                        }
                        Spacer()
                        if state == nil, let q = app.queue, q.queued + q.failed.count > 0 { Text("\(q.queued + q.failed.count)").font(Typeset.small.monospacedDigit()).foregroundStyle(Ink.ink3) }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("AI Queue (⌘6)")
                if app.queue?.busy == true {
                    Button { app.perform("AI requests cancelled") { try await Engine.run("cancelAi") } } label: { Image(systemName: "xmark") }
                        .buttonStyle(.borderless)
                        .help("Cancel all AI requests")
                }
            }
            .padding(.horizontal, 10).padding(.vertical, 4)
            .background(RoundedRectangle(cornerRadius: 6).fill(app.screen == .queue ? Ink.sel : .clear))
            SettingsLink {
                Label("Settings", systemImage: "slider.horizontal.3").frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 10).padding(.vertical, 4)
            .help("Settings (⌘,)")
        }
        .padding(.horizontal, 8).padding(.bottom, 10)
    }

    private var warn: Bool { app.queue?.rateLimit != nil || !(app.queue?.failed.isEmpty ?? true) }

    private var state: String? {
        guard let q = app.queue else { return nil }
        if q.rateLimit != nil { return "Limit reached, paused" }
        if !q.failed.isEmpty { return "\(q.failed.count) failed" }
        if q.userPaused && q.queued > 0 { return "Paused" }
        if q.run?.state == "waiting" { return "Waiting for Claude" }
        if q.busy { return "Organizing \(q.chunksDone)/\(q.chunksTotal > 0 ? q.chunksTotal : q.queued)" }
        return nil
    }
}

// MARK: - Toolbar strip and clock

/// This session's blocks coloured by course, the next proposal, and the countdown. The one orchestrated motion:
/// the strip moves one block left when a new block starts.
struct Ribbon: View {
    @Environment(AppModel.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var shift: CGFloat = 0

    var body: some View {
        if let s = app.session { running(s) } else { idle }
    }

    private var idle: some View {
        HStack(spacing: 10) {
            Text("No session running.").font(Typeset.small).foregroundStyle(Ink.ink2)
            Button { app.screen = .session; app.quickstart += 1 } label: {
                HStack(spacing: 6) { Image(systemName: "play.fill"); Text("Start next suggested"); KeyHint(key: "S") }
            }
            .disabled(app.data?.topics.isEmpty ?? true)
        }
    }

    private func running(_ s: SessionState) -> some View {
        let done = s.blocks.filter { $0.state != "current" }
        let cur = s.blocks.first { $0.state == "current" }
        return HStack(spacing: 12) {
            HStack(spacing: 3) {
                ForEach(Array(done.enumerated()), id: \.offset) { _, b in
                    RoundedRectangle(cornerRadius: 2)
                        .fill(b.state == "skipped" ? Ink.line2 : app.color(b.courseId))
                        .frame(width: max(18, min(90, b.minutes * 2.2)), height: 6)
                        .help("\(b.name): \(b.state == "skipped" ? "skipped" : "rated \(b.rating?.rawValue ?? "")"), \(Int(b.minutes.rounded())) min")
                }
                if let cur {
                    Button { app.screen = .session } label: {
                        HStack(spacing: 6) {
                            Dot(color: app.color(cur.courseId))
                            Text(cur.name).font(Typeset.serif(12, weight: .semibold)).lineLimit(1)
                            if s.mode == "pomodoro" && s.blocksForTopic > 1 { Text("\(s.blockIndex + 1)/\(s.blocksForTopic)").font(Typeset.small.monospacedDigit()).foregroundStyle(Ink.ink2) }
                        }
                        .frame(maxWidth: 220)
                    }
                    .buttonStyle(.borderless)
                    .help("Open the session")
                }
                if let p = s.proposal, s.phase != "work" {
                    Button { app.screen = .session } label: {
                        HStack(spacing: 6) {
                            Text("Next").font(Typeset.small).foregroundStyle(Ink.ink2)
                            Dot(color: app.color(app.topics[p.topicId]?.courseId))
                            Text(p.name).font(Typeset.serif(12)).lineLimit(1)
                        }
                        .frame(maxWidth: 200)
                    }
                    .buttonStyle(.borderless)
                    .help(p.reason)
                }
            }
            .offset(x: shift)
            .onChange(of: app.advance) {
                guard !reduceMotion else { return }
                shift = 24
                withAnimation(.timingCurve(0.22, 1, 0.36, 1, duration: 0.3)) { shift = 0 }
            }
            if s.timing {
                Button { app.togglePause() } label: { Image(systemName: s.paused ? "play.fill" : "pause.fill") }
                    .help(s.paused ? "Resume (space)" : "Pause (space)")
                Button { app.perform { try await Engine.run("sessionSkip") } } label: { Image(systemName: "forward.end.fill") }
                    .help(s.phase == "break" ? "Skip break (N)" : "Skip topic, keep time (N)")
            }
            TimelineView(.periodic(from: .now, by: 1)) { ctx in
                VStack(alignment: .trailing, spacing: 0) {
                    if s.timing {
                        Text(fmtClock(s.left(ctx.date)))
                            .font(.system(size: 15, weight: .medium).monospacedDigit())
                            .foregroundStyle(s.paused ? Ink.ink3 : Ink.ink)
                    }
                    Text(phaseText(s)).font(.system(size: s.timing ? 10 : 12)).foregroundStyle(Ink.ink2)
                }
                .frame(minWidth: 70, alignment: .trailing)
            }
        }
    }

    private func phaseText(_ s: SessionState) -> String {
        if s.timing && s.paused { return "Paused" }
        switch s.phase {
        case "break": return s.breakKind == "long" ? "Long break" : "Break"
        case "work": return "Studying"
        case "rate": return "Rate this block"
        case "next": return "Choose the next topic"
        default: return "Save or discard the sandbox"
        }
    }
}

// MARK: - Banners, toast, drop target

/// Shown on every view while the provider's rate limit has paused the AI. Continue resumes the saved queue: the
/// request that hit the limit is retried and everything already finished stays done.
private struct RateLimitBanner: View {
    @Environment(AppModel.self) private var app
    var body: some View {
        if let rl = app.queue?.rateLimit {
            let name = Providers.info(rl.provider).shortLabel
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "pause.circle").foregroundStyle(Ink.due)
                VStack(alignment: .leading, spacing: 2) {
                    TimelineView(.periodic(from: .now, by: 1)) { ctx in
                        (Text(rl.provider == "routine" ? "The Claude scheduled task stopped answering or hit its limit. " : "\(name) rate limit reached. ").bold()
                         + Text("Everything is paused and nothing is lost; Continue picks up exactly where it stopped.") + Text(wait(rl, name, ctx.date)))
                    }
                    Text("\(rl.model): \(String(rl.message.prefix(160)))").font(Typeset.small).foregroundStyle(Ink.ink2).textSelection(.enabled).help(rl.message)
                }
                Spacer()
                Button { app.perform("Continuing") { try await Engine.run("resumeQueue") } } label: { Label("Continue", systemImage: "play.fill") }
                    .buttonStyle(.primary)
            }
            .font(Typeset.body)
            .padding(.horizontal, 20).padding(.vertical, 10)
            .background(Ink.panel)
            .overlay(alignment: .bottom) { Divider() }
        }
    }

    private func wait(_ rl: RateLimitInfo, _ name: String, _ now: Date) -> String {
        guard let reset = rl.resetAt else { return "" }
        let left = max(0, Int(((reset - now.ms) / 1000).rounded(.up)))
        if left == 0 { return " \(name)’s wait time has passed." }
        return " \(name) asked to wait \(left >= 120 ? "about \(Int((Double(left) / 60).rounded(.up))) min" : "\(left) s")."
    }
}

private struct ToastView: View {
    @Environment(AppModel.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        VStack {
            if app.importer.busy {
                bubble { HStack(spacing: 8) { ProgressView().controlSize(.small); Text("Importing…") } }
            } else if let t = app.toast {
                bubble { Text(t.message).foregroundStyle(t.error ? Ink.due : Ink.ink).textSelection(.enabled) }
                    .accessibilityAddTraits(.updatesFrequently)
            }
        }
        .padding(.bottom, 24)
        .animation(reduceMotion ? nil : .timingCurve(0.22, 1, 0.36, 1, duration: 0.2), value: app.toast)
    }

    private func bubble<C: View>(@ViewBuilder _ c: () -> C) -> some View {
        c()
            .font(Typeset.body)
            .padding(.horizontal, 16).padding(.vertical, 10)
            .background(RoundedRectangle(cornerRadius: 8).fill(Ink.raised))
            .shadow(color: .black.opacity(0.06), radius: 1, y: 1)
            .shadow(color: .black.opacity(0.18), radius: 16, y: 12)
            .frame(maxWidth: 560)
            .transition(.opacity.combined(with: .move(edge: .bottom)))
    }
}

private struct DropTarget: View {
    @Environment(AppModel.self) private var app
    var body: some View {
        let target = app.screen == .library && app.courseWizard == nil ? app.openCourse.flatMap { app.courses[$0] } : nil
        VStack(spacing: 4) {
            Text(app.courseWizard != nil ? "Drop files or folders to add them to the new course"
                 : target.map { "Drop lesson files or folders to add them to \($0.name)" } ?? "Drop lesson files or folders to start a new course")
                .font(Typeset.title)
            Text(FileTypes.dropSummary(app.fileSupport)).font(Typeset.small).foregroundStyle(Ink.ink2)
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Ink.paper.opacity(0.92))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Ink.ink3, style: .init(lineWidth: 1.5, dash: [6, 4])).padding(16))
        .allowsHitTesting(false)
    }
}

// MARK: - One-key shortcuts

/// Space pauses or resumes, N skips, 1–4 rate, Return saves the rating or starts the proposal, X asks for something
/// else, S starts studying. Ignored while typing, in a sheet, or with a modifier held.
private struct StudyKeys: NSViewRepresentable {
    let app: AppModel

    func makeNSView(context: Context) -> NSView {
        let v = NSView()
        context.coordinator.monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak v] e in
            guard let w = v?.window, e.window === w, w.attachedSheet == nil else { return e }
            if !e.modifierFlags.intersection([.command, .control, .option]).isEmpty { return e }
            if w.firstResponder is NSText || w.firstResponder is NSDatePicker { return e }
            return MainActor.assumeIsolated { handle(e) } ? nil : e
        }
        return v
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        if let m = coordinator.monitor { NSEvent.removeMonitor(m) }
    }

    func makeCoordinator() -> Coordinator { Coordinator() }
    final class Coordinator { var monitor: Any? }

    @MainActor private func handle(_ e: NSEvent) -> Bool {
        let s = app.session
        let key = e.charactersIgnoringModifiers?.lowercased() ?? ""
        let run = { (m: String, a: [Any?]) in app.perform { _ = try await Engine.raw(m, a) } }
        switch key {
        case " " where s?.timing == true:
            app.togglePause()
        case "n" where s != nil:
            run("sessionSkip", [])
        case "1", "2", "3", "4":
            guard s?.phase == "rate" || s?.phase == "work" else { return false }
            app.pending.rating = Rating.allCases[Int(key)! - 1]
            app.screen = .session
        case "\r" where s?.phase == "rate" && app.pending.rating != nil:
            app.rate()
        case "\r" where s?.phase == "next" && s?.proposal != nil:
            run("sessionAccept", [s!.proposal!.topicId])
        case "x" where (s?.phase == "next" || s?.phase == "break") && s?.proposal != nil:
            run("sessionSkipSuggestion", [])
        case "s":
            app.screen = .session
            if s == nil { app.quickstart += 1 }
        default:
            return false
        }
        return true
    }
}
