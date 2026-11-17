// Adding and editing a deadline, and importing deadlines from a spreadsheet: read by the AI with a review step, or
// mapped column by column with a preview that checks every row.
import SwiftUI

struct DeadlineForm: View {
    enum Mode: Identifiable {
        case new(String?), edit(Deadline)
        var id: String { if case .edit(let d) = self { d.id } else { "new" } }
    }

    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    let mode: Mode
    @State private var title = ""
    @State private var day = Date()
    @State private var link = "none"
    @State private var courseId = ""
    @State private var docId = ""
    @State private var topicIds: [String] = []
    @State private var filter = ""
    @State private var notes = ""

    private var initial: Deadline? { if case .edit(let d) = mode { d } else { nil } }

    var body: some View {
        SheetFrame(title: initial == nil ? "Add a deadline" : "Edit deadline", width: 520) {
            Form {
                TextField("Title", text: $title, prompt: Text("e.g. Biochemistry midterm"))
                DatePicker("Date", selection: $day, displayedComponents: .date)
                Picker("Gives priority to", selection: $link) {
                    Text("Nothing (reminder only)").tag("none")
                    Text("A course").tag("course")
                    Text("A lesson").tag("lesson")
                    Text("Chosen topics").tag("topics")
                }
                if link == "course" {
                    Picker("Course", selection: $courseId) { ForEach(app.data?.courses ?? []) { Text($0.name).tag($0.id) } }
                }
                if link == "lesson" {
                    Picker("Lesson", selection: $docId) { ForEach(app.data?.docs ?? []) { Text($0.title).tag($0.id) } }
                }
                TextField("Notes", text: $notes, axis: .vertical).lineLimit(2...4)
            }
            if link == "topics" { TopicPicker(ids: $topicIds, multiple: true, filter: $filter) }
        } actions: {
            Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
            Button(initial == nil ? "Add Deadline" : "Save Deadline") { Task { await save() } }
                .keyboardShortcut(.defaultAction).buttonStyle(.primary)
                .disabled(title.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .onAppear(perform: load)
    }

    private func load() {
        courseId = app.data?.courses.first?.id ?? ""
        docId = app.data?.docs.first?.id ?? ""
        switch mode {
        case .new(let k): day = Days.date(k ?? Days.add(Days.key(), 14))
        case .edit(let d):
            title = d.title
            day = Days.date(d.date)
            notes = d.notes ?? ""
            topicIds = d.topicIds ?? []
            if let c = d.courseId { courseId = c }
            if let doc = d.docId { docId = doc }
            link = !(d.topicIds ?? []).isEmpty ? "topics" : d.docId != nil ? "lesson" : d.courseId != nil ? "course" : "none"
        }
    }

    private func save() async {
        let payload: [String: Any?] = [
            "title": title, "date": Days.key(day), "notes": notes,
            "courseId": link == "course" ? courseId : nil, "docId": link == "lesson" ? docId : nil, "topicIds": link == "topics" ? topicIds : [],
        ]
        let ok: Void? = await app.act(initial == nil ? "Deadline added" : "Deadline saved") {
            if let d = initial { try await Engine.run("updateDeadline", d.id, payload) } else { try await Engine.run("addDeadline", payload) }
        }
        if ok != nil { dismiss() }
    }
}

// MARK: - AI import

/// Spreadsheet → text → AI. Starts as soon as it opens; nothing is added until the student reviews what was found.
struct AiSheetImport: View {
    @Environment(AppModel.self) private var app
    @Environment(\.openSettings) private var openSettings
    let sheet: SheetData
    let onManual: () -> Void
    let onClose: () -> Void
    @State private var res: DeadlineExtract?
    @State private var error: String?
    @State private var skip: Set<Int> = []

    private var existing: Set<String> { Set((app.data?.deadlines ?? []).map(\.key)) }
    private func dup(_ it: ExtractedDeadline) -> Bool { existing.contains(DeadlineMap.key(it.title, it.date, it.courseName)) }
    private var chosen: [ExtractedDeadline] { (res?.items ?? []).enumerated().filter { !skip.contains($0.offset) }.map(\.element) }

    var body: some View {
        SheetFrame(title: "Import deadlines", width: 760) {
            Text(sheet.path).font(Typeset.small).foregroundStyle(Ink.ink2)
            if res == nil && error == nil {
                HStack(spacing: 8) { ProgressView().controlSize(.small); Text("Converting the spreadsheet and asking the AI to find the deadlines…") }
            }
            if let error {
                Text(error).foregroundStyle(Ink.due).textSelection(.enabled)
                if error.range(of: "API key", options: .caseInsensitive) != nil { Button("Open Settings") { onClose(); openSettings() }.buttonStyle(.primary) }
                else { Text("You can still import this file by mapping its columns manually.").font(Typeset.small).foregroundStyle(Ink.ink2) }
            }
            if let res {
                Text((res.converter == "markitdown" ? "Converted with markitdown" : "Converted with the built-in converter") + (res.chunks > 1 ? " and read in \(res.chunks) parts" : "") + ". Check the dates before importing; untick anything you don’t want. Deadlines you already have are unticked.")
                    .font(Typeset.small).foregroundStyle(Ink.ink2)
                if let note = res.converterNote { Text(note).font(Typeset.small) }
                if res.items.isEmpty {
                    Text("The AI found no dated items in this spreadsheet. Try mapping its columns manually.").foregroundStyle(Ink.ink2)
                } else {
                    Table(res.items.indexed()) {
                        TableColumn("") { r in Toggle("", isOn: Binding(get: { !skip.contains(r.id) }, set: { if $0 { skip.remove(r.id) } else { skip.insert(r.id) } })).labelsHidden() }.width(24)
                        TableColumn("Title") { r in Text(r.value.title) + (dup(r.value) ? Text(" · already added").foregroundStyle(Ink.ink2) : Text("")) }
                        TableColumn("Date") { r in Text(Days.date(r.value.date).formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day().year())).monospacedDigit() }.width(130)
                        TableColumn("Course") { r in Text(r.value.courseName ?? "") }.width(120)
                        TableColumn("Notes") { r in Text(r.value.notes).font(Typeset.small) }
                    }
                    .frame(height: 320)
                }
            }
        } actions: {
            if let res { Text("\(res.items.count) found" + (res.dropped > 0 ? ", \(res.dropped) skipped without a usable date" : "")).font(Typeset.small).foregroundStyle(Ink.ink2) }
            Spacer()
            Button("Map Columns Manually") { leave(onManual) }
            Button("Cancel") { leave(onClose) }.keyboardShortcut(.cancelAction)
            Button("Import \(plural(chosen.count, "Deadline"))") { Task { await doImport() } }.keyboardShortcut(.defaultAction).buttonStyle(.primary).disabled(chosen.isEmpty)
        }
        .task {
            do {
                let r: DeadlineExtract = try await Engine.call("extractDeadlines", sheet.path)
                res = r
                skip = Set(r.items.indices.filter { dup(r.items[$0]) })
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    /// Leaving while the AI is still reading the sheet cancels that request.
    private func leave(_ then: () -> Void) {
        if res == nil && error == nil { Task { try? await Engine.run("cancelAi", ["kind": "deadlines"]) } }
        then()
    }

    private func doImport() async {
        let items = chosen.map { ["title": $0.title, "date": $0.date, "courseName": $0.courseName, "notes": $0.notes] as [String: Any?] }
        let r: Void? = await app.act { try await Engine.run("importDeadlines", items) }
        if r != nil { onClose() }
    }
}

// MARK: - Manual import

struct SheetImport: View {
    @Environment(AppModel.self) private var app
    let sheet: SheetData
    let onClose: () -> Void
    @State private var si = 0
    @State private var map = DeadlineMap.Mapping(title: 0, date: 1, course: nil, notes: nil, headerRow: true, dateFormat: .auto)

    private var rows: [[JSONValue]] { si < sheet.sheets.count ? sheet.sheets[si].rows : [] }

    var body: some View {
        let width = max(1, rows.prefix(50).map(\.count).max() ?? 1)
        let result = DeadlineMap.map(rows, map, existing: Set((app.data?.deadlines ?? []).map(\.key)))
        let valid = Dictionary(result.valid.map { ($0.row, $0) }, uniquingKeysWith: { a, _ in a })
        let errors = Dictionary(result.errors.map { ($0.row, $0.error) }, uniquingKeysWith: { a, _ in a })
        let first = map.headerRow ? 1 : 0
        SheetFrame(title: "Import deadlines", width: 760) {
            Text(sheet.path).font(Typeset.small).foregroundStyle(Ink.ink2)
            Form {
                if sheet.sheets.count > 1 {
                    Picker("Sheet", selection: $si) { ForEach(sheet.sheets.indices, id: \.self) { Text(sheet.sheets[$0].name).tag($0) } }
                        .onChange(of: si) { map = DeadlineMap.guess(rows.first ?? []) }
                }
                column("Title column", \.title, width)
                column("Date column", \.date, width)
                optionalColumn("Course column", \.course, width)
                optionalColumn("Notes column", \.notes, width)
                Picker("Date format for numbers like 03/04/2026", selection: $map.dateFormat) {
                    Text("Detect (day/month when unclear)").tag(DeadlineMap.DateFormat.auto)
                    Text("Day/month/year").tag(DeadlineMap.DateFormat.dmy)
                    Text("Month/day/year").tag(DeadlineMap.DateFormat.mdy)
                    Text("Year-month-day only").tag(DeadlineMap.DateFormat.ymd)
                }
                Toggle("First row is a header", isOn: $map.headerRow)
            }
            Text("Rows already imported (same title, date and course) are skipped, so re-importing the same sheet is safe. Course names that match an existing course are linked to it.")
                .font(Typeset.small).foregroundStyle(Ink.ink2)
            Table(Array(rows.indexed().dropFirst(first).prefix(200))) {
                TableColumn("Row") { r in Text("\(r.id + 1)").monospacedDigit().foregroundStyle(Ink.ink2) }.width(36)
                TableColumn("Title") { r in Text(cell(r.value, map.title)) }
                TableColumn("Date") { r in Text(valid[r.id + 1]?.date ?? cell(r.value, map.date)).monospacedDigit() }.width(100)
                TableColumn("Course") { r in Text(map.course.map { cell(r.value, $0) } ?? "") }.width(120)
                TableColumn("Check") { r in
                    Text(errors[r.id + 1] ?? valid[r.id + 1].map { $0.warning ?? "OK" } ?? "Duplicate or empty")
                        .font(Typeset.small).foregroundStyle(errors[r.id + 1] != nil ? Ink.due : Ink.ink2)
                }
            }
            .frame(height: 300)
        } actions: {
            Text("\(result.valid.count) ready, \(result.errors.count) with problems, \(result.duplicates) duplicates skipped").font(Typeset.small).foregroundStyle(Ink.ink2)
            Spacer()
            Button("Cancel", action: onClose).keyboardShortcut(.cancelAction)
            Button("Import \(plural(result.valid.count, "Deadline"))") {
                let items = result.valid.map { ["title": $0.title, "date": $0.date, "courseName": $0.course, "notes": $0.notes] as [String: Any?] }
                Task { if await app.act({ try await Engine.run("importDeadlines", items) }) != nil { onClose() } }
            }
            .keyboardShortcut(.defaultAction).buttonStyle(.primary)
            .disabled(result.valid.isEmpty)
        }
        .onAppear { map = DeadlineMap.guess(rows.first ?? []) }
    }

    private func cell(_ r: [JSONValue], _ i: Int) -> String { i < r.count ? r[i].text : "" }

    private func label(_ i: Int) -> String {
        let h = map.headerRow ? (rows.first ?? []) : []
        let name = i < h.count ? String(h[i].text.trimmingCharacters(in: .whitespaces).prefix(30)) : ""
        return DeadlineMap.columnName(i) + (name.isEmpty ? "" : ": \(name)")
    }

    private func column(_ title: String, _ key: WritableKeyPath<DeadlineMap.Mapping, Int>, _ width: Int) -> some View {
        Picker(title, selection: Binding(get: { map[keyPath: key] }, set: { map[keyPath: key] = $0 })) {
            ForEach(0..<width, id: \.self) { Text(label($0)).tag($0) }
        }
    }

    private func optionalColumn(_ title: String, _ key: WritableKeyPath<DeadlineMap.Mapping, Int?>, _ width: Int) -> some View {
        Picker(title, selection: Binding(get: { map[keyPath: key] }, set: { map[keyPath: key] = $0 })) {
            Text("Not used").tag(Int?.none)
            ForEach(0..<width, id: \.self) { Text(label($0)).tag(Int?.some($0)) }
        }
    }
}

// MARK: - Column mapping and date reading

enum DeadlineMap {
    enum DateFormat { case auto, dmy, mdy, ymd }

    struct Mapping {
        var title: Int, date: Int, course: Int?, notes: Int?
        var headerRow: Bool
        var dateFormat: DateFormat
    }

    struct Row { let row: Int, title: String, date: String, course: String?, notes: String, warning: String? }

    static func columnName(_ i: Int) -> String {
        var s = "", n = i + 1
        while n > 0 { let r = (n - 1) % 26; s = String(UnicodeScalar(65 + r)!) + s; n = (n - 1) / 26 }
        return s
    }

    static func norm(_ s: String) -> String {
        s.lowercased().replacingOccurrences(of: #"[^\p{L}\p{N}]+"#, with: " ", options: .regularExpression).trimmingCharacters(in: .whitespaces)
    }

    /// The same de-duplication key the engine stores with each deadline.
    static func key(_ title: String, _ date: String, _ course: String?) -> String { "\(norm(title))|\(date)|\(norm(course ?? ""))" }

    /// Guess the columns from a header row.
    static func guess(_ header: [JSONValue]) -> Mapping {
        let h = header.map { norm($0.text) }
        let find = { (p: String) in h.firstIndex { $0.range(of: p, options: .regularExpression) != nil } }
        let title = find(#"^(title|event|name|task|assessment|item|description|what)"#)
        let date = find("(date|due|deadline|when|day)")
        return Mapping(title: title ?? 0, date: date ?? (h.count > 1 ? 1 : 0), course: find("(course|subject|module|class)"), notes: find("(note|comment|detail|remark)"),
                       headerRow: title != nil || date != nil, dateFormat: .auto)
    }

    private static let months = ["jan": 1, "feb": 2, "mar": 3, "apr": 4, "may": 5, "jun": 6, "jul": 7, "aug": 8, "sep": 9, "oct": 10, "nov": 11, "dec": 12]

    private static func iso(_ y0: Int, _ m: Int, _ d: Int) -> String? {
        let y = y0 < 100 ? y0 + 2000 : y0
        var c = DateComponents(); c.year = y; c.month = m; c.day = d
        guard let dt = Calendar(identifier: .gregorian).date(from: c), Calendar(identifier: .gregorian).dateComponents([.year, .month, .day], from: dt) == DateComponents(year: y, month: m, day: d) else { return nil }
        return String(format: "%04d-%02d-%02d", y, m, d)
    }

    /// An Excel serial day (1900 system).
    private static func serial(_ n: Double) -> String? {
        guard n >= 1, n <= 2_958_465 else { return nil }
        let d = Date(timeIntervalSince1970: (n.rounded(.down) - 25569) * 86400)
        var cal = Calendar(identifier: .gregorian); cal.timeZone = TimeZone(identifier: "UTC")!
        let c = cal.dateComponents([.year, .month, .day], from: d)
        return iso(c.year!, c.month!, c.day!)
    }

    static func parse(_ v: JSONValue, _ fmt: DateFormat) -> (date: String?, warning: String?) {
        if case .number(let n) = v { return (serial(n), nil) }
        let s = v.text.trimmingCharacters(in: .whitespaces)
        if s.isEmpty { return (nil, nil) }
        func m(_ p: String) -> [String]? {
            guard let r = try? NSRegularExpression(pattern: p), let x = r.firstMatch(in: s, range: NSRange(s.startIndex..., in: s)) else { return nil }
            return (0..<x.numberOfRanges).map { Range(x.range(at: $0), in: s).map { String(s[$0]) } ?? "" }
        }
        if m(#"^\d{5}(\.\d+)?$"#) != nil { return (serial(Double(s)!), nil) }
        if let g = m(#"^(\d{4})[-/.](\d{1,2})[-/.](\d{1,2})(?:[T\s].*)?$"#) { return (iso(Int(g[1])!, Int(g[2])!, Int(g[3])!), nil) }
        if let g = m(#"^(\d{1,2})[-/.](\d{1,2})[-/.](\d{2,4})$"#) {
            let a = Int(g[1])!, b = Int(g[2])!, y = Int(g[3])!
            switch fmt {
            case .dmy: return (iso(y, b, a), nil)
            case .mdy: return (iso(y, a, b), nil)
            case .ymd: return (nil, nil)
            case .auto:
                if a > 12 && b <= 12 { return (iso(y, b, a), nil) }
                if b > 12 && a <= 12 { return (iso(y, a, b), nil) }
                return (iso(y, b, a), "Ambiguous day/month; read as day/month. Pick a date format to be sure.")
            }
        }
        if let g = m(#"^(\d{1,2})(?:st|nd|rd|th)?\s+([A-Za-z]{3,9})\.?,?\s+(\d{2,4})$"#), let mo = months[String(g[2].prefix(3)).lowercased()] { return (iso(Int(g[3])!, mo, Int(g[1])!), nil) }
        if let g = m(#"^([A-Za-z]{3,9})\.?\s+(\d{1,2})(?:st|nd|rd|th)?,?\s+(\d{2,4})$"#), let mo = months[String(g[1].prefix(3)).lowercased()] { return (iso(Int(g[3])!, mo, Int(g[2])!), nil) }
        return (nil, nil)
    }

    static func map(_ rows: [[JSONValue]], _ map: Mapping, existing: Set<String>) -> (valid: [Row], errors: [(row: Int, error: String)], duplicates: Int) {
        var valid: [Row] = [], errors: [(Int, String)] = [], seen = Set<String>(), dups = 0
        for (i, r) in rows.enumerated() {
            if map.headerRow && i == 0 { continue }
            let rowNo = i + 1
            if r.allSatisfy(\.isEmpty) { continue }
            let at = { (c: Int) in c < r.count ? r[c] : JSONValue.null }
            let title = at(map.title).text.trimmingCharacters(in: .whitespaces)
            if title.isEmpty { errors.append((rowNo, "Missing title")); continue }
            let raw = at(map.date)
            let pd = parse(raw, map.dateFormat)
            guard let date = pd.date else { errors.append((rowNo, raw.isEmpty ? "Missing date" : "Unreadable date “\(raw.text)”")); continue }
            let course = map.course.map { at($0).text.trimmingCharacters(in: .whitespaces) }.flatMap { $0.isEmpty ? nil : $0 }
            let notes = map.notes.map { at($0).text.trimmingCharacters(in: .whitespaces) } ?? ""
            let k = key(title, date, course)
            if seen.contains(k) || existing.contains(k) { dups += 1; continue }
            seen.insert(k)
            valid.append(Row(row: rowNo, title: title, date: date, course: course, notes: notes, warning: pd.warning))
        }
        return (valid, errors.map { (row: $0.0, error: $0.1) }, dups)
    }
}

/// A value with its position, for tables of plain arrays.
struct Indexed<T>: Identifiable {
    let id: Int
    let value: T
}

extension Array {
    func indexed() -> [Indexed<Element>] { enumerated().map { Indexed(id: $0.offset, value: $0.element) } }
}
