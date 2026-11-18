// The topic inspector: why it is due, the AI's summary, terms and prerequisites, its source text in the lesson (the
// AI's exact cut when there is one), history, notes, every override, split and delete, a quiz and "Study Now".
import SwiftUI

struct InspectorView: View {
    @Environment(AppModel.self) private var app
    /// The pane runs up under the toolbar; its content starts below it, where the page's does.
    let toolbarHeight: CGFloat

    var body: some View {
        if let id = app.selected, let t = app.topics[id] {
            TopicDetail(t: t, toolbarHeight: toolbarHeight).id(t.id)
        } else {
            Text("Select a topic to see why it is due, its history and overrides.")
                .font(Typeset.body).foregroundStyle(Ink.ink2).multilineTextAlignment(.center)
                .padding(24).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

private enum PushUnit: String, CaseIterable, Identifiable {
    case minutes, hours, days, months
    var id: String { rawValue }
}

private struct TopicDetail: View {
    @Environment(AppModel.self) private var app
    let t: Topic
    let toolbarHeight: CGFloat

    @State private var notes = ""
    @State private var source: TopicText?
    @State private var showAll = false
    @State private var splitAt: Int?
    @State private var splitName = ""
    @State private var dueAt = Date()
    @State private var push = 3.0
    @State private var pushUnit: PushUnit = .days
    @State private var score = 50.0
    @State private var quiz = false
    @FocusState private var notesFocused: Bool
    // Which of the longer sections are open, remembered across topics.
    @AppStorage("inspector.source") private var sourceOpen = false
    @AppStorage("inspector.schedule") private var scheduleOpen = false
    @AppStorage("inspector.history") private var historyOpen = false

    var body: some View {
        let d = app.docs[t.docId]
        let now = app.now
        let m = t.mastery(now)
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 6) {
                    Text([app.courses[t.courseId]?.name, d?.title, t.unit != d?.title && !t.unit.isEmpty ? t.unit : nil].compactMap { $0 }.joined(separator: " / "))
                        .font(Typeset.small).foregroundStyle(Ink.ink2).lineLimit(1)
                    Text(t.name).font(Typeset.serif(20, weight: .medium)).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    if t.provisional { Text("Provisional name until the AI organizes this lesson.").font(Typeset.small).foregroundStyle(Ink.ink2) }
                }
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 12) {
                        StatusChip(status: t.status(now))
                        MasteryBar(value: m, overdue: t.isDue(now) && m < 50)
                    }
                    Text(t.why(now)).font(Typeset.small).foregroundStyle(t.isDue(now) ? Ink.due : Ink.ink2).fixedSize(horizontal: false, vertical: true)
                }
                HStack(spacing: 12) {
                    Button { Task { await studyNow() } } label: { Label("Study Now", systemImage: "play.fill") }.buttonStyle(.primary)
                    Button { quiz = true } label: { Label("Quiz", systemImage: "questionmark.circle") }.buttonStyle(.secondary)
                }
                if !t.summary.isEmpty { Text(t.summary).font(Typeset.serif(13)).foregroundStyle(Ink.ink).textSelection(.enabled).fixedSize(horizontal: false, vertical: true) }
                if !t.terms.isEmpty { section("Key terms") { Chips(items: t.terms) } }
                if !t.prereqs.isEmpty { section("Prerequisites") { Chips(items: t.prereqs) } }
                section("Notes") {
                    TextEditor(text: $notes)
                        .font(Typeset.serif(13))
                        .frame(minHeight: 70)
                        .focused($notesFocused)
                        .scrollContentBackground(.hidden)
                        .padding(.horizontal, 4).padding(.vertical, 6)
                        .background(RoundedRectangle(cornerRadius: 6).fill(Ink.raised))
                        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Ink.line2, lineWidth: 1))
                        .onChange(of: notesFocused) { if !notesFocused { saveNotes() } }
                        .onDisappear(perform: saveNotes)
                }
                VStack(alignment: .leading, spacing: 0) {
                    Divider()
                    fold("Source in the lecture file", $sourceOpen) { sourceSection(d) }
                    Divider()
                    fold("Schedule and overrides", $scheduleOpen) { VStack(alignment: .leading, spacing: 16) { facts; overrides } }
                    Divider()
                    fold("History" + (t.reviews.isEmpty ? "" : " (\(t.reviews.count))"), $historyOpen) { history }
                    Divider()
                }
                Button(role: .destructive) {
                    app.confirmThen("Delete “\(t.name)”?", detail: "The topic and its study history are removed.", ok: "Delete", danger: true) {
                        try await Engine.run("deleteTopics", [t.id])
                        app.selected = nil
                    }
                } label: { Text("Delete Topic…") }
                .buttonStyle(.quiet).font(Typeset.small)
            }
            .font(Typeset.body)
            .padding(20)
            .padding(.top, toolbarHeight)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        // Beside the toolbar AppKit insets this scroll view (or any content margin) by itself, and SwiftUI's hit
        // testing misses that: everything was drawn lower than where it took clicks. So it ignores the toolbar's
        // safe area and keeps clear of the toolbar with padding.
        .ignoresSafeArea(edges: .top)
        .onAppear {
            notes = t.notes
            dueAt = date(t.due ?? nowMs + DAY)
            score = t.override ?? 50
        }
        .task(id: "\(t.id)|\(t.words)") {
            source = nil
            splitAt = nil
            source = try? await Engine.call("topicText", t.id)
        }
        .sheet(isPresented: $quiz) { QuizSheet(t: t) }
    }

    private func section<C: View>(_ title: String, @ViewBuilder _ c: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(Typeset.heading)
            c()
        }
    }

    /// A section that opens and closes with its heading.
    private func fold<C: View>(_ title: String, _ open: Binding<Bool>, @ViewBuilder _ c: () -> C) -> some View {
        let content = c()
        return DisclosureGroup(isExpanded: open) {
            content.padding(.top, 8).padding(.bottom, 12).frame(maxWidth: .infinity, alignment: .leading)
        } label: {
            Text(title).font(Typeset.heading).foregroundStyle(Ink.ink)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
                .onTapGesture { withAnimation(.quick) { open.wrappedValue.toggle() } }
        }
        .padding(.vertical, 8)
    }

    private func saveNotes() {
        guard notes != t.notes else { return }
        let (id, n) = (t.id, notes)
        app.perform { try await Engine.run("updateTopic", id, ["notes": n]) }
    }

    private func studyNow() async {
        if let s = app.session {
            app.screen = .session
            if s.phase == "next" || s.phase == "break" { await app.act { try await Engine.run("sessionAccept", t.id) } }
            return
        }
        await app.startSession(pool: .topic([t.id]), label: t.name, topicId: t.id)
    }

    // MARK: Source

    private func sourceSection(_ d: Doc?) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if let d { Text(d.deckName + (t.start != nil && t.end != nil ? ", paragraphs \(Int(t.start!))–\(Int(t.end!))" : "") + (splitAt == nil ? "" : ". Choose where the new topic starts.")).font(Typeset.small).foregroundStyle(Ink.ink2) }
            if let src = source {
                if let at = splitAt {
                    VStack(alignment: .leading, spacing: 6) {
                        ChoiceList(selection: Binding(get: { at }, set: { splitAt = $0 }), values: Array(src.paras.indices), spacing: 6, isDisabled: { $0 == 0 }) { i in
                            let p = src.paras[i]
                            Text(p.count > 220 ? p.prefix(220) + "…" : p).font(Typeset.serif(12)).foregroundStyle(i >= at ? Ink.ink : Ink.ink2)
                        }
                        TextField("Name of the new topic (optional)", text: $splitName, prompt: Text("Taken from its first sentence if empty")).field()
                        HStack(spacing: 8) {
                            Button("Cancel") { splitAt = nil }.buttonStyle(.secondary)
                            Button("Split Here") {
                                let (id, name) = (t.id, splitName)
                                Task {
                                    if let newId: String = await app.act("Topic split", { try await Engine.call("splitTopic", id, at, name.isEmpty ? nil : name) }) {
                                        splitAt = nil
                                        splitName = ""
                                        app.selected = newId
                                    }
                                }
                            }
                            .buttonStyle(.primary)
                            .disabled(at == 0)
                        }
                        .controlSize(.small)
                    }
                } else {
                    let text = src.excerpt ?? (src.paras.isEmpty ? "No stored text." : src.paras.joined(separator: "\n\n"))
                    Text(text)
                        .font(Typeset.serif(13))
                        .textSelection(.enabled)
                        .lineLimit(showAll ? nil : 12)
                        .padding(12)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 8).fill(Ink.raised))
                        .overlay(alignment: .leading) { Rectangle().fill(app.color(t.courseId)).frame(width: 2) }
                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Ink.line, lineWidth: 0.5))
                    if src.excerpt == nil && !t.provisional && t.manual != true && !src.paras.isEmpty {
                        Text("This is the topic’s paragraphs. The AI’s exact cut is stored for lessons organized from now on; re-organize this lesson to see it here.")
                            .font(Typeset.small).foregroundStyle(Ink.ink2)
                    }
                    HStack(spacing: 8) {
                        Button(showAll ? "Show Less" : "Show All") { showAll.toggle() }.buttonStyle(.quiet)
                        Spacer()
                        if src.paras.count > 1 {
                            Button { splitAt = (src.paras.count + 1) / 2 } label: { Label("Split This Topic", systemImage: "arrow.triangle.branch") }.buttonStyle(.secondary)
                        }
                    }
                    .controlSize(.small)
                }
            } else {
                ProgressView().controlSize(.small)
            }
        }
    }

    // MARK: Facts and overrides

    private var facts: some View {
        Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
            GridRow {
                Text("Suggested time").foregroundStyle(Ink.ink2)
                HStack(spacing: 4) {
                    NumberField(value: t.minutes, range: 1...240, width: 52) { v in let id = t.id; app.perform { try await Engine.run("updateTopic", id, ["minutes": v]) } }
                    Text("min")
                }
            }
            row("Time spent", fmtMin(t.spent))
            row("Reviews", "\(t.reviews.count)")
            row("Interval", t.stab.map { "\(fixed1($0)) days" } ?? "none yet")
            if let diff = t.difficulty, diff > 0 { row("Difficulty", "\(Int(diff)) of 5") }
            row("Length", "\(t.words.formatted()) words")
            if let due = t.due { row("Next due", date(due).formatted(date: .abbreviated, time: .shortened)) }
        }
    }

    private func row(_ k: String, _ v: String) -> some View {
        GridRow { Text(k).foregroundStyle(Ink.ink2); Text(v).monospacedDigit() }
    }

    private var overrides: some View {
        let id = t.id
        let over = { (action: [String: Any], ok: String) in app.perform(ok) { try await Engine.run("override", id, action) } }
        return VStack(alignment: .leading, spacing: 10) {
            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 8) {
                GridRow {
                    Text("Due").foregroundStyle(Ink.ink2)
                    HStack(spacing: 6) {
                        DateField(date: $dueAt)
                        Button("Set") { over(["kind": "due", "at": dueAt.ms], "Due date set") }
                        Button("Now") { over(["kind": "dueNow"], "Due now") }.help("Make it due now")
                    }
                }
                GridRow {
                    Text("Push back").foregroundStyle(Ink.ink2)
                    HStack(spacing: 6) {
                        NumberField(value: push, range: 1...999, width: 44) { push = $0 }
                        MenuChoice(selection: $pushUnit, options: PushUnit.allCases.map { ($0.rawValue, $0) })
                        Button("Push") { over(["kind": "push", "amount": push, "unit": pushUnit.rawValue], "Pushed \(Int(push)) \(push == 1 ? String(pushUnit.rawValue.dropLast()) : pushUnit.rawValue)") }
                    }
                }
                GridRow {
                    Text("Score").foregroundStyle(Ink.ink2)
                    HStack(spacing: 6) {
                        NumberField(value: score, range: 0...100, width: 44) { score = $0 }
                        Button("Set") { over(["kind": "score", "value": score], "Score set") }
                        if t.override != nil { Button("Clear") { over(["kind": "clearScore"], "Score override cleared") } }
                    }
                }
            }
            HStack(spacing: 8) {
                Button("Mark Solid for 30 Days") { over(["kind": "solid"], "Marked solid for 30 days") }
                Button("Reset…", role: .destructive) {
                    app.confirmThen("Reset this topic?", detail: "Clears its schedule, reviews, time spent and score override. It becomes new again.", ok: "Reset", danger: true, done: "Topic reset") {
                        try await Engine.run("override", id, ["kind": "reset"])
                    }
                }
            }
        }
        .buttonStyle(.secondary)
        .controlSize(.small)
    }

    @ViewBuilder private var history: some View {
        if t.reviews.isEmpty {
            Text("No reviews yet.").font(Typeset.small).foregroundStyle(Ink.ink2)
        } else {
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 4) {
                ForEach(Array(t.reviews.reversed().prefix(30).enumerated()), id: \.offset) { _, r in
                    GridRow(alignment: .firstTextBaseline) {
                        Text(date(r.at).formatted(.dateTime.month(.abbreviated).day().year())).foregroundStyle(Ink.ink2)
                        (Text(r.selfRating.map { $0 != r.rating ? "\($0.rawValue), with quiz \(r.rating.rawValue)" : r.rating.rawValue } ?? r.rating.rawValue)
                         + Text(r.note.map { ": \($0)" } ?? "").foregroundStyle(Ink.ink2))
                        Text(fmtMin(r.minutes) + (r.quiz.map { ", quiz \(Int(($0 * 100).rounded()))%" } ?? "")).monospacedDigit().foregroundStyle(Ink.ink2)
                    }
                }
            }
            .font(Typeset.small)
        }
    }
}

/// A quiz from the inspector: the score, then a self-rating, is recorded as a review.
private struct QuizSheet: View {
    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    let t: Topic
    @State private var score: Double?

    var body: some View {
        SheetFrame(title: "Quiz: \(t.name)", width: 620) {
            if let score {
                Text("Quiz score \(Int((score * 100).rounded()))%. How well do you know it? The quiz counts double.")
                HStack(spacing: 8) {
                    ForEach(Rating.allCases) { r in
                        Button(r.label) {
                            let id = t.id
                            Task {
                                if await app.act("Review recorded", { try await Engine.run("reviewTopic", id, r.rawValue, score, 0) }) != nil { dismiss() }
                            }
                        }
                        .buttonStyle(.secondary)
                    }
                }
            } else {
                QuizPanel(topicId: t.id, scoreLabel: "Record as a Review") { score = $0 }
            }
        } actions: {
            Button("Close") { dismiss() }.keyboardShortcut(.cancelAction)
        }
    }
}

// MARK: - New course and add topic

struct NewCourseSheet: View {
    @Environment(AppModel.self) private var app
    @State private var step = 1
    @State private var name = ""
    @State private var tint: Tint?
    @State private var custom = Color.teal
    @State private var busy = false
    @State private var error: String?

    private var clean: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var taken: Bool { !clean.isEmpty && (app.data?.courses ?? []).contains { $0.name.lowercased() == clean.lowercased() } }
    private var staged: [String] { app.courseWizard ?? [] }
    private var preview: Color { tint?.color ?? Hue.color(Hue.palette[(app.data?.courses.count ?? 0) % 8]) }

    var body: some View {
        SheetFrame(title: "New course", width: 560) {
            Text(step == 1 ? "Step 1 of 2: name and colour" : "Step 2 of 2: lecture files").font(Typeset.small).foregroundStyle(Ink.ink2)
            HStack(alignment: .top, spacing: 20) {
                card
                VStack(alignment: .leading, spacing: 12) {
                    if step == 1 { nameStep } else { filesStep }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        } actions: {
            if step == 1 {
                Button("Cancel") { app.courseWizard = nil }.keyboardShortcut(.cancelAction)
                Button("Next") { step = 2 }.keyboardShortcut(.defaultAction).buttonStyle(.primary).disabled(clean.isEmpty || taken)
            } else {
                Button("Back") { step = 1 }
                Spacer()
                Button("Cancel") { app.courseWizard = nil }.keyboardShortcut(.cancelAction)
                Button(staged.isEmpty ? "Create Empty Course" : "Create and Add \(plural(staged.count, "Item"))") { Task { await create() } }
                    .keyboardShortcut(.defaultAction).buttonStyle(.primary)
                    .disabled(busy)
            }
        }
    }

    private var card: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(staged.prefix(3), id: \.self) { Text(($0 as NSString).lastPathComponent).font(Typeset.small).foregroundStyle(Ink.ink3).lineLimit(1) }
            Spacer(minLength: 12)
            Text(clean.isEmpty ? "Course name" : clean).font(.system(size: 15, weight: .semibold)).foregroundStyle(clean.isEmpty ? Ink.ink3 : Ink.ink)
            Text(staged.isEmpty ? "No lessons yet" : "\(plural(staged.count, "item")) to add").font(Typeset.small).foregroundStyle(Ink.ink2)
        }
        .padding(14)
        .frame(width: 180, height: 140, alignment: .topLeading)
        .background(RoundedRectangle(cornerRadius: 8).fill(Ink.raised))
        .overlay(alignment: .top) { UnevenRoundedRectangle(topLeadingRadius: 8, topTrailingRadius: 8).fill(preview).frame(height: 4) }
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Ink.line, lineWidth: 0.5))
        .accessibilityHidden(true)
    }

    @ViewBuilder private var nameStep: some View {
        TextField("Course name", text: $name, prompt: Text("e.g. Organic Chemistry or CHM110"))
            .font(Typeset.serif(15))
            .field()
            .onSubmit { if !clean.isEmpty && !taken { step = 2 } }
        if taken { Text("There is already a course called “\(clean)”.").font(Typeset.small).foregroundStyle(Ink.due) }
        Text("Colour").font(Typeset.heading)
        FlowLayout(spacing: 6) {
            swatch(nil, Ink.line2, help: "Default colour")
            ForEach(Hue.swatches, id: \.self) { h in swatch(.hue(h), Hue.color(h), help: Hue.name(h)) }
            swatch(.neutral, Ink.ink3, help: "Grey")
            ColorPicker("Custom colour", selection: Binding(get: { custom }, set: { custom = $0; tint = .custom(hex($0)) }), supportsOpacity: false).labelsHidden()
        }
    }

    private func swatch(_ t: Tint?, _ c: Color, help: String) -> some View {
        Button { tint = t } label: {
            Circle().fill(c).frame(width: 18, height: 18)
                .overlay(Circle().stroke(Ink.ink, lineWidth: tint == t ? 2 : 0).padding(-3))
                .overlay { if t == nil { Image(systemName: "xmark").font(.system(size: 8, weight: .bold)).foregroundStyle(Ink.ink2) } }
        }
        .buttonStyle(.plain)
        .help(help)
        .accessibilityLabel(help)
    }

    private func hex(_ c: Color) -> String {
        let n = NSColor(c).usingColorSpace(.sRGB) ?? .gray
        return String(format: "#%02x%02x%02x", Int(n.redComponent * 255), Int(n.greenComponent * 255), Int(n.blueComponent * 255))
    }

    @ViewBuilder private var filesStep: some View {
        (Text("Add the lecture files for ") + Text(clean).bold() + Text(". Each file becomes a lesson, which the AI then splits into topics. You can skip this and add files later."))
            .fixedSize(horizontal: false, vertical: true)
        HStack(spacing: 12) {
            Button { pick("files") } label: { Label("Upload Files…", systemImage: "doc") }
            Button { pick("folder") } label: { Label("Upload a Folder…", systemImage: "folder") }
        }
        .buttonStyle(.secondary)
        Text("Or drop files and folders anywhere in the window. .md, .txt, .pdf or .docx.").font(Typeset.small).foregroundStyle(Ink.ink2)
        if !staged.isEmpty {
            List {
                ForEach(staged, id: \.self) { p in
                    HStack {
                        Image(systemName: ["md", "markdown", "txt", "pdf", "docx"].contains((p as NSString).pathExtension.lowercased()) ? "doc" : "folder").foregroundStyle(Ink.ink2)
                        Text((p as NSString).lastPathComponent).lineLimit(1)
                        Text((p as NSString).deletingLastPathComponent).font(Typeset.small).foregroundStyle(Ink.ink3).lineLimit(1).truncationMode(.head)
                        Spacer()
                        Button { app.courseWizard?.removeAll { $0 == p } } label: { Image(systemName: "xmark") }.buttonStyle(.quiet).help("Remove")
                    }
                }
            }
            .listStyle(.bordered)
            .frame(height: 140)
        }
        if let error { Text(error).font(Typeset.small).foregroundStyle(Ink.due) }
    }

    private func pick(_ kind: String) {
        Task {
            guard let paths: [String] = await app.act({ try await Engine.call("pickPaths", kind) }), !paths.isEmpty else { return }
            app.courseWizard = Array(NSOrderedSet(array: staged + paths)) as? [String]
        }
    }

    private func create() async {
        busy = true
        error = nil
        do {
            let id: String = try await Engine.call("createCourse", clean)
            if let tint { app.library.colors[id] = tint }
            let files = staged
            app.courseWizard = nil
            app.scope = .course(id)
            app.screen = .library
            if files.isEmpty { app.say("Created \(clean)") } else { await app.importer.paths(files, course: id, app: app) }
        } catch {
            self.error = error.localizedDescription
            busy = false
        }
    }
}

struct AddTopicSheet: View {
    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    @State private var courseId = ""
    @State private var courseName = ""
    @State private var docId = ""
    @State private var lessonTitle = "Manual topics"
    @State private var unit = ""
    @State private var name = ""
    @State private var text = ""
    @State private var enrich = true

    private let new = "__new"

    var body: some View {
        SheetFrame(title: "Add a topic", width: 560) {
            Form {
                Picker("Course", selection: $courseId) {
                    ForEach(app.data?.courses ?? []) { Text($0.name).tag($0.id) }
                    Divider()
                    Text("New Course…").tag(new)
                }
                .onChange(of: courseId) { if app.docs[docId]?.courseId != courseId { docId = app.lessons(of: courseId).first?.id ?? new } }
                if courseId == new {
                    TextField("New course name", text: $courseName)
                } else {
                    Picker("Lesson", selection: $docId) {
                        ForEach(app.lessons(of: courseId)) { Text($0.title).tag($0.id) }
                        Divider()
                        Text("New Lesson…").tag(new)
                    }
                }
                if docId == new { TextField("Lesson title", text: $lessonTitle) }
                TextField("Unit (optional)", text: $unit)
                TextField("Topic name", text: $name, prompt: Text("e.g. Michaelis-Menten kinetics"))
                LabeledContent("Text to study") {
                    TextEditor(text: $text).font(Typeset.body).frame(height: 150)
                        .scrollContentBackground(.hidden)
                        .padding(.horizontal, 2).padding(.vertical, 4)
                        .background(RoundedRectangle(cornerRadius: 5).fill(Ink.raised))
                        .overlay(RoundedRectangle(cornerRadius: 5).strokeBorder(Ink.line2, lineWidth: 0.5))
                        // An editor has no text baseline, so the form would pin the label to its top edge; put the
                        // label's baseline on the editor's first line instead.
                        .alignmentGuide(.firstTextBaseline) { $0[.top] + 4 + NSFont.systemFont(ofSize: 13).ascender }
                }
                Toggle("Let the AI add a summary, key terms, prerequisites and difficulty (one small request)", isOn: $enrich).toggleStyle(.check)
            }
        } actions: {
            Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
            Button("Add Topic") { Task { await save() } }
                .keyboardShortcut(.defaultAction).buttonStyle(.primary)
                .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || text.trimmingCharacters(in: .whitespaces).isEmpty || (courseId == new && courseName.trimmingCharacters(in: .whitespaces).isEmpty))
        }
        .onAppear(perform: preset)
    }

    /// Starts in the lesson, unit or course the Library is showing.
    private func preset() {
        var preDoc = ""
        switch app.scope {
        case .lesson(let d): preDoc = d
        case .unit(let d, let u): preDoc = d; unit = u
        default: break
        }
        courseId = app.openCourse ?? app.data?.courses.first?.id ?? new
        docId = preDoc.isEmpty ? app.lessons(of: courseId).first?.id ?? new : preDoc
    }

    private func save() async {
        let input: [String: Any?] = [
            "docId": docId == new ? nil : docId, "courseId": courseId == new ? nil : courseId, "courseName": courseId == new ? courseName : nil,
            "lessonTitle": docId == new ? lessonTitle : nil, "unit": unit, "name": name, "text": text, "enrich": enrich,
        ]
        if let id: String = await app.act("Topic added", { try await Engine.call("addTopic", input) }) {
            app.selected = id
            dismiss()
        }
    }
}
