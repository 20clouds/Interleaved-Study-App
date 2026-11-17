// Debug builds only: with INTERLEAVED_STUDY_SNAPSHOTS=<folder>, the app opens each view in light and dark and saves
// a PNG of its window there, so the interface can be checked without screen recording.
#if DEBUG
import AppKit

@MainActor
enum Snapshots {
    static func run(_ app: AppModel) {
        guard let dir = ProcessInfo.processInfo.environment["INTERLEAVED_STUDY_SNAPSHOTS"] else { return }
        NSLog("snapshots: to %@", dir)
        Task {
            try? await Task.sleep(for: .seconds(2))
            NSLog("snapshots: starting")
            for look in ["light", "dark"] {
                NSApp.appearance = NSAppearance(named: look == "dark" ? .darkAqua : .aqua)
                for s in Screen.allCases {
                    app.screen = s
                    try? await Task.sleep(for: .seconds(1.5))
                    save("\(dir)/\(look)-\(s.rawValue).png")
                }
                // A course open in the Library, with a topic in the inspector.
                if let t = app.data?.topics.first {
                    app.screen = .library
                    app.scope = .course(t.courseId)
                    app.library.openLessons.insert(t.docId)
                    app.selected = t.id
                    try? await Task.sleep(for: .seconds(1.5))
                    save("\(dir)/\(look)-library-course.png")
                    app.scope = .all
                }
            }
            NSApp.terminate(nil)
        }
    }

    private static func save(_ path: String) {
        NSLog("snapshot windows: %@", NSApp.windows.map { "\($0.className) \($0.frame) visible=\($0.isVisible)" }.joined(separator: "; "))
        guard let w = NSApp.windows.first(where: { $0.contentView != nil && $0.frame.width > 600 }),
              let view = w.contentView?.superview ?? w.contentView,
              let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
    }
}
#endif
