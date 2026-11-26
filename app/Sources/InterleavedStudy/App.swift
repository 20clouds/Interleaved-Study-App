// Interleaved Study: one main window, a Settings window, the menu bar timer and the menus. The Rust engine starts
// before any of them and keeps running while the window is closed, as a Mac app does.
import AppKit
import SwiftUI
import UserNotifications

@main
struct InterleavedStudyApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var app: AppModel

    init() {
        let dir = Self.libraryFolder()
        if !Engine.start(dataDir: dir) {
            let a = NSAlert()
            a.messageText = "Interleaved Study could not open its library."
            a.informativeText = "The database in \(dir.path) could not be opened. Check that the folder exists and can be written to."
            a.runModal()
            exit(1)
        }
        let model = AppModel()
        _app = State(initialValue: model)
        model.start()
        #if DEBUG
        Snapshots.run(model)
        #endif
    }

    /// The library in Application Support. Only a debug build may take another from INTERLEAVED_STUDY_DATA: a library
    /// planted through the environment could name any https address for the AI, and the app would send it the
    /// Keychain's keys without asking.
    private static func libraryFolder() -> URL {
        #if DEBUG
        if let env = ProcessInfo.processInfo.environment["INTERLEAVED_STUDY_DATA"], !env.trimmingCharacters(in: .whitespaces).isEmpty {
            return URL(fileURLWithPath: env)
        }
        #endif
        return FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Interleaved Study 2")
    }

    var body: some Scene {
        Window("Interleaved Study", id: "main") {
            RootView()
                .environment(app)
                .frame(minWidth: 960, minHeight: 620)
        }
        .defaultSize(width: 1320, height: 860)
        .commands { AppCommands(app: app) }

        SwiftUI.Settings {
            SettingsView().environment(app)
        }

        MenuBarExtra {
            MenuBarMenu().environment(app)
        } label: {
            MenuBarLabel().environment(app)
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    func applicationWillFinishLaunching(_ notification: Notification) { UNUserNotificationCenter.current().delegate = self }
    /// Show the banner even while the app is in front: "Time's up" matters most while you are reading in it.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions { [.banner, .list] }

    /// Closing the window keeps the app, and the menu bar timer, running.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    func applicationDidBecomeActive(_ notification: Notification) { NSApp.dockTile.badgeLabel = nil }
    func applicationWillTerminate(_ notification: Notification) { Engine.stop() }
}

/// The menu bar: the time left in the block (or what the session waits for) beside the app's mark.
struct MenuBarLabel: View {
    @Environment(AppModel.self) private var app
    var body: some View {
        // Redrawn by the model's one-second clock: a TimelineView inside a menu bar label loops forever.
        HStack(spacing: 4) {
            Image(nsImage: Self.mark)
            if let t = title(app.clock) { Text(t).monospacedDigit() }
        }
    }

    private static let mark: NSImage = {
        let img = Bundle.main.image(forResource: "trayTemplate") ?? NSImage(systemSymbolName: "timer", accessibilityDescription: nil)!
        img.isTemplate = true
        return img
    }()

    private func title(_ now: Date) -> String? {
        guard let s = app.session else { return nil }
        if s.timing && s.paused { return "Paused" }
        switch s.phase {
        case "work": return fmtClock(s.left(now))
        case "break": return "Break \(fmtClock(s.left(now)))"
        case "rate": return "Rate"
        case "next": return "Next"
        case "done": return "Save?"
        default: return nil
        }
    }
}

struct MenuBarMenu: View {
    @Environment(AppModel.self) private var app
    @Environment(\.openWindow) private var openWindow
    var body: some View {
        if let s = app.session, !s.topicName.isEmpty { Text(s.topicName) }
        Button("Open Interleaved Study") { show() }
        if let s = app.session {
            Divider()
            if s.timing { Button(s.paused ? "Resume" : "Pause") { app.togglePause() } }
            if s.phase == "work" { Button("Skip Topic") { app.perform { try await Engine.run("sessionSkip") } } }
            if s.phase == "rate" || s.phase == "next" || s.phase == "done" { Button("Open the Session") { app.screen = .session; show() } }
        } else if !(app.data?.topics.isEmpty ?? true) {
            Button("Start Next Suggested") { app.screen = .session; app.quickstart += 1; show() }
        }
        Divider()
        Button("Quit Interleaved Study") { NSApp.terminate(nil) }.keyboardShortcut("q")
    }

    private func show() {
        openWindow(id: "main")
        NSApp.activate()
    }
}

struct AppCommands: Commands {
    let app: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("Import Lessons…") { pick("files") }.keyboardShortcut("o")
            Button("Import Folder…") { pick("folder") }.keyboardShortcut("o", modifiers: [.command, .shift])
            Divider()
            Button("Export Backup…") { backup("exportBackup") }
            Button("Restore from Backup…") { backup("importBackup") }
        }
        CommandGroup(before: .sidebar) {
            ForEach(Screen.allCases) { s in
                Button(s.title) { go(s) }.keyboardShortcut(s.key)
            }
            Divider()
        }
        CommandMenu("Study") {
            Button("Start Next Suggested Session") { go(.session); if app.session == nil { app.quickstart += 1 } }
                .keyboardShortcut("s", modifiers: [.command, .shift])
            Button("Pause or Resume Timer") { app.togglePause() }
                .keyboardShortcut("p", modifiers: [.command, .shift])
                .disabled(!(app.session?.timing ?? false))
            Button("End Session…") { app.endSession() }
                .disabled(app.session == nil || app.session?.phase == "done")
        }
    }

    private func go(_ s: Screen) {
        app.screen = s
        openWindow(id: "main")
    }

    /// The picked files go wherever dropped files would.
    private func pick(_ kind: String) {
        openWindow(id: "main")
        Task {
            guard let paths: [String] = await app.act({ try await Engine.call("pickPaths", kind) }), !paths.isEmpty else { return }
            app.importer.drop(paths, app: app)
        }
    }

    private func backup(_ method: String) {
        openWindow(id: "main")
        Task {
            do {
                let data = try await Engine.raw(method, [])
                if method == "exportBackup", let path = try? JSONDecoder().decode(String.self, from: data) { app.say("Backup saved to \(path)") }
                if method == "importBackup", let r = try? JSONDecoder().decode([String: JSONValue].self, from: data) {
                    if r["ok"] == .bool(false) { throw EngineError(message: r["error"]?.text ?? "The backup could not be restored.") }
                    await app.refreshBoot()
                    app.say("Backup restored")
                }
            } catch {
                app.show(error)
            }
        }
    }
}
