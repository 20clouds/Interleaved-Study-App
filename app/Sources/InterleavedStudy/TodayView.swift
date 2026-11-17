// Today, the home: what is due, the order the app would study it in (drawn as the interleave line), one Start
// button, and beside it the day's goal, the next deadlines and each course's standing. Everything is real data;
// nothing is estimated beyond "minutes per topic × topics".
import SwiftUI

struct TodayView: View {
    @Environment(AppModel.self) private var app
    @State private var order: [Suggestion]?

    var body: some View {
        if let d = app.data, let settings = app.settings {
            if d.courses.isEmpty {
                EmptyState(title: "Start with a course", message: "A course holds the lecture files for one subject. Each file becomes a lesson, the AI splits it into topics, and this page then shows what to study and in which order. You can also drop lecture files anywhere in this window.") {
                    Button { app.screen = .library; app.courseWizard = [] } label: { Label("New Course", systemImage: "folder.badge.plus") }
                        .buttonStyle(.primaryBig)
                }
            } else {
                page(d, settings)
                    .task(id: app.dataVersion) {
                        order = d.topics.isEmpty ? [] : ((try? await Engine.call("sessionPreview", Pool.global, false)) ?? [])
                    }
            }
        }
    }

    private func page(_ d: DataSnapshot, _ settings: Settings) -> some View {
        let now = app.now
        let due = d.topics.filter { $0.isDue(now) }
        let fresh = d.topics.filter { $0.status(now) == .new }
        let nextDue = d.topics.filter { ($0.due ?? 0) > now }.min { ($0.due ?? 0) < ($1.due ?? 0) }
        let top = Array((order ?? []).prefix(8))
        let dueMin = due.reduce(0) { $0 + app.perTopic($1) }

        var headline: String, sub: String
        if !due.isEmpty {
            headline = "\(plural(due.count, "topic is", "topics are")) due for review."
            sub = "About \(fmtMin(dueMin)) at your current pace" + (fresh.isEmpty ? "." : ", with \(plural(fresh.count, "new topic")) ready after them.")
        } else if !fresh.isEmpty {
            headline = "Nothing is due."
            sub = "\(plural(fresh.count, "new topic is", "new topics are")) ready to learn."
        } else {
            headline = "All caught up."
            sub = nextDue.map { "The next review is \(fmtRel($0.due, now)): \($0.name)." } ?? "Nothing is scheduled yet."
        }

        return Page {
            header(headline: headline, sub: sub, canStart: !top.isEmpty)
            HStack(alignment: .top, spacing: 48) {
                upNext(top, d)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                VStack(alignment: .leading, spacing: 32) {
                    goal(d, settings)
                    deadlines(d)
                    courses(d)
                }
                .frame(width: 300)
            }
        }
    }

    // MARK: Header

    @ViewBuilder private func header(headline: String, sub: String, canStart: Bool) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(Date().formatted(.dateTime.weekday(.wide).month(.wide).day())).font(Typeset.small).foregroundStyle(Ink.ink2)
            if let s = app.session {
                Text("You are in a session.").font(Typeset.display)
                TimelineView(.periodic(from: .now, by: 1)) { ctx in
                    lede(s, ctx.date)
                }
                HStack(spacing: 12) {
                    Button { app.screen = .session } label: { Label("Back to the Session", systemImage: "timer") }.buttonStyle(.primaryBig)
                    if s.phase == "work" || s.phase == "rate", let t = s.topicId.flatMap({ app.topics[$0] }) { DeckLabel(doc: app.docs[t.docId]) }
                }
                .padding(.top, 6)
            } else {
                Text(headline).font(Typeset.display)
                Text(sub).font(.system(size: 15)).foregroundStyle(Ink.ink2)
                HStack(spacing: 12) {
                    Button { Task { await app.startSession() } } label: { HStack(spacing: 8) { Label("Start Studying", systemImage: "play.fill"); KeyHint(key: "S") } }
                        .buttonStyle(.primaryBig).disabled(!canStart)
                    Button("Choose What to Study") { app.screen = .session }.controlSize(.large)
                }
                .padding(.top, 6)
            }
        }
    }

    private func lede(_ s: SessionState, _ now: Date) -> some View {
        let serifName = Text(s.topicName).font(Typeset.serif(15)).foregroundStyle(Ink.ink)
        let t: Text = switch s.phase {
        case "work": Text("Studying ") + serifName + Text(", \(fmtClock(s.left(now))) left.")
        case "break": Text("On a break, \(fmtClock(s.left(now))) left.")
        case "rate": Text("Rate ") + serifName + Text(" to go on.")
        default: Text("Choose the next topic to go on.")
        }
        return t.font(.system(size: 15)).foregroundStyle(Ink.ink2)
    }

    // MARK: Up next

    private func upNext(_ top: [Suggestion], _ d: DataSnapshot) -> some View {
        let topCourses = Array(NSOrderedSet(array: top.compactMap { app.topics[$0.topicId]?.courseId })) as? [String] ?? []
        return VStack(alignment: .leading, spacing: 12) {
            SectionHead("Up next") {
                if topCourses.count > 1 { Text("Mixed across \(listNames(topCourses.compactMap { app.courses[$0]?.name }))").foregroundStyle(Ink.ink2) }
            }
            if order == nil {
                ProgressView().controlSize(.small)
            } else if top.isEmpty {
                Text("No topics yet. Lessons appear here once they are imported.").foregroundStyle(Ink.ink2)
            } else {
                Lanes(items: top.enumerated().map { i, s in
                    let t = app.topics[s.topicId]
                    return LaneItem(id: s.topicId, courseId: t?.courseId ?? "", state: i == 0 ? .next : .later, name: s.name,
                                    deck: t.flatMap { app.docs[$0.docId] }, meta: s.reason, aside: t.map { fmtMin(app.perTopic($0)) } ?? "",
                                    help: "Start a session with this topic",
                                    action: app.session == nil ? { Task { await app.startSession(topicId: s.topicId) } } : nil)
                }, label: "Suggested study order")
            }
            if let o = order, o.count > top.count {
                Text("\(plural(o.count - top.count, "more topic")) after these. The order is worked out again after every block.")
                    .font(Typeset.small).foregroundStyle(Ink.ink2)
            }
        }
    }

    // MARK: Side

    private func goal(_ d: DataSnapshot, _ settings: Settings) -> some View {
        let today = Days.key()
        let rec = d.daily.first { $0.id == today }
        let goal = rec?.goal ?? DayGoal(kind: settings.goal.kind, value: settings.goal.value)
        let done = goal.kind == "minutes" ? rec?.minutes ?? 0 : rec?.topics ?? 0
        let streak = Streak(Set(d.daily.filter { $0.minutes > 0 }.map(\.id)), today: today, restPerWeek: Int(settings.goal.restDaysPerWeek))
        return VStack(alignment: .leading, spacing: 8) {
            SectionHead("Today’s goal") { Button("Change") { app.screen = .plan }.buttonStyle(QuietLink()) }
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(goal.kind == "minutes" ? fmtMin(done) : "\(Int(done))").font(.system(size: 22, weight: .semibold).monospacedDigit())
                Text("of \(goal.kind == "minutes" ? fmtMin(goal.value) : plural(Int(goal.value), "topic"))").foregroundStyle(Ink.ink2)
            }
            ProgressView(value: min(1, done / max(1, goal.value))).tint(Ink.ink).accessibilityLabel("Goal progress")
            Text((streak.current > 0 ? "\(plural(streak.current, "day")) in a row" : "No streak yet") + (streak.best > streak.current ? ", best \(streak.best)" : "") + ". The goal is a guide, not a rule.")
                .font(Typeset.small).foregroundStyle(Ink.ink2)
        }
    }

    private func deadlines(_ d: DataSnapshot) -> some View {
        let today = Days.key()
        let list = d.deadlines.filter { $0.date >= today }.sorted { $0.date < $1.date }.prefix(5)
        return VStack(alignment: .leading, spacing: 8) {
            SectionHead("Deadlines") { Button("Plan") { app.screen = .plan }.buttonStyle(QuietLink()) }
            if list.isEmpty {
                Text("None coming up. Linked deadlines give their topics priority from three weeks out.").font(Typeset.small).foregroundStyle(Ink.ink2)
            } else {
                ForEach(list) { dl in
                    let n = Days.between(today, dl.date)
                    HStack(spacing: 8) {
                        Dot(color: app.courseOf(dl).map { app.color($0.id) } ?? Ink.ink3)
                        Text(Days.date(dl.date).formatted(.dateTime.month(.abbreviated).day())).monospacedDigit().foregroundStyle(Ink.ink2).frame(width: 48, alignment: .leading)
                        Text(dl.title).lineLimit(1)
                        Spacer()
                        Text(n == 0 ? "today" : n == 1 ? "tomorrow" : "\(n) days").monospacedDigit().foregroundStyle(n <= 7 ? Ink.due : Ink.ink2)
                    }
                    .font(Typeset.body)
                }
            }
        }
    }

    private func courses(_ d: DataSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            SectionHead("Courses") { Button("Library") { app.scope = .all; app.screen = .library }.buttonStyle(QuietLink()) }
                .padding(.bottom, 4)
            ForEach(d.courses) { c in
                let rows = d.topics.filter { $0.courseId == c.id }
                let due = rows.filter { $0.isDue(app.now) }.count
                let m = rows.isEmpty ? 0 : rows.reduce(0) { $0 + $1.mastery(app.now) } / rows.count
                Button { app.scope = .course(c.id); app.screen = .library } label: {
                    HStack(spacing: 8) {
                        Dot(color: app.color(c.id))
                        Text(c.name).lineLimit(1)
                        Spacer()
                        Text(due > 0 ? "\(due) due" : plural(rows.count, "topic")).font(Typeset.small.monospacedDigit()).foregroundStyle(due > 0 ? Ink.due : Ink.ink2)
                        MasteryBar(value: m)
                    }
                    .padding(.vertical, 6).padding(.horizontal, 6)
                    .contentShape(Rectangle())
                }
                .buttonStyle(HoverRow())
            }
        }
    }
}

/// A plain row that highlights on hover.
struct HoverRow: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View { Hovered(configuration: configuration) }
    private struct Hovered: View {
        let configuration: Configuration
        @State private var hover = false
        var body: some View {
            configuration.label
                .background(RoundedRectangle(cornerRadius: 6).fill(configuration.isPressed ? Ink.sel : hover ? Ink.hover : .clear))
                .onHover { hover = $0 }
                .animation(.quick, value: hover)
        }
    }
}
