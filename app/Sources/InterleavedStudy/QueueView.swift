// The AI queue: what a run costs, live progress, pause and resume (including after a provider rate limit), cancel,
// and failures with retry.
import SwiftUI

struct QueueView: View {
    @Environment(AppModel.self) private var app
    @Environment(\.openSettings) private var openSettings
    @State private var estimate: Estimate?

    private let statusText = ["awaiting": "Awaiting AI organization", "queued": "Organizing", "organized": "Organized", "needs": "Needs organizing", "error": "Import problem"]

    var body: some View {
        if let q = app.queue, let d = app.data {
            let routine = app.settings?.ai.provider == "routine"
            let awaiting = d.docs.filter { $0.status == "awaiting" || $0.status == "needs" }
            Page {
                banners(q, routine)
                VStack(alignment: .leading, spacing: 10) {
                    Text("Waiting to be organized").font(Typeset.heading)
                    if awaiting.isEmpty {
                        Text("Every lesson is organized or in the queue.").foregroundStyle(Ink.ink2)
                    } else {
                        Text("\(plural(awaiting.count, "lesson")) not organized yet" + (estimate.map { ": about \(plural($0.requests, "request")), \(Int($0.tokens).formatted()) tokens" + ($0.cached > 0 ? " (\($0.cached) windows cached)" : "") } ?? "") + ".")
                        Button { app.perform("Organizing started") { try await Engine.run("startQueue") } } label: { Label("Start Organizing", systemImage: "sparkles") }
                            .buttonStyle(.primary).disabled(q.noKey)
                    }
                }
                progress(q, routine)
                if !q.failed.isEmpty { failed(q) }
                lessons(d)
            }
            .task(id: app.dataVersion) { estimate = try? await Engine.call("estimate") }
        } else {
            ProgressView().controlSize(.small).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    @ViewBuilder private func banners(_ q: QueueStatus, _ routine: Bool) -> some View {
        if q.noKey {
            banner(routine ? "Finish the scheduled task setup to organize lessons and make quizzes." : "Add an API key for the selected AI provider to organize lessons and make quizzes.", alert: true) {
                Button("Open Settings") { openSettings() }.buttonStyle(.primary)
            }
        }
        if q.userPaused && q.rateLimit == nil {
            banner(q.pauseReason ?? "The queue is paused.") {
                Button { app.perform { try await Engine.run("resumeQueue") } } label: { Label("Resume", systemImage: "play.fill") }.buttonStyle(.primary)
            }
        }
        if !q.userPaused, let until = q.pausedUntil {
            TimelineView(.periodic(from: .now, by: 1)) { ctx in
                banner("\(q.pauseReason ?? "Waiting before trying again.") Next try in \(max(0, Int(((until - ctx.date.ms) / 1000).rounded(.up)))) s.", alert: true) {
                    Button("Try Now") { app.perform { try await Engine.run("resumeQueue") } }.buttonStyle(.secondary)
                }
            }
        }
    }

    private func banner<A: View>(_ text: String, alert: Bool = false, @ViewBuilder actions: () -> A) -> some View {
        HStack(spacing: 12) {
            Text(text).foregroundStyle(alert ? Ink.due : Ink.ink)
            Spacer()
            actions()
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 8).fill(Ink.panel))
    }

    private func progress(_ q: QueueStatus, _ routine: Bool) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                Text("Progress").font(Typeset.heading)
                Spacer()
                if q.userPaused {
                    Button { app.perform { try await Engine.run("resumeQueue") } } label: { Label(q.rateLimit != nil ? "Continue" : "Resume", systemImage: "play.fill") }
                } else {
                    Button { app.perform { try await Engine.run("pauseQueue") } } label: { Label("Pause", systemImage: "pause.fill") }
                }
                if q.queued > 0 { Button { app.perform("AI requests cancelled") { try await Engine.run("cancelAi") } } label: { Label("Cancel All", systemImage: "xmark") } }
            }
            .buttonStyle(.secondary)
            HStack(spacing: 32) {
                Stat(value: "\(q.docsDone)/\(q.docsTotal)", label: "lessons done")
                Stat(value: "\(q.chunksDone)/\(q.chunksTotal)", label: "windows done")
            }
            if let running = q.running {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(running)
                    Spacer()
                    if let id = q.runningId { Button("Cancel") { app.perform("Request cancelled") { try await Engine.run("cancelAi", ["jobId": id]) } }.buttonStyle(.secondary) }
                }
            }
            if let run = q.run {
                HStack {
                    Text(runText(run))
                    Spacer()
                    if let u = run.url.flatMap(URL.init(string:)) { Link(destination: u) { Label("Watch the Run", systemImage: "arrow.up.right.square") }.buttonStyle(.quiet) }
                }
            }
            Text((routine ? "Requests travel to your Claude scheduled task in runs, several at a time. Cancel" : "One request at a time, as fast as the provider allows. Cancel")
                 + " stops the request in flight at once; a lesson whose organizing is cancelled keeps the topics it had, and windows that already finished stay cached for next time. If the provider's rate limit is reached, everything pauses and you get a notification; Continue picks up exactly where it stopped. If the app closes mid-run, the queue resumes on the next launch.")
                .font(Typeset.small).foregroundStyle(Ink.ink2).fixedSize(horizontal: false, vertical: true)
        }
    }

    private func runText(_ r: RunInfo) -> String {
        let n = plural(r.requests, "request")
        let time = { (t: Double) in date(t).formatted(date: .omitted, time: .shortened) }
        switch r.state {
        case "waiting": return "Claude is working on \(n), started at \(time(r.firedAt)). Runs usually take a few minutes."
        case "holding": return "\(n) waiting for the next run" + (r.nextFireAt.map { " at \(time($0))" } ?? "") + "."
        default: return "Sending \(n) to the scheduled task."
        }
    }

    private func failed(_ q: QueueStatus) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Failed").font(Typeset.heading)
            ForEach(q.failed) { f in
                HStack(alignment: .top, spacing: 12) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(f.label)
                        Text(f.error).font(Typeset.small).foregroundStyle(Ink.due).textSelection(.enabled)
                    }
                    Spacer()
                    Button("Discard") { app.perform { try await Engine.run("discardJob", f.id) } }.buttonStyle(.quiet)
                    Button { app.perform { try await Engine.run("retryJob", f.id) } } label: { Label("Retry", systemImage: "arrow.clockwise") }.buttonStyle(.secondary)
                }
                Divider()
            }
        }
    }

    private func lessons(_ d: DataSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Lessons").font(Typeset.heading)
            if d.docs.isEmpty {
                Text("No lessons imported yet.").foregroundStyle(Ink.ink2)
            } else {
                // A card sized to its rows, in place of a table: the list is short and lives inside the scrolling page.
                Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 0) {
                    GridRow {
                        Text("Lesson").frame(maxWidth: .infinity, alignment: .leading)
                        Text("Status")
                        Text("Words").gridColumnAlignment(.trailing)
                        Text("Topics").gridColumnAlignment(.trailing)
                    }
                    .font(Typeset.small).foregroundStyle(Ink.ink2)
                    .padding(.vertical, 8)
                    ForEach(d.docs) { doc in
                        Divider()
                        GridRow(alignment: .firstTextBaseline) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(doc.title).font(Typeset.serif(13))
                                if let n = doc.importNote { Text(n).font(Typeset.small).foregroundStyle(Ink.ink2) }
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            VStack(alignment: .leading, spacing: 4) {
                                Text((statusText[doc.status] ?? doc.status) + (doc.status == "queued" ? " \(doc.winsDone)/\(doc.windows)" : ""))
                                    .foregroundStyle(doc.status == "needs" || doc.status == "error" ? Ink.due : Ink.ink)
                                if let e = doc.error { Text(e).font(Typeset.small).foregroundStyle(Ink.ink2).lineLimit(2).frame(maxWidth: 260, alignment: .leading).help(e) }
                                if doc.status == "needs" || doc.status == "error" {
                                    Button { app.perform { try await Engine.run("retryDoc", doc.id) } } label: { Label("Retry", systemImage: "arrow.clockwise") }.buttonStyle(.secondary).controlSize(.small)
                                }
                            }
                            Text(doc.words.formatted()).monospacedDigit()
                            Text("\(d.topics.filter { $0.docId == doc.id }.count)").monospacedDigit()
                        }
                        .padding(.vertical, 10)
                    }
                }
                .padding(.horizontal, 16).padding(.bottom, 4)
                .background(RoundedRectangle(cornerRadius: 8).fill(Ink.panel))
            }
        }
    }
}
