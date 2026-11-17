// Importing lessons, shared by dropped files, the Upload buttons and File > Import: confirm replacing changed lessons
// that carry study history, then show the AI estimate before organizing starts. Files dropped outside an open
// course go to the New course sheet, so a course is never named after its files.
import SwiftUI

@MainActor @Observable
final class ImportFlow {
    var busy = false
    /// Lessons that changed and carry history: replace them or keep the old versions.
    var changed: ImportResult?
    /// Problems or notes from the last import, shown with the estimate (or alone).
    var report: ImportResult?
    var estimate: (est: Estimate, docIds: [String]?)?

    func paths(_ paths: [String], course: String, app: AppModel) async {
        busy = true
        defer { busy = false }
        do {
            let res: ImportResult = try await Engine.call("importPaths", paths, false, course)
            await finish(res, app: app)
        } catch { app.show(error) }
    }

    func pick(_ kind: String, course: String, app: AppModel) async {
        guard let paths: [String] = await app.act({ try await Engine.call("pickPaths", kind) }), !paths.isEmpty else { return }
        await self.paths(paths, course: course, app: app)
    }

    /// Files dropped on the window or picked from the File menu: into the New course sheet when it is open, else
    /// the course open in the Library, else a new course.
    func drop(_ paths: [String], app: AppModel) {
        if let staged = app.courseWizard { app.courseWizard = Array(NSOrderedSet(array: staged + paths)) as? [String] }
        else if app.screen == .library, let course = app.openCourse { Task { await self.paths(paths, course: course, app: app) } }
        else { app.screen = .library; app.courseWizard = Array(NSOrderedSet(array: paths)) as? [String] }
    }

    func estimate(_ docIds: [String]?, app: AppModel) async {
        if let est: Estimate = await app.act({ try await Engine.call("estimate", docIds) }) { estimate = (est, docIds) }
    }

    private func finish(_ res: ImportResult, app: AppModel) async {
        if !res.needsConfirm.isEmpty { changed = res; return }
        changed = nil
        report = res.errors.isEmpty && res.notes.isEmpty ? nil : res
        if !res.docIds.isEmpty { await estimate(res.docIds, app: app) }
        else if res.errors.isEmpty { app.say(res.unchanged > 0 ? "Nothing new: \(res.unchanged) file(s) unchanged since the last import." : "No lessons found.") }
    }

    func replace(app: AppModel) async {
        guard let r = changed else { return }
        busy = true
        defer { busy = false }
        do {
            let r2: ImportResult = try await Engine.call("importPaths", r.needsConfirm.map(\.path), true)
            await finish(ImportResult(added: r.added + r2.added, unchanged: r.unchanged + r2.unchanged, replaced: r.replaced + r2.replaced, needsConfirm: r2.needsConfirm,
                                      errors: r.errors + r2.errors, notes: Array(NSOrderedSet(array: r.notes + r2.notes)) as? [String] ?? [], docIds: r.docIds + r2.docIds), app: app)
        } catch { app.show(error) }
    }

    func keepOld(app: AppModel) async {
        guard var r = changed else { return }
        r.needsConfirm = []
        await finish(r, app: app)
    }

    func startOrganizing(app: AppModel) async {
        let ids = estimate?.docIds
        guard await app.act({ try await Engine.run("startQueue", ids) }) != nil else { return }
        estimate = nil
        report = nil
        app.say("Organizing started. Follow progress in the AI Queue.")
    }
}

/// The import sheets, attached to the main window.
struct ImportSheets: ViewModifier {
    @Environment(AppModel.self) private var app

    func body(content: Content) -> some View {
        @Bindable var flow = app.importer
        content
            .sheet(item: Binding(get: { flow.changed.map(SheetBox.init) }, set: { if $0 == nil { flow.changed = nil } })) { box in
                ChangedLessons(result: box.value)
            }
            .sheet(isPresented: Binding(get: { flow.estimate != nil }, set: { if !$0 { flow.estimate = nil; flow.report = nil } })) {
                OrganizeSheet()
            }
            .sheet(isPresented: Binding(get: { flow.estimate == nil && flow.changed == nil && !(flow.report?.errors.isEmpty ?? true) }, set: { if !$0 { flow.report = nil } })) {
                ImportProblems()
            }
    }
}

/// Identifiable wrapper for a value shown in a sheet.
struct SheetBox<T>: Identifiable {
    let id = UUID()
    let value: T
}

private struct ChangedLessons: View {
    @Environment(AppModel.self) private var app
    let result: ImportResult
    var body: some View {
        SheetFrame(title: "Some lessons changed", width: 480) {
            Text("These files changed since you imported them, and their topics have study history. Replacing them rebuilds the topics. History moves to new topics whose name or text still matches (at least 60% overlap); unmatched topics start fresh.")
                .fixedSize(horizontal: false, vertical: true)
            Table(result.needsConfirm) {
                TableColumn("Lesson", value: \.title)
                TableColumn("Topics with history") { Text("\($0.withHistory)").monospacedDigit() }.width(130)
            }
            .frame(minHeight: 120)
        } actions: {
            Button("Keep the Old Versions") { Task { await app.importer.keepOld(app: app) } }
            Button("Replace and Carry History Over") { Task { await app.importer.replace(app: app) } }.keyboardShortcut(.defaultAction).buttonStyle(.primary)
        }
    }
}

extension ImportChange: Identifiable { var id: String { docId } }

private struct OrganizeSheet: View {
    @Environment(AppModel.self) private var app
    var body: some View {
        let flow = app.importer
        if let e = flow.estimate {
            SheetFrame(title: "Organize with AI", width: 540) {
                if let r = flow.report { Notes(result: r) }
                Text("\(plural(e.est.docs, "lesson")) ready. The AI names topics, groups units, writes summaries and suggests study times.")
                let docs = (app.data?.docs ?? []).filter { e.docIds?.contains($0.id) == true }
                if let files = app.fileSupport, !docs.isEmpty {
                    ScrollView { Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                        GridRow { Text("Lesson").font(Typeset.heading); Text("How \(files.provider) gets it").font(Typeset.heading) }
                        ForEach(docs) { d in
                            GridRow(alignment: .top) {
                                Text(d.fileName.isEmpty ? d.title : d.fileName).lineLimit(2)
                                if let dl = Delivery.of(d, files) { DeliveryTag(delivery: dl, detail: true) }
                            }
                        }
                    } }
                    .frame(maxHeight: 260)
                }
                HStack(spacing: 32) {
                    Stat(value: "~\(e.est.requests)", label: "requests")
                    Stat(value: "~\(Int(e.est.tokens).formatted())", label: "tokens")
                    if e.est.cached > 0 { Stat(value: "\(e.est.cached)", label: "windows already cached") }
                }
                Text("Until then, lessons show provisional topic names taken from headings or first sentences, and you can already study them. You can start this later from the AI Queue.")
                    .font(Typeset.small).foregroundStyle(Ink.ink2).fixedSize(horizontal: false, vertical: true)
            } actions: {
                Button("Open the AI Queue") { flow.estimate = nil; flow.report = nil; app.screen = .queue }
                Spacer()
                Button("Later") { flow.estimate = nil; flow.report = nil }.keyboardShortcut(.cancelAction)
                Button(e.est.requests == 0 && e.est.cached > 0 ? "Apply Cached Results" : "Start Organizing") { Task { await flow.startOrganizing(app: app) } }
                    .keyboardShortcut(.defaultAction).buttonStyle(.primary)
                    .disabled(e.est.docs == 0)
            }
        }
    }
}

private struct ImportProblems: View {
    @Environment(AppModel.self) private var app
    var body: some View {
        SheetFrame(title: "Import finished with problems", width: 460) {
            if let r = app.importer.report { Notes(result: r) }
        } actions: {
            Button("Close") { app.importer.report = nil }.keyboardShortcut(.defaultAction).buttonStyle(.primary)
        }
    }
}

private struct Notes: View {
    let result: ImportResult
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(result.notes, id: \.self) { Text($0).font(Typeset.small).foregroundStyle(Ink.ink2) }
            ForEach(result.errors, id: \.path) { e in
                Text("\((e.path as NSString).lastPathComponent): ").foregroundStyle(Ink.due) + Text(e.error)
            }
            .font(Typeset.small)
        }
        .textSelection(.enabled)
    }
}

/// A sheet with a title, content and a row of buttons at the bottom, as macOS dialogs are laid out.
struct SheetFrame<Content: View, Actions: View>: View {
    let title: String
    var width: CGFloat = 480
    @ViewBuilder var content: Content
    @ViewBuilder var actions: Actions
    var body: some View {
        // As tall as its content: long lists inside give themselves a height and scroll.
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 14) {
                Text(title).font(.system(size: 15, weight: .semibold))
                content
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
            Divider()
            HStack { Spacer(); actions }.padding(16)
        }
        .frame(width: width)
        .fixedSize(horizontal: false, vertical: true)
    }
}

// MARK: - How a lesson file reaches the AI

/// How a lesson goes to the model now: its file type's row, adjusted for this file's size, pages and history.
struct Delivery: Hashable {
    let kind: String // file | text | extracted | unsupported
    let label: String
    let detail: String

    static func of(_ d: Doc, _ table: FileSupportTable) -> Delivery? {
        guard d.ext != "manual" else { return nil }
        let ext = d.ext == "jpeg" ? "jpg" : d.ext == "markdown" ? "md" : d.ext
        guard let row = table.types.first(where: { $0.ext == ext }) else { return nil }
        let mb = { (b: Double) in Int((b / 1_048_576).rounded()) }
        if row.kind != "file" {
            if d.needsReading == true { return .init(kind: "unsupported", label: "Can’t be read", detail: "\(row.detail) The Mac found no text in this file.") }
            return .init(kind: row.kind, label: row.label, detail: row.detail)
        }
        if d.readByAi == true && d.needsReading != true { return .init(kind: "text", label: "Read by AI", detail: "The AI wrote out this lesson’s text from the file; organizing uses that text.") }
        if d.fileHash == nil { return .init(kind: "extracted", label: "Text extracted", detail: "Imported before files could be sent. Import the file again to send the file itself.") }
        if let max = row.maxBytes, (d.fileBytes ?? 0) > max { return .init(kind: "extracted", label: "Text extracted", detail: "This file is \(mb(d.fileBytes ?? 0)) MB; files over \(mb(max)) MB go as extracted text.") }
        if let max = row.maxPages, (d.pages ?? 0) > max { return .init(kind: "extracted", label: "Text extracted", detail: "This PDF has \(d.pages ?? 0) pages; over \(max), \(table.provider) gets the extracted text.") }
        if d.needsReading == true && d.ext == "pdf" && (d.pages ?? 0) > 40 { return .init(kind: "unsupported", label: "Too long to read", detail: "This scan has \(d.pages ?? 0) pages; the AI writes out at most 40 at a time. Split it into shorter PDFs.") }
        if d.needsReading == true { return .init(kind: row.kind, label: "AI reads it", detail: "The Mac found no text here (a scan or a picture), so \(table.provider) reads the file and writes out its text first.") }
        return .init(kind: row.kind, label: row.label, detail: row.detail)
    }

    var symbol: String {
        switch kind {
        case "file": "doc"
        case "unsupported": "xmark"
        default: "doc.text"
        }
    }
}

/// A quiet inline status: glyph and words, with the reason on hover and for VoiceOver.
struct DeliveryTag: View {
    let delivery: Delivery
    var detail = false
    var compact = false
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Group { if compact { Image(systemName: delivery.symbol) } else { Label(delivery.label, systemImage: delivery.symbol) } }
                .foregroundStyle(delivery.kind == "unsupported" ? Ink.due : Ink.ink2)
            if detail { Text(delivery.detail).foregroundStyle(Ink.ink2).fixedSize(horizontal: false, vertical: true) }
        }
        .font(Typeset.small)
        .help(delivery.detail)
        .accessibilityLabel("\(delivery.label). \(delivery.detail)")
    }
}

enum FileTypes {
    static func name(_ ext: String) -> String {
        ["pdf": "PDF", "docx": "Word", "md": "Markdown", "txt": "Text", "png": "Pictures", "jpg": "JPEG", "webp": "WebP"][ext] ?? ext.uppercased()
    }

    /// One sentence for the drop target: what the current model reads as files.
    static func dropSummary(_ table: FileSupportTable?) -> String {
        guard let table else { return "PDF, Word, Markdown, text, and pictures for Gemini or the scheduled task" }
        let pics: Set = ["png", "jpg", "webp"]
        let kinds = { (k: String) in Array(NSOrderedSet(array: table.types.filter { $0.kind == k }.map { pics.contains($0.ext) ? "pictures" : name($0.ext) })) as? [String] ?? [] }
        let files = kinds("file"), extracted = kinds("extracted")
        if files.isEmpty { return "\(table.provider) gets text only: Markdown and text as written, \(listNames(extracted)) as extracted text" }
        return "\(table.provider) reads \(listNames(files)) as they are" + (extracted.isEmpty ? "" : "; \(listNames(extracted)) go as extracted text")
    }
}

