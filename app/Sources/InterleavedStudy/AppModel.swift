// App-wide state: the engine's data (refetched on change events), queue and session state pushed by the engine,
// and what must survive switching views (selection, scope, the pending rating).
import AppKit
import Observation
import SwiftUI
import UserNotifications

enum Screen: String, CaseIterable, Identifiable {
    case today, session, library, plan, progress, queue
    var id: String { rawValue }
    var title: String {
        switch self {
        case .today: "Today"
        case .session: "Study"
        case .library: "Library"
        case .plan: "Plan"
        case .progress: "Progress"
        case .queue: "AI Queue"
        }
    }
    var symbol: String {
        switch self {
        case .today: "sun.max"
        case .session: "timer"
        case .library: "books.vertical"
        case .plan: "calendar"
        case .progress: "chart.bar"
        case .queue: "square.stack.3d.up"
        }
    }
    var key: KeyEquivalent { KeyEquivalent(Character(String((Screen.allCases.firstIndex(of: self) ?? 0) + 1))) }
}

/// What the Library shows: everything, one course, one lesson, or one unit of a lesson.
enum Scope: Hashable {
    case all, course(String), lesson(String), unit(docId: String, unit: String)
}

struct Toast: Equatable {
    let message: String
    let error: Bool
}

@MainActor @Observable
final class AppModel {
    var boot: Bootstrap?
    var data: DataSnapshot?
    /// Bumped on every reload, so views that ask the engine for derived lists (suggestions) ask again.
    var dataVersion = 0
    var dataError: String?
    var queue: QueueStatus?
    var session: SessionState?
    var fileSupport: FileSupportTable?
    var now = nowMs
    /// Ticks every second while a block is timed, for the menu bar countdown.
    var clock = Date()

    var screen: Screen = Screen(rawValue: UserDefaults.standard.string(forKey: "view") ?? "") ?? .today {
        didSet { UserDefaults.standard.set(screen.rawValue, forKey: "view") }
    }
    var scope: Scope = .all
    var selected: String?
    var toast: Toast?
    /// The rating being chosen for the block that just ended.
    var pending = PendingRating()
    /// Bumped when a new block starts: the toolbar strip's one orchestrated move.
    var advance = 0
    /// Bumped to start the next suggested session from anywhere (S, the Study menu).
    var quickstart = 0
    var quickstartHandled = 0
    /// Files and folders waiting in the New course sheet; nil while it is closed.
    var courseWizard: [String]?
    var importer = ImportFlow()
    var library = LibraryPrefs.load() { didSet { library.save() } }

    var mode: String = UserDefaults.standard.string(forKey: "mode") ?? "timed" {
        didSet { UserDefaults.standard.set(mode, forKey: "mode") }
    }

    var settings: Settings? { boot?.settings }
    private(set) var courses: [String: Course] = [:]
    private(set) var docs: [String: Doc] = [:]
    private(set) var topics: [String: Topic] = [:]
    private var toastTask: Task<Void, Never>?
    private var lastTopic: String?

    struct PendingRating: Equatable {
        var rating: Rating?
        var quiz: Double?
        var note = ""
    }

    // MARK: Start

    func start() {
        Engine.onEvent = { [weak self] name, payload in self?.event(name, payload) }
        Task {
            await refreshBoot()
            await reload()
            queue = try? await Engine.call("queueStatus")
            setSession(try? await Engine.call("sessionState"))
        }
        // Mastery and "due" move with time; the clock itself redraws on its own.
        Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { _ in Task { @MainActor in self.now = nowMs } }
        Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { _ in Task { @MainActor in if self.session?.timing == true { self.clock = Date() } } }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { granted, error in
            if !granted { NSLog("Interleaved Study: notifications not allowed: %@", error?.localizedDescription ?? "turned off in System Settings") }
        }
    }

    private func event(_ name: String, _ payload: Data) {
        switch name {
        case "data": Task { await reload() }
        case "queue": queue = decode(QueueStatus.self, payload, name)
        case "session": setSession(decode(SessionState?.self, payload, name) ?? nil)
        case "alert":
            if let a = decode([String: String].self, payload, name) { alert(a["title"] ?? "", a["body"] ?? "") }
        default: break
        }
    }

    /// An engine event the app cannot read is a bug between the two halves: say so in the log.
    private func decode<T: Decodable>(_ t: T.Type, _ d: Data, _ what: String) -> T? {
        do { return try JSONDecoder().decode(t, from: d) } catch {
            NSLog("Interleaved Study: could not read the engine's %@ event: %@", what, String(describing: error))
            return nil
        }
    }

    private func setSession(_ s: SessionState?) {
        session = s
        // A new block: the strip advances and the pending rating starts over.
        let id = s?.phase == "work" ? s?.topicId : nil
        if let id, id != lastTopic {
            advance += 1
            pending = PendingRating()
        }
        lastTopic = s?.topicId
    }

    func reload() async {
        do {
            let d: DataSnapshot = try await Engine.call("snapshot")
            data = d
            courses = Dictionary(d.courses.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
            docs = Dictionary(d.docs.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
            topics = Dictionary(d.topics.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
            dataVersion += 1
            dataError = nil
            now = nowMs
            // A model can turn a file type down mid-run, and settings change elsewhere: refresh with the data.
            fileSupport = try? await Engine.call("fileSupport")
        } catch {
            dataError = error.localizedDescription
        }
    }

    func refreshBoot() async {
        guard let b: Bootstrap = try? await Engine.call("bootstrap") else { return }
        boot = b
        applyTheme()
        fileSupport = try? await Engine.call("fileSupport")
    }

    /// Save part of Settings: a nested dictionary in the engine's JSON shape.
    func saveSettings(_ patch: [String: Any]) {
        Task {
            do {
                let s: Settings = try await Engine.call("setSettings", patch)
                boot?.settings = s
                applyTheme()
                fileSupport = try? await Engine.call("fileSupport")
            } catch {
                show(error)
            }
        }
    }

    private func applyTheme() {
        NSApp.appearance = switch settings?.theme { case "light": NSAppearance(named: .aqua); case "dark": NSAppearance(named: .darkAqua); default: nil }
    }

    // MARK: Actions

    /// Run an engine action and report a failure (or the given success message) as a toast.
    @discardableResult
    func act<T>(_ ok: String? = nil, _ fn: @escaping () async throws -> T) async -> T? {
        do {
            let r = try await fn()
            if let ok { say(ok) }
            return r
        } catch {
            show(error)
            return nil
        }
    }

    @discardableResult
    func act<T>(_ fn: @escaping () async throws -> T) async -> T? { await act(nil, fn) }

    /// The same, started from a button.
    func perform(_ ok: String? = nil, _ fn: @escaping () async throws -> Void) {
        Task { await act(ok, fn) }
    }

    /// Ask in a sheet on the window first.
    func confirmThen(_ message: String, detail: String, ok: String, danger: Bool = false, done: String? = nil, _ fn: @escaping () async throws -> Void) {
        Task {
            guard await Host.confirm(message: message, detail: detail, ok: ok, danger: danger) else { return }
            await act(done, fn)
        }
    }

    func say(_ message: String) { showToast(Toast(message: message, error: false)) }
    func show(_ error: Error) { showToast(Toast(message: error.localizedDescription, error: true)) }

    private func showToast(_ t: Toast) {
        toast = t
        toastTask?.cancel()
        toastTask = Task {
            try? await Task.sleep(for: .seconds(t.error ? 7 : 3.2))
            if !Task.isCancelled { toast = nil }
        }
    }

    // MARK: Session

    func startSession(pool: Pool = .global, label: String = "Everything", topicId: String? = nil) async {
        let opts: [String: Any?] = ["mode": mode, "pool": pool, "poolLabel": label, "topicId": topicId]
        await act { try await Engine.run("sessionStart", opts) }
        screen = .session
    }

    func togglePause() {
        guard let s = session, s.timing else { return }
        perform { try await Engine.run(s.paused ? "sessionResume" : "sessionPause") }
    }

    func rate() {
        let p = pending
        perform { try await Engine.run("sessionRate", ["rating": p.rating?.rawValue, "quiz": p.quiz, "note": p.note.isEmpty ? nil : p.note] as [String: Any?]) }
    }

    func endSession() {
        let sandbox = session?.sandbox == true
        confirmThen("End this session?",
                    detail: "Time already spent on the current topic is recorded without a rating." + (sandbox ? " You can then save the sandbox to your schedule or discard it." : ""),
                    ok: "End Session") { try await Engine.run("sessionEnd") }
    }

    /// Minutes a topic gets in the current mode.
    func perTopic(_ t: Topic) -> Double {
        guard let s = settings else { return t.minutes }
        if mode == "pomodoro" { return s.pomo.work }
        return s.study.useSuggested ? t.minutes : s.study.minutes
    }

    // MARK: Lookups

    func color(_ courseId: String?) -> Color {
        guard let id = courseId, let c = courses[id] else { return Ink.ink3 }
        return library.colors[id]?.color ?? Hue.color(Hue.palette[((c.color % 8) + 8) % 8])
    }

    func courseOf(_ d: Deadline) -> Course? {
        let id = d.courseId ?? d.docId.flatMap { docs[$0]?.courseId } ?? d.topicIds?.first.flatMap { topics[$0]?.courseId }
        return id.flatMap { courses[$0] }
    }

    /// The course open in the Library, if any: dropped files go into it.
    var openCourse: String? {
        switch scope {
        case .all: nil
        case .course(let id): id
        case .lesson(let d), .unit(let d, _): docs[d]?.courseId
        }
    }

    /// A course's lessons, in the order the student arranged them.
    func lessons(of courseId: String) -> [Doc] {
        let mine = (data?.docs ?? []).filter { $0.courseId == courseId }
        let order = library.lessonOrder[courseId] ?? []
        return mine.sorted { (order.firstIndex(of: $0.id) ?? .max) < (order.firstIndex(of: $1.id) ?? .max) }
    }

    /// A lesson's units in lesson order.
    func units(of docId: String) -> [String] {
        var seen = Set<String>()
        return (data?.topics ?? []).filter { $0.docId == docId }.sorted { $0.ord < $1.ord }.map(\.unit).filter { seen.insert($0).inserted }
    }

    var dueCount: Int { data?.topics.filter { $0.isDue(now) }.count ?? 0 }

    // MARK: Notifications

    /// A system notification, plus a Dock bounce and badge when the window is not in front, as Settings say.
    private func alert(_ title: String, _ body: String) {
        let n = settings?.notify
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)) { error in
            if let error { NSLog("Interleaved Study: the notification was not shown: %@", error.localizedDescription) }
        }
        // The chosen system sound plays here: a notification can only name sounds inside the app.
        if let sound = n?.sound, sound != "none" { NSSound(named: sound)?.play() }
        guard !NSApp.isActive else { return }
        if n?.bounce ?? true { NSApp.requestUserAttention(.informationalRequest) }
        if n?.badge ?? true { NSApp.dockTile.badgeLabel = "1" }
    }
}

/// Library display choices, kept per Mac.
struct LibraryPrefs: Codable, Equatable {
    enum Density: String, Codable, CaseIterable { case compact, cozy, roomy }
    var density: Density = .roomy
    var topicSize: Double = 13.5
    var counts = true
    var deck = true
    var inspector = true
    var columns = TableColumnCustomization<LibraryRow>()
    /// Lessons opened in the list (closed until opened) and units folded shut.
    var openLessons: Set<String> = []
    var closedUnits: Set<String> = []
    /// Course id → the colour picked for it (the palette colour when absent).
    var colors: [String: Tint] = [:]
    /// Course id → lesson ids in the order the student arranged them.
    var lessonOrder: [String: [String]] = [:]

    static func load() -> LibraryPrefs {
        guard let d = UserDefaults.standard.data(forKey: "library"), let p = try? JSONDecoder().decode(LibraryPrefs.self, from: d) else { return LibraryPrefs() }
        return p
    }
    func save() { UserDefaults.standard.set(try? JSONEncoder().encode(self), forKey: "library") }
}
