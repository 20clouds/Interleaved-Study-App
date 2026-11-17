// Plan: the daily goal (a guide, never a nag), the streak with rest days, deadlines as an agenda and a month
// calendar, and importing deadlines from a spreadsheet.
import SwiftUI

struct PlanView: View {
    @Environment(AppModel.self) private var app
    @State private var form: DeadlineForm.Mode?
    @State private var sheet: SheetData?
    @State private var manual = false
    @State private var showPast = false

    var body: some View {
        if let d = app.data, let settings = app.settings {
            Page {
                HStack(alignment: .top, spacing: 48) {
                    goal(d, settings).frame(maxWidth: .infinity, alignment: .topLeading)
                    deadlines(d).frame(maxWidth: .infinity, alignment: .topLeading)
                }
                CalendarView(deadlines: d.deadlines, studied: Dictionary(d.daily.map { ($0.id, $0.minutes) }, uniquingKeysWith: { a, _ in a }),
                             onAdd: { form = .new($0) }, onOpen: { form = .edit($0) })
            }
            .sheet(item: $form) { DeadlineForm(mode: $0) }
            .sheet(item: Binding(get: { sheet.map(SheetBox.init) }, set: { if $0 == nil { sheet = nil } })) { box in
                if manual { SheetImport(sheet: box.value) { sheet = nil } }
                else { AiSheetImport(sheet: box.value, onManual: { manual = true }) { sheet = nil } }
            }
        }
    }

    private func goal(_ d: DataSnapshot, _ settings: Settings) -> some View {
        let today = Days.key()
        let rec = d.daily.first { $0.id == today }
        let goal = rec?.goal ?? DayGoal(kind: settings.goal.kind, value: settings.goal.value)
        let done = goal.kind == "minutes" ? rec?.minutes ?? 0 : rec?.topics ?? 0
        let rest = Int(settings.goal.restDaysPerWeek)
        let streak = Streak(Set(d.daily.filter { $0.minutes > 0 }.map(\.id)), today: today, restPerWeek: rest)
        let setDay = { (g: DayGoal?) in app.perform { try await Engine.run("setDayGoal", g) } }
        return VStack(alignment: .leading, spacing: 14) {
            Text("Daily goal").font(Typeset.heading)
            HStack(spacing: 32) {
                Stat(value: goal.kind == "minutes" ? fmtMin(done) : "\(Int(done))", label: "of \(goal.kind == "minutes" ? fmtMin(goal.value) : "\(Int(goal.value)) topics") goal")
                Stat(value: "\(streak.current)", label: "day streak" + (streak.restUsed > 0 ? ", \(plural(streak.restUsed, "rest day"))" : ""))
                Stat(value: "\(streak.best)", label: "best streak")
            }
            ProgressView(value: min(1, done / max(1, goal.value))).tint(Ink.ink)
            Text("The goal is a guide, not a rule. \(plural(rest, "rest day")) per week \(rest == 1 ? "keeps" : "keep") a streak alive.").font(Typeset.small).foregroundStyle(Ink.ink2)
            HStack(spacing: 6) {
                Text("Today’s goal")
                NumberField(value: goal.value, range: 1...10_000) { setDay(DayGoal(kind: goal.kind, value: $0)) }
                Picker("", selection: Binding(get: { goal.kind }, set: { setDay(DayGoal(kind: $0, value: goal.value)) })) {
                    Text("minutes").tag("minutes"); Text("topics").tag("topics")
                }
                .labelsHidden().fixedSize()
                if rec?.goal != nil { Button("Use Default") { setDay(nil) }.buttonStyle(.borderless) }
            }
            HStack(spacing: 6) {
                Text("Default goal")
                NumberField(value: settings.goal.value, range: 1...10_000) { app.saveSettings(["goal": ["value": $0]]) }
                Picker("", selection: Binding(get: { settings.goal.kind }, set: { app.saveSettings(["goal": ["kind": $0]]) })) {
                    Text("minutes").tag("minutes"); Text("topics").tag("topics")
                }
                .labelsHidden().fixedSize()
                Text("rest days per week")
                NumberField(value: settings.goal.restDaysPerWeek, range: 0...3, width: 40) { app.saveSettings(["goal": ["restDaysPerWeek": $0]]) }
            }
        }
    }

    private func deadlines(_ d: DataSnapshot) -> some View {
        let today = Days.key()
        let list = d.deadlines.filter { showPast || $0.date >= today }.sorted { $0.date < $1.date }
        return VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Deadlines").font(Typeset.heading)
                Spacer()
                Button { form = .new(nil) } label: { Label("Add", systemImage: "plus") }
                Button { Task { if let s: SheetData? = await app.act({ try await Engine.call("openSheet") }), let s { manual = false; sheet = s } } } label: {
                    Label("Import .xlsx or .csv", systemImage: "square.and.arrow.down")
                }
            }
            if list.isEmpty {
                Text("No upcoming deadlines. Linked deadlines give their topics priority as the date approaches (from three weeks out).").foregroundStyle(Ink.ink2)
            } else {
                ScrollView {
                    Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 6) {
                        ForEach(list) { dl in
                            let n = Days.between(today, dl.date)
                            GridRow {
                                Text(Days.date(dl.date).formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day())).monospacedDigit().foregroundStyle(Ink.ink2)
                                HStack(spacing: 6) {
                                    Dot(color: app.courseOf(dl).map { app.color($0.id) } ?? Ink.ink3, size: 7)
                                    Text(dl.title)
                                    if let l = link(dl) { Text("(\(l))").font(Typeset.small).foregroundStyle(Ink.ink2) }
                                }
                                Text(Days.countdown(dl.date)).font(Typeset.small.monospacedDigit()).foregroundStyle(n <= 7 && n >= 0 ? Ink.due : Ink.ink2)
                                HStack(spacing: 2) {
                                    Button { form = .edit(dl) } label: { Image(systemName: "pencil") }.help("Edit")
                                    Button {
                                        app.confirmThen("Delete “\(dl.title)”?", detail: "The deadline is removed; topics are not affected.", ok: "Delete", danger: true) { try await Engine.run("deleteDeadline", dl.id) }
                                    } label: { Image(systemName: "trash") }.help("Delete")
                                }
                                .buttonStyle(.borderless)
                            }
                        }
                    }
                }
                .frame(maxHeight: 260)
            }
            Toggle("Show past deadlines", isOn: $showPast).font(Typeset.small)
        }
    }

    private func link(_ d: Deadline) -> String? {
        if let ids = d.topicIds, !ids.isEmpty { return "\(ids.count) topic(s)" }
        if let id = d.docId { return app.docs[id]?.title }
        if let id = d.courseId { return app.courses[id]?.name }
        return nil
    }
}

// MARK: - Calendar

/// A month laid out like Calendar's month view: "‹ Today ›", weekends shaded, today circled, deadline chips in the
/// course colour, minutes studied per day, "N more" opening the day, double-click or "+" to add a deadline.
private struct CalendarView: View {
    @Environment(AppModel.self) private var app
    let deadlines: [Deadline]
    let studied: [String: Double]
    let onAdd: (String) -> Void
    let onOpen: (Deadline) -> Void
    @State private var month = Calendar.current.dateInterval(of: .month, for: Date())!.start
    @State private var more: String?

    private var cal: Calendar { Calendar.current }

    var body: some View {
        let byDay = Dictionary(grouping: deadlines, by: \.date).mapValues { $0.sorted { $0.title < $1.title } }
        let weeks = weeks
        let today = Days.key()
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                (Text(month.formatted(.dateTime.month(.wide))).bold() + Text(" ") + Text(month.formatted(.dateTime.year()))).font(Typeset.title)
                Spacer()
                ControlGroup {
                    Button { go(-1) } label: { Image(systemName: "chevron.left") }.help("Previous month")
                    Button("Today") { month = cal.dateInterval(of: .month, for: Date())!.start }
                    Button { go(1) } label: { Image(systemName: "chevron.right") }.help("Next month")
                }
                .fixedSize()
            }
            Grid(horizontalSpacing: 0, verticalSpacing: 0) {
                GridRow {
                    ForEach(0..<7, id: \.self) { i in
                        let wd = (cal.firstWeekday - 1 + i) % 7
                        Text(cal.shortWeekdaySymbols[wd]).font(Typeset.small).foregroundStyle(Ink.ink2).frame(maxWidth: .infinity).padding(.bottom, 6)
                    }
                }
                ForEach(weeks, id: \.self) { week in
                    GridRow {
                        ForEach(week, id: \.self) { day in cell(day, byDay[Days.key(day)] ?? [], today: today) }
                    }
                }
            }
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Ink.line, lineWidth: 0.5))
            legend(weeks.flatMap { $0 }.flatMap { byDay[Days.key($0)] ?? [] }, anyStudied: weeks.flatMap { $0 }.contains { (studied[Days.key($0)] ?? 0) >= 1 })
        }
    }

    private func go(_ n: Int) { month = cal.date(byAdding: .month, value: n, to: month)! }

    /// Only as many weeks as the month needs, each starting on the system's first weekday.
    private var weeks: [[Date]] {
        let lead = (cal.component(.weekday, from: month) - cal.firstWeekday + 7) % 7
        let days = cal.range(of: .day, in: .month, for: month)!.count
        let rows = (lead + days + 6) / 7
        return (0..<rows).map { w in (0..<7).map { i in cal.date(byAdding: .day, value: w * 7 + i - lead, to: month)! } }
    }

    private func cell(_ day: Date, _ list: [Deadline], today: String) -> some View {
        let k = Days.key(day)
        let inMonth = cal.isDate(day, equalTo: month, toGranularity: .month)
        let weekend = cal.isDateInWeekend(day)
        let shown = list.count > 3 ? 2 : list.count
        let mins = studied[k] ?? 0
        return VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 4) {
                if mins >= 1 { Text(fmtMin(mins)).font(.system(size: 10).monospacedDigit()).foregroundStyle(Ink.ok).help("\(fmtMin(mins)) studied") }
                Spacer()
                if cal.component(.day, from: day) == 1 { Text(day.formatted(.dateTime.month(.abbreviated))).font(Typeset.small).foregroundStyle(Ink.ink2) }
                Text("\(cal.component(.day, from: day))")
                    .font(.system(size: 12, weight: k == today ? .semibold : .regular).monospacedDigit())
                    .foregroundStyle(k == today ? Ink.paper : inMonth ? Ink.ink : Ink.ink3)
                    .frame(minWidth: 20, minHeight: 20)
                    .background(Circle().fill(k == today ? Ink.ink : .clear))
            }
            ForEach(list.prefix(shown)) { chip($0, past: k < today) }
            if list.count > shown {
                Button("\(list.count - shown) more") { more = k }
                    .buttonStyle(.borderless).font(Typeset.small)
                    .popover(isPresented: Binding(get: { more == k }, set: { if !$0 { more = nil } })) { dayCard(day, list) }
            }
            Spacer(minLength: 0)
        }
        .padding(5)
        .frame(maxWidth: .infinity, minHeight: 92, alignment: .topLeading)
        .background(weekend ? Ink.panel : Ink.paper)
        .overlay(Rectangle().stroke(Ink.line, lineWidth: 0.5))
        .opacity(inMonth ? 1 : 0.7)
        .contentShape(Rectangle())
        .onTapGesture(count: 2) { onAdd(k) }
        .contextMenu { Button("Add Deadline on \(day.formatted(.dateTime.month(.wide).day()))…") { onAdd(k) } }
        .help("Double-click to add a deadline")
    }

    private func chip(_ d: Deadline, past: Bool) -> some View {
        let c = app.courseOf(d)
        let color = c.map { app.color($0.id) } ?? Ink.ink3
        return Button { more = nil; onOpen(d) } label: {
            Text(d.title)
                .font(Typeset.small).lineLimit(1)
                .padding(.horizontal, 5).padding(.vertical, 2)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 4).fill(color.opacity(past ? 0.08 : 0.16)))
                .overlay(alignment: .leading) { RoundedRectangle(cornerRadius: 1).fill(color).frame(width: 2) }
                .foregroundStyle(past ? Ink.ink2 : Ink.ink)
        }
        .buttonStyle(.plain)
        .help(d.title + (c.map { " · \($0.name)" } ?? "") + (d.notes.map { $0.isEmpty ? "" : "\n\($0)" } ?? ""))
    }

    private func dayCard(_ day: Date, _ list: [Deadline]) -> some View {
        let k = Days.key(day)
        return VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(day.formatted(.dateTime.weekday(.wide).month(.wide).day())).font(Typeset.heading)
                Spacer()
                if let m = studied[k], m >= 1 { Text("\(fmtMin(m)) studied").font(Typeset.small).foregroundStyle(Ink.ok) }
            }
            ForEach(list) { chip($0, past: k < Days.key()) }
            Button { more = nil; onAdd(k) } label: { Label("Add Deadline", systemImage: "plus") }.buttonStyle(.borderless).font(Typeset.small)
        }
        .padding(12)
        .frame(width: 260)
    }

    @ViewBuilder private func legend(_ shown: [Deadline], anyStudied: Bool) -> some View {
        let courses = Array(Set(shown.compactMap { app.courseOf($0) })).sorted { $0.name < $1.name }
        let unlinked = shown.contains { app.courseOf($0) == nil }
        if !courses.isEmpty || unlinked || anyStudied {
            HStack(spacing: 16) {
                ForEach(courses) { c in HStack(spacing: 6) { Dot(color: app.color(c.id)); Text(c.name) } }
                if unlinked { HStack(spacing: 6) { Dot(color: Ink.ink3); Text("No course") } }
                if anyStudied { HStack(spacing: 6) { Text("45m").foregroundStyle(Ink.ok).monospacedDigit(); Text("time studied that day") } }
            }
            .font(Typeset.small).foregroundStyle(Ink.ink2)
        }
    }
}
