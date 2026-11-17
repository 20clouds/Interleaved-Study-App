// The Library: a shelf of course cards; opening a course (or a lesson or unit from the sidebar) lists its lessons,
// units and topics in an outline table with the topic inspector beside it. Search and filters are in the toolbar.
import SwiftUI

/// One row of the outline: a lesson, a unit inside it, or a topic.
enum LibraryRow: Identifiable, Hashable {
    case lesson(Doc), unit(docId: String, name: String, count: Int), topic(Topic)
    var id: String {
        switch self {
        case .lesson(let d): "lesson:\(d.id)"
        case .unit(let d, let n, _): "unit:\(d):\(n)"
        case .topic(let t): t.id
        }
    }
}

private enum DueFilter: String, CaseIterable, Identifiable {
    case any, overdue, today, week
    var id: String { rawValue }
    var label: String {
        switch self {
        case .any: "Any due date"
        case .overdue: "Overdue"
        case .today: "Due by tonight"
        case .week: "Due within 7 days"
        }
    }
}

struct LibraryView: View {
    @Environment(AppModel.self) private var app
    @State private var query = ""
    @State private var hits: Set<String>?
    @State private var status: Status?
    @State private var due: DueFilter = .any
    @State private var selection: Set<String> = []
    @State private var adding = false
    @State private var renaming: Rename?

    var body: some View {
        @Bindable var app = app
        content
            .searchable(text: $query, placement: .toolbar, prompt: "Topics, terms and text")
            .task(id: "\(query)|\(app.dataVersion)") {
                let q = query.trimmingCharacters(in: .whitespaces)
                guard !q.isEmpty else { hits = nil; return }
                try? await Task.sleep(for: .milliseconds(160))
                guard !Task.isCancelled else { return }
                hits = Set((try? await Engine.call("search", q) as [String]) ?? [])
            }
            .toolbar { toolbar }
            // The details pane belongs to a list of topics; the course shelf has nothing to select.
            .inspector(isPresented: Binding(get: { app.library.inspector && showsList }, set: { app.library.inspector = $0 })) {
                InspectorView().inspectorColumnWidth(min: 300, ideal: 360, max: 520)
            }
            .sheet(isPresented: $adding) { AddTopicSheet() }
            .alert(renaming?.title ?? "", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } }), presenting: renaming) { r in
                RenameField(initial: r.value) { v in
                    let rename = r
                    renaming = nil
                    guard !v.isEmpty, v != rename.value else { return }
                    app.perform { try await Engine.run(rename.method, rename.id, v) }
                }
            }
            .onChange(of: selection) {
                let topics = selection.filter { app.topics[$0] != nil }
                if topics.count == 1 { app.selected = topics.first }
            }
            .onChange(of: app.selected) { if let s = app.selected, !selection.contains(s) { selection = [s] } }
    }

    private struct Rename: Identifiable {
        let id: String, method: String, title: String, value: String
    }

    private var filtering: Bool { hits != nil || status != nil || due != .any }
    private var showsList: Bool { !(app.data?.courses.isEmpty ?? true) && (app.scope != .all || filtering) }

    @ViewBuilder private var content: some View {
        if let d = app.data {
            if d.courses.isEmpty {
                EmptyState(title: "Create your first course", message: "A course is a folder for one subject. Give it a name and a colour, then upload lecture files or a whole folder, or start empty and add them later. Each file becomes a lesson. PDF and DOCX work on a best-effort basis; .md gives the cleanest topics. You can also drop files anywhere in this window.") {
                    HStack {
                        Button { app.courseWizard = [] } label: { Label("New Course", systemImage: "folder.badge.plus") }.buttonStyle(.primary)
                        Button { adding = true } label: { Label("Add a Topic by Hand", systemImage: "plus") }
                    }
                }
            } else if app.scope == .all && !filtering {
                Shelf(adding: $adding)
            } else {
                VStack(spacing: 0) {
                    if let cid = app.openCourse { CourseHead(courseId: cid, adding: $adding) { renaming = Rename(id: cid, method: "renameCourse", title: "Rename course", value: app.courses[cid]?.name ?? "") } }
                    list(d)
                }
                .background(Ink.paper)
            }
        }
    }

    // MARK: Rows

    private func visible(_ t: Topic) -> Bool {
        let now = app.now
        switch app.scope {
        case .all: break
        case .course(let id): if t.courseId != id { return false }
        case .lesson(let id): if t.docId != id { return false }
        case .unit(let id, let u): if t.docId != id || t.unit != u { return false }
        }
        if let hits, !hits.contains(t.id) { return false }
        if let status, t.status(now) != status { return false }
        let endToday = Calendar.current.startOfDay(for: Date()).addingTimeInterval(86_400).ms - 1
        switch due {
        case .any: return true
        case .overdue: return t.isDue(now)
        case .today: return (t.due ?? .infinity) <= endToday
        case .week: return (t.due ?? .infinity) <= now + 7 * DAY
        }
    }

    private struct LessonGroup: Identifiable {
        let doc: Doc
        let units: [(name: String, topics: [Topic])]
        var id: String { doc.id }
        var topics: [Topic] { units.flatMap(\.topics) }
    }

    private func groups(_ d: DataSnapshot) -> [LessonGroup] {
        let shown = d.topics.filter(visible).sorted { $0.ord < $1.ord }
        let byDoc = Dictionary(grouping: shown, by: \.docId)
        let courses: [String] = app.openCourse.map { [$0] } ?? d.courses.map(\.id)
        return courses.flatMap { app.lessons(of: $0) }.compactMap { doc in
            let topics = byDoc[doc.id] ?? []
            // Without filters, a lesson in scope shows even when it has no topics yet.
            let inScope: Bool = switch app.scope {
            case .all: true
            case .course(let id): doc.courseId == id
            case .lesson(let id), .unit(let id, _): doc.id == id
            }
            guard !topics.isEmpty || (!filtering && inScope) else { return nil }
            var seen: [String] = []
            for t in topics where !seen.contains(t.unit) { seen.append(t.unit) }
            return LessonGroup(doc: doc, units: seen.map { u in (u, topics.filter { $0.unit == u }) })
        }
    }

    private func lessonOpen(_ id: String) -> Binding<Bool> {
        Binding(
            get: {
                if filtering { return true }
                switch app.scope {
                case .lesson(let d), .unit(let d, _): if d == id { return true }
                default: break
                }
                return app.library.openLessons.contains(id)
            },
            set: { if $0 { app.library.openLessons.insert(id) } else { app.library.openLessons.remove(id) } })
    }

    private func unitOpen(_ key: String) -> Binding<Bool> {
        Binding(get: { !app.library.closedUnits.contains(key) }, set: { if $0 { app.library.closedUnits.remove(key) } else { app.library.closedUnits.insert(key) } })
    }

    @ViewBuilder private func list(_ d: DataSnapshot) -> some View {
        @Bindable var app = app
        let groups = groups(d)
        if groups.isEmpty {
            if let cid = app.openCourse, !filtering, !d.docs.contains(where: { $0.courseId == cid }) {
                EmptyState(title: "No lessons yet", message: "Upload lecture files or a whole folder; each file becomes a lesson in \(app.courses[cid]?.name ?? "this course"). You can also drop them anywhere in this window.") {
                    HStack {
                        Button { Task { await app.importer.pick("files", course: cid, app: app) } } label: { Label("Upload Files", systemImage: "doc") }.buttonStyle(.primary)
                        Button { Task { await app.importer.pick("folder", course: cid, app: app) } } label: { Label("Upload a Folder", systemImage: "folder") }
                        Button { adding = true } label: { Label("Add a Topic by Hand", systemImage: "plus") }
                    }
                }
            } else {
                EmptyState(title: query.isEmpty ? "Nothing matches these filters" : "No topics match this search", message: "Clear the search or filters to see all topics in this scope.") { EmptyView() }
            }
        } else {
            Table(of: LibraryRow.self, selection: $selection, columnCustomization: $app.library.columns) {
                columns
            } rows: {
                ForEach(groups) { g in
                    DisclosureTableRow(.lesson(g.doc), isExpanded: lessonOpen(g.doc.id)) {
                        if g.units.count > 1 {
                            ForEach(g.units, id: \.name) { u in
                                DisclosureTableRow(.unit(docId: g.doc.id, name: u.name, count: u.topics.count), isExpanded: unitOpen("u:\(g.doc.id):\(u.name)")) {
                                    ForEach(u.topics) { TableRow(LibraryRow.topic($0)) }
                                }
                            }
                        } else {
                            ForEach(g.topics) { TableRow(LibraryRow.topic($0)) }
                        }
                    }
                    .draggable(g.doc.id)
                    .dropDestination(for: String.self) { ids in moveLessons(ids, before: g.doc) }
                }
            }
            .contextMenu(forSelectionType: String.self) { ids in menu(ids) } primaryAction: { ids in
                if let id = ids.first, let doc = docOf(id) { lessonOpen(doc.id).wrappedValue.toggle() }
            }
            .environment(\.defaultMinListRowHeight, rowHeight)
            .alternatingRowBackgrounds(.disabled)
            .scrollContentBackground(.hidden)
        }
    }

    private var rowHeight: CGFloat {
        switch app.library.density { case .compact: 24; case .cozy: 30; case .roomy: 36 }
    }

    @TableColumnBuilder<LibraryRow, Never> private var columns: some TableColumnContent<LibraryRow, Never> {
        TableColumn("Topic") { row in NameCell(row: row) }
            .width(min: 200, ideal: 280)
            .customizationID("name")
            .disabledCustomizationBehavior(.visibility)
        TableColumn("Status") { row in
            switch row {
            case .topic(let t): StatusChip(status: t.status(app.now))
            case .lesson(let d): DocState(doc: d)
            case .unit: EmptyView()
            }
        }
            .width(min: 80, ideal: 150)
            .customizationID("status")
        TableColumn("Time") { row in if case .topic(let t) = row { Text("\(Int(t.minutes))m").monospacedDigit() } }
            .width(min: 40, ideal: 48)
            .alignment(.trailing)
            .customizationID("time")
        TableColumn("Spent") { row in if case .topic(let t) = row, t.spent > 0 { Text(fmtMin(t.spent)).monospacedDigit().foregroundStyle(.secondary) } }
            .width(min: 44, ideal: 56)
            .alignment(.trailing)
            .customizationID("spent")
        TableColumn("Last") { row in if case .topic(let t) = row { Text(fmtRel(t.last, app.now)).monospacedDigit().foregroundStyle(.secondary) } }
            .width(min: 50, ideal: 64)
            .alignment(.trailing)
            .customizationID("last")
        TableColumn("Next due") { row in if case .topic(let t) = row { Text(fmtRel(t.due, app.now)).monospacedDigit().foregroundStyle(t.isDue(app.now) ? Ink.due : .primary) } }
            .width(min: 60, ideal: 76)
            .alignment(.trailing)
            .customizationID("due")
        TableColumn("Mastery") { row in MasteryCell(row: row) }
            .width(min: 96, ideal: 104)
            .customizationID("mastery")
    }

    // MARK: Context menu

    private func docOf(_ rowId: String) -> Doc? {
        rowId.hasPrefix("lesson:") ? app.docs[String(rowId.dropFirst(7))] : nil
    }

    @ViewBuilder private func menu(_ ids: Set<String>) -> some View {
        let topics = ids.compactMap { app.topics[$0] }
        if ids.count == 1, let doc = ids.first.flatMap(docOf) {
            Button("Rename Lesson…") { renaming = Rename(id: doc.id, method: "renameDoc", title: "Rename lesson", value: doc.title) }
            if doc.status == "awaiting" { Button("Organize with AI…") { Task { await app.importer.estimate([doc.id], app: app) } } }
            if doc.status == "needs" || doc.status == "error" { Button("Retry Organizing") { app.perform("Retrying") { try await Engine.run("retryDoc", doc.id) } } }
            if doc.ext != "manual" && (doc.status == "organized" || doc.status == "needs") { Button("Re-organize with AI…") { reorganize(doc) } }
            Divider()
            Button("Delete Lesson…", role: .destructive) {
                app.confirmThen("Delete “\(doc.title)”?", detail: "This removes the lesson, its topics and their study history. Export a backup first if unsure.", ok: "Delete Lesson", danger: true) {
                    try await Engine.run("deleteDoc", doc.id)
                }
            }
        } else if !topics.isEmpty {
            if topics.count == 1, let t = topics.first {
                Button("Study Now") { Task { await studyNow(t) } }
                Button("Rename Topic…") { renaming = Rename(id: t.id, method: "renameTopic", title: "Rename topic", value: t.name) }
            }
            if topics.count >= 2 {
                Button("Merge \(topics.count) Topics…") {
                    app.confirmThen("Merge \(topics.count) topics?", detail: "The text is joined in lesson order. The study history of the most-reviewed topic is kept; time spent is added up.", ok: "Merge") {
                        let id: String = try await Engine.call("mergeTopics", topics.map(\.id))
                        selection = [id]
                        app.selected = id
                    }
                }
            }
            Divider()
            Button(topics.count == 1 ? "Delete Topic…" : "Delete \(topics.count) Topics…", role: .destructive) {
                app.confirmThen(topics.count == 1 ? "Delete “\(topics[0].name)”?" : "Delete \(topics.count) topics?", detail: "Their study history is removed too.", ok: "Delete", danger: true) {
                    try await Engine.run("deleteTopics", topics.map(\.id))
                    selection = []
                }
            }
        }
    }

    private func reorganize(_ d: Doc) {
        Task {
            let n: Int = (try? await Engine.call("historyCount", d.id)) ?? 0
            let run = { try await Engine.run("reorganize", d.id); app.say("Re-organizing with AI. History carries over to matching topics.") }
            if n > 0 {
                app.confirmThen("Re-organize “\(d.title)”?", detail: "\(n) topic(s) in this lesson have study history. The AI will rebuild the topic list; history moves to new topics whose name or text still matches, and unmatched topics start fresh.", ok: "Re-organize", run)
            } else {
                await app.act(nil, run)
            }
        }
    }

    private func studyNow(_ t: Topic) async {
        if let s = app.session {
            app.screen = .session
            if s.phase == "next" || s.phase == "break" { await app.act { try await Engine.run("sessionAccept", t.id) } }
            return
        }
        await app.startSession(pool: .topic([t.id]), label: t.name, topicId: t.id)
    }

    /// Lessons dragged onto another lesson go just before it.
    private func moveLessons(_ ids: [String], before target: Doc) {
        let cid = target.courseId
        var order = app.lessons(of: cid).map(\.id)
        let moving = ids.filter { id in order.contains(id) && id != target.id }
        guard !moving.isEmpty else { return }
        order.removeAll { moving.contains($0) }
        order.insert(contentsOf: moving, at: order.firstIndex(of: target.id) ?? order.endIndex)
        app.library.lessonOrder[cid] = order
    }

    // MARK: Toolbar

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
        ToolbarItemGroup {
            Menu {
                Picker("Status", selection: $status) {
                    Text("Any status").tag(Status?.none)
                    ForEach(Status.allCases) { Text($0.label).tag(Status?.some($0)) }
                }
                Picker("Due", selection: $due) { ForEach(DueFilter.allCases) { Text($0.label).tag($0) } }
                if filtering { Divider(); Button("Clear Filters") { status = nil; due = .any } }
            } label: {
                Label("Filter", systemImage: filtering && hits == nil ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
            }
            .help("Filter by status or due date")
            Menu {
                Picker("Density", selection: Binding(get: { app.library.density }, set: { app.library.density = $0 })) {
                    Text("Compact").tag(LibraryPrefs.Density.compact)
                    Text("Cozy").tag(LibraryPrefs.Density.cozy)
                    Text("Roomy").tag(LibraryPrefs.Density.roomy)
                }
                Picker("Topic Text", selection: Binding(get: { app.library.topicSize }, set: { app.library.topicSize = $0 })) {
                    ForEach([12, 13, 13.5, 14, 15, 16, 17], id: \.self) { Text("\($0.formatted()) pt").tag($0) }
                }
                Divider()
                Toggle("Topic Counts on Lessons", isOn: Binding(get: { app.library.counts }, set: { app.library.counts = $0 }))
                Toggle("Lecture File Next to Topics", isOn: Binding(get: { app.library.deck }, set: { app.library.deck = $0 }))
                Divider()
                Text("Right-click a column heading to show or hide columns.")
            } label: {
                Label("View Options", systemImage: "slider.horizontal.3")
            }
            .help("Customize this view")
        }
        if showsList { ToolbarItem {
            Button { app.library.inspector.toggle() } label: { Label("Details", systemImage: "sidebar.trailing") }
                .help(app.library.inspector ? "Hide the details pane" : "Show the details pane")
        } }
    }
}

// MARK: - Cells

private struct NameCell: View {
    @Environment(AppModel.self) private var app
    let row: LibraryRow
    var body: some View {
        switch row {
        case .lesson(let d):
            let topics = (app.data?.topics ?? []).filter { $0.docId == d.id }
            let due = topics.filter { $0.isDue(app.now) }.count
            HStack(spacing: 10) {
                if app.openCourse == nil { Dot(color: app.color(d.courseId), size: 7) }
                Text(d.title).font(.system(size: app.library.topicSize + 1, weight: .semibold)).lineLimit(1)
                if !["md", "txt", "markdown"].contains(d.ext), let files = app.fileSupport, let dl = Delivery.of(d, files) { DeliveryTag(delivery: dl, compact: true) }
                if app.library.counts && !topics.isEmpty { Text(plural(topics.count, "topic")).font(Typeset.small.monospacedDigit()).foregroundStyle(.secondary) }
                if due > 0 { Text("\(due) due").font(Typeset.small.monospacedDigit()).foregroundStyle(Ink.due) }
            }
        case .unit(_, let name, let count):
            HStack(spacing: 8) {
                Text(name).font(.system(size: app.library.topicSize, weight: .medium)).foregroundStyle(.secondary)
                if app.library.counts { Text("\(count)").font(Typeset.small.monospacedDigit()).foregroundStyle(.tertiary) }
            }
        case .topic(let t):
            HStack(spacing: 10) {
                Text(t.name)
                    .font(Typeset.serif(app.library.topicSize))
                    .italic(t.provisional)
                    .foregroundStyle(t.provisional ? HierarchicalShapeStyle.secondary : .primary)
                    .lineLimit(1)
                    .help(t.provisional ? "Provisional name until the AI organizes this lesson" : t.name)
                if app.library.deck { DeckLabel(doc: app.docs[t.docId]) }
            }
        }
    }
}

private struct MasteryCell: View {
    @Environment(AppModel.self) private var app
    let row: LibraryRow
    var body: some View {
        switch row {
        case .topic(let t):
            let m = t.mastery(app.now)
            MasteryBar(value: m, overdue: t.isDue(app.now) && m < 50)
        case .lesson(let d):
            let rows = (app.data?.topics ?? []).filter { $0.docId == d.id }
            if !rows.isEmpty { MasteryBar(value: rows.reduce(0) { $0 + $1.mastery(app.now) } / rows.count) }
        case .unit: EmptyView()
        }
    }
}

/// A lesson's organizing state, with its one action.
private struct DocState: View {
    @Environment(AppModel.self) private var app
    let doc: Doc
    var body: some View {
        switch doc.status {
        case "awaiting":
            HStack(spacing: 6) {
                Button("Organize") { Task { await app.importer.estimate([doc.id], app: app) } }.buttonStyle(.bordered).controlSize(.small)
                    .help("Not organized yet: the AI names its topics, groups units and writes summaries")
            }
            .font(Typeset.small)
        case "queued":
            let pct = doc.windows > 0 ? Double(doc.winsDone) / Double(doc.windows) : 0
            HStack(spacing: 6) {
                ProgressView(value: pct).frame(width: 40).controlSize(.small)
                Text("\(doc.winsDone)/\(doc.windows)").monospacedDigit().foregroundStyle(.secondary)
            }
            .font(Typeset.small)
        case "needs", "error":
            HStack(spacing: 6) {
                Text("Needs organizing").foregroundStyle(Ink.due).help(doc.error ?? "")
                Button { app.perform("Retrying") { try await Engine.run("retryDoc", doc.id) } } label: { Image(systemName: "arrow.clockwise") }
                    .buttonStyle(.borderless).help("Retry organizing")
            }
            .font(Typeset.small)
        default:
            EmptyView()
        }
    }
}

/// The text field inside a rename alert.
private struct RenameField: View {
    let initial: String
    let done: (String) -> Void
    @State private var text = ""
    var body: some View {
        TextField("Name", text: $text).onAppear { text = initial }
        Button("Rename") { done(text.trimmingCharacters(in: .whitespacesAndNewlines)) }
        Button("Cancel", role: .cancel) {}
    }
}

// MARK: - Course header

private struct CourseHead: View {
    @Environment(AppModel.self) private var app
    let courseId: String
    @Binding var adding: Bool
    let rename: () -> Void

    var body: some View {
        if let c = app.courses[courseId] {
            let topics = (app.data?.topics ?? []).filter { $0.courseId == c.id }
            let lessons = (app.data?.docs ?? []).filter { $0.courseId == c.id }.count
            let due = topics.filter { $0.isDue(app.now) }.count
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 4) {
                    Button { app.scope = .all } label: { Label("All Courses", systemImage: "chevron.left") }.buttonStyle(QuietLink())
                    if app.scope != .course(c.id) {
                        Image(systemName: "chevron.right").font(.system(size: 9)).foregroundStyle(Ink.ink3)
                        Button(c.name) { app.scope = .course(c.id) }.buttonStyle(QuietLink())
                    }
                }
                .font(Typeset.small)
                HStack(spacing: 10) {
                    RoundedRectangle(cornerRadius: 2).fill(app.color(c.id)).frame(width: 4, height: 22)
                    Text(c.name).font(Typeset.title).onTapGesture(count: 2, perform: rename).help("Double-click to rename")
                    CourseMenu(course: c, rename: rename)
                    Spacer()
                    Button { adding = true } label: { Label("Add Topic", systemImage: "plus") }
                    Menu {
                        Button("Files…") { Task { await app.importer.pick("files", course: c.id, app: app) } }
                        Button("A Folder…") { Task { await app.importer.pick("folder", course: c.id, app: app) } }
                        Divider()
                        Text("Each file becomes a lesson. You can also drop files anywhere in this window.")
                    } label: { Label("Upload", systemImage: "square.and.arrow.up") }
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .help("Upload lessons to \(c.name)")
                }
                (Text("\(plural(lessons, "lesson")), \(plural(topics.count, "topic"))") + (due > 0 ? Text(", ") + Text("\(due) due now").foregroundStyle(Ink.due) : Text("")))
                    .font(Typeset.small).foregroundStyle(Ink.ink2)
            }
            .padding(.horizontal, 20).padding(.vertical, 14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(alignment: .bottom) { Divider() }
        }
    }
}

/// Colour, rename and delete for a course: on its card, its header and in the sidebar.
struct CourseMenu: View {
    @Environment(AppModel.self) private var app
    let course: Course
    var rename: (() -> Void)?
    var body: some View {
        Menu {
            CourseMenuItems(course: course, rename: rename)
        } label: {
            Image(systemName: "ellipsis.circle")
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Course options")
    }
}

struct CourseMenuItems: View {
    @Environment(AppModel.self) private var app
    let course: Course
    var rename: (() -> Void)?
    var body: some View {
        Menu("Colour") {
            Button { app.library.colors[course.id] = nil } label: { Text("Default") }
            ForEach(Hue.swatches, id: \.self) { h in
                Button { app.library.colors[course.id] = .hue(h) } label: { Label { Text(Hue.name(h)) } icon: { Swatch(color: Hue.color(h)) } }
            }
            Button { app.library.colors[course.id] = .neutral } label: { Label { Text("Grey") } icon: { Swatch(color: Ink.ink3) } }
        }
        if let rename { Button("Rename…", action: rename) }
        Divider()
        Button("Delete Course…", role: .destructive) {
            app.confirmThen("Delete course “\(course.name)”?", detail: "This removes every lesson and topic in the course, with their history.", ok: "Delete Course", danger: true) {
                try await Engine.run("deleteCourse", course.id)
                if app.openCourse == course.id { app.scope = .all }
            }
        }
    }
}

/// A coloured circle that keeps its colour inside menus.
struct Swatch: View {
    let color: Color
    var body: some View {
        let size = NSSize(width: 12, height: 12)
        let img = NSImage(size: size, flipped: false) { r in
            NSColor(color).setFill()
            NSBezierPath(ovalIn: r.insetBy(dx: 1, dy: 1)).fill()
            return true
        }
        img.isTemplate = false
        return Image(nsImage: img)
    }
}

extension Hue {
    static func name(_ h: Double) -> String {
        [25: "Red", 50: "Vermilion", 75: "Orange", 100: "Amber", 130: "Olive", 155: "Green", 180: "Jade", 200: "Teal",
         225: "Cyan", 250: "Blue", 275: "Indigo", 300: "Violet", 325: "Magenta", 350: "Rose"][h] ?? "Hue \(Int(h))"
    }
}

// MARK: - Shelf

/// Course cards: the Library's home.
private struct Shelf: View {
    @Environment(AppModel.self) private var app
    @Binding var adding: Bool

    var body: some View {
        let d = app.data!
        Page {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("Courses").font(Typeset.title)
                Text("\(plural(d.courses.count, "course")), \(plural(d.topics.count, "topic"))").foregroundStyle(Ink.ink2)
                Spacer()
                Button { adding = true } label: { Label("Add a Topic by Hand", systemImage: "plus") }.buttonStyle(.borderless)
            }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 220, maximum: 300), spacing: 20)], alignment: .leading, spacing: 20) {
                ForEach(d.courses) { CourseCard(course: $0) }
                Button { app.courseWizard = [] } label: {
                    VStack(alignment: .leading, spacing: 6) {
                        Image(systemName: "folder.badge.plus").font(.system(size: 18)).foregroundStyle(Ink.ink2)
                        Spacer()
                        Text("Add a course").font(.system(size: 15, weight: .semibold))
                        Text("Name it, pick a colour, add files").font(Typeset.small).foregroundStyle(Ink.ink2)
                    }
                    .padding(16)
                    .frame(maxWidth: .infinity, minHeight: 150, alignment: .topLeading)
                    .background(RoundedRectangle(cornerRadius: 8).strokeBorder(Ink.line2, style: .init(lineWidth: 1, dash: [5, 4])))
                    .contentShape(Rectangle())
                }
                .buttonStyle(HoverRow())
            }
        }
    }
}

private struct CourseCard: View {
    @Environment(AppModel.self) private var app
    let course: Course
    @State private var hover = false

    var body: some View {
        let topics = (app.data?.topics ?? []).filter { $0.courseId == course.id }
        let lessons = app.lessons(of: course.id)
        let due = topics.filter { $0.isDue(app.now) }.count
        let fresh = topics.filter { $0.status(app.now) == .new }.count
        let mastery = topics.isEmpty ? 0 : topics.reduce(0) { $0 + $1.mastery(app.now) } / topics.count
        let state = due > 0 ? "\(due) due now" : lessons.isEmpty ? "No lessons yet" : topics.isEmpty ? "Awaiting topics" : fresh == topics.count ? "Not started" : "Nothing due"
        Button { app.scope = .course(course.id) } label: {
            VStack(alignment: .leading, spacing: 4) {
                ForEach(lessons.prefix(3)) { d in
                    Text(d.deckName).font(Typeset.small).foregroundStyle(Ink.ink3).lineLimit(1)
                }
                Spacer(minLength: 12)
                Text(course.name).font(.system(size: 15, weight: .semibold)).foregroundStyle(Ink.ink).lineLimit(2)
                Text("\(plural(lessons.count, "lesson")), \(plural(topics.count, "topic"))").font(Typeset.small).foregroundStyle(Ink.ink2)
                HStack {
                    Text(state).font(Typeset.small).foregroundStyle(due > 0 ? Ink.due : Ink.ink2)
                    Spacer()
                    MasteryBar(value: mastery)
                }
                .padding(.top, 6)
            }
            .padding(16)
            .frame(maxWidth: .infinity, minHeight: 150, alignment: .topLeading)
            .background(RoundedRectangle(cornerRadius: 8).fill(hover ? Ink.hover : Ink.raised))
            .overlay(alignment: .top) { UnevenRoundedRectangle(topLeadingRadius: 8, topTrailingRadius: 8).fill(app.color(course.id)).frame(height: 4) }
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Ink.line, lineWidth: 0.5))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .animation(.quick, value: hover)
        .contextMenu { CourseMenuItems(course: course) }
        .accessibilityLabel("Open \(course.name)")
    }
}
