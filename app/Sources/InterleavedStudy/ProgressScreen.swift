// Progress: plain charts of real data. Minutes per day, mastery over time, time per course and per topic, what is
// due today, recent reviews, the streak and the next deadlines.
import Charts
import SwiftUI

struct ProgressScreen: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        if let d = app.data, let settings = app.settings {
            if d.topics.isEmpty {
                EmptyState(title: "Progress appears once you study", message: "Import lessons and run a session; time, ratings and mastery are tracked from then on.") { EmptyView() }
            } else {
                page(d, settings)
            }
        }
    }

    private func page(_ d: DataSnapshot, _ settings: Settings) -> some View {
        let now = app.now
        let today = Days.key()
        let last30 = (0..<30).map { Days.add(today, $0 - 29) }
        let daily = Dictionary(d.daily.map { ($0.id, $0.minutes) }, uniquingKeysWith: { a, _ in a })
        let endToday = Days.date(today).ms + DAY - 1
        let dueToday = d.topics.filter { ($0.due ?? .infinity) <= endToday }.sorted { ($0.due ?? 0) < ($1.due ?? 0) }
        let total = d.daily.reduce(0) { $0 + $1.minutes }
        let week = last30.suffix(7).reduce(0) { $0 + (daily[$1] ?? 0) }
        let avg = d.topics.reduce(0) { $0 + $1.mastery(now) } / max(1, d.topics.count)
        let streak = Streak(Set(d.daily.filter { $0.minutes > 0 }.map(\.id)), today: today, restPerWeek: Int(settings.goal.restDaysPerWeek))
        let perCourse = d.courses.map { c in (c, d.topics.filter { $0.courseId == c.id }.reduce(0) { $0 + $1.spent }) }.filter { $0.1 > 0 }.sorted { $0.1 > $1.1 }
        let topTopics = d.topics.filter { $0.spent > 0 }.sorted { $0.spent > $1.spent }.prefix(12)
        let reviews = d.topics.flatMap { t in t.reviews.map { (t, $0) } }.sorted { $0.1.at > $1.1.at }.prefix(25)

        return Page {
            HStack(spacing: 40) {
                Stat(value: fmtMin(total), label: "studied in total")
                Stat(value: fmtMin(week), label: "last 7 days")
                Stat(value: "\(dueToday.count)", label: "due by tonight")
                Stat(value: "\(avg)", label: "average mastery")
                Stat(value: "\(streak.current)", label: "day streak")
            }
            HStack(alignment: .top, spacing: 40) {
                block("Minutes per day, last 30 days") {
                    Chart(last30, id: \.self) { k in
                        BarMark(x: .value("Day", Days.date(k), unit: .day), y: .value("Minutes", daily[k] ?? 0))
                            .foregroundStyle(Ink.ink2)
                    }
                    .chartYScale(domain: 0...max(30, last30.map { daily[$0] ?? 0 }.max() ?? 0))
                    .chartXAxis { AxisMarks(values: .stride(by: .day, count: 7)) { _ in AxisValueLabel(format: .dateTime.month(.abbreviated).day()) } }
                    .frame(height: 140)
                    .accessibilityLabel("Minutes studied per day")
                }
                block("Average mastery over time") {
                    let points = d.snapshots.suffix(90)
                    if points.count < 2 {
                        Text("One point per day appears here once the app has been opened on two or more days.").font(Typeset.small).foregroundStyle(Ink.ink2)
                    } else {
                        Chart(Array(points)) { s in
                            LineMark(x: .value("Day", Days.date(s.id), unit: .day), y: .value("Mastery", s.avg)).foregroundStyle(Ink.ink)
                        }
                        .chartYScale(domain: 0...100)
                        .frame(height: 140)
                        .accessibilityLabel("Average mastery")
                    }
                    Text("Recorded once a day when the app starts. Mastery decays between reviews, so a flat line means reviews keep pace.").font(Typeset.small).foregroundStyle(Ink.ink2)
                }
            }
            HStack(alignment: .top, spacing: 40) {
                block("Time per course") {
                    if perCourse.isEmpty { Text("No time recorded yet.").foregroundStyle(Ink.ink2) } else {
                        bars(perCourse.map { ($0.0.id, $0.0.name, $0.1, app.color($0.0.id), nil) })
                    }
                }
                block("Time per topic") {
                    if topTopics.isEmpty { Text("No time recorded yet.").foregroundStyle(Ink.ink2) } else {
                        bars(topTopics.map { ($0.id, $0.name, $0.spent, app.color($0.courseId), $0.id) })
                    }
                }
            }
            HStack(alignment: .top, spacing: 40) {
                block("Due by tonight") {
                    if dueToday.isEmpty { Text("Nothing is due today.").foregroundStyle(Ink.ink2) } else {
                        ForEach(dueToday.prefix(20)) { t in
                            Button { open(t.id) } label: {
                                HStack {
                                    Text(t.name).font(Typeset.topic).lineLimit(1)
                                    DeckLabel(doc: app.docs[t.docId])
                                    Spacer()
                                    Text(t.isDue(now) ? "overdue \(fixed1(max(0, (now - (t.due ?? now)) / DAY)))d" : "later today").foregroundStyle(t.isDue(now) ? Ink.due : Ink.ink2)
                                    Text("\(t.mastery(now))").monospacedDigit().frame(width: 28, alignment: .trailing)
                                }
                                .padding(.vertical, 4).padding(.horizontal, 6).contentShape(Rectangle())
                            }
                            .buttonStyle(HoverRow())
                        }
                    }
                }
                block("Deadlines") {
                    let next = d.deadlines.filter { $0.date >= today }.sorted { $0.date < $1.date }.prefix(6)
                    if next.isEmpty { Text("No upcoming deadlines.").foregroundStyle(Ink.ink2) } else {
                        ForEach(next) { dl in
                            let n = Days.between(today, dl.date)
                            HStack {
                                Text(dl.title)
                                Spacer()
                                Text(Days.date(dl.date).formatted(.dateTime.month(.abbreviated).day())).monospacedDigit().foregroundStyle(Ink.ink2)
                                Text(n == 0 ? "today" : "\(n) days").monospacedDigit().foregroundStyle(n <= 7 ? Ink.due : Ink.ink2).frame(width: 60, alignment: .trailing)
                            }
                            .padding(.vertical, 4)
                        }
                    }
                }
            }
            block("Recent reviews") {
                if reviews.isEmpty { Text("No reviews yet.").foregroundStyle(Ink.ink2) } else {
                    Grid(alignment: .leading, horizontalSpacing: 20, verticalSpacing: 6) {
                        GridRow {
                            Text("Date"); Text("Topic"); Text("Rating"); Text("Minutes").gridColumnAlignment(.trailing); Text("Quiz").gridColumnAlignment(.trailing)
                        }
                        .font(Typeset.heading)
                        ForEach(Array(reviews.enumerated()), id: \.offset) { _, x in
                            GridRow {
                                Text(date(x.1.at).formatted(.dateTime.month(.abbreviated).day().year())).foregroundStyle(Ink.ink2)
                                Button { open(x.0.id) } label: { Text(x.0.name).font(Typeset.topic).lineLimit(1) }.buttonStyle(.plain)
                                Text(x.1.rating.rawValue)
                                Text(fmtMin(x.1.minutes)).monospacedDigit()
                                Text(x.1.quiz.map { "\(Int(($0 * 100).rounded()))%" } ?? "").monospacedDigit()
                            }
                        }
                    }
                }
            }
        }
    }

    private func open(_ id: String) {
        app.selected = id
        if let t = app.topics[id] { app.scope = .course(t.courseId) }
        app.screen = .library
    }

    private func block<C: View>(_ title: String, @ViewBuilder _ c: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(Typeset.heading)
            c()
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }

    /// Horizontal bars in the course colour, longest first.
    private func bars(_ rows: [(id: String, name: String, minutes: Double, color: Color, topic: String?)]) -> some View {
        let max = rows.map(\.minutes).max() ?? 1
        return Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 8) {
            ForEach(rows, id: \.id) { r in
                GridRow {
                    Group {
                        if let id = r.topic { Button { open(id) } label: { Text(r.name).font(Typeset.topic) }.buttonStyle(.plain) } else { Text(r.name) }
                    }
                    .lineLimit(1)
                    .frame(maxWidth: 220, alignment: .leading)
                    GeometryReader { g in
                        Capsule().fill(r.color).frame(width: max > 0 ? g.size.width * r.minutes / max : 0, height: 6).frame(maxHeight: .infinity)
                    }
                    .frame(height: 14)
                    Text(fmtMin(r.minutes)).monospacedDigit().foregroundStyle(Ink.ink2).gridColumnAlignment(.trailing)
                }
            }
        }
    }
}
