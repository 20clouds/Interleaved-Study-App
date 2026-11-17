// A quiz on one topic, built from that topic's text only (cached). Multiple choice is scored here; short answers are
// marked by the student against the key, or graded by the AI on request. Either AI request can be cancelled, and
// closing the quiz cancels one still running.
import SwiftUI

struct QuizPanel: View {
    @Environment(AppModel.self) private var app
    let topicId: String
    var scoreLabel = "Use This Score"
    let onScore: (Double) -> Void

    @State private var quiz: Quiz?
    @State private var error: String?
    @State private var loading = true
    @State private var answers: [Int: String] = [:]
    @State private var checked = false
    @State private var marks: [Int: Bool] = [:]
    @State private var grades: [Int: Grade] = [:]
    @State private var grading = false
    @State private var seq = 0 // only the latest load may set state: an earlier, cancelled one settles late

    var body: some View {
        Group {
            if loading {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text("Preparing a quiz from this topic’s text (cached after the first time)…").foregroundStyle(Ink.ink2)
                    Button("Cancel") { cancel("quiz") }
                }
            } else if let quiz, error == nil {
                questions(quiz.questions)
            } else {
                VStack(alignment: .leading, spacing: 8) {
                    Text(error ?? "No quiz available.").foregroundStyle(error == "Quiz cancelled." ? Ink.ink2 : Ink.due).textSelection(.enabled)
                    Button { load() } label: { Label("Try Again", systemImage: "arrow.clockwise") }
                }
            }
        }
        .task(id: topicId) { load() }
        .onDisappear {
            seq += 1
            Task { try? await Engine.run("cancelAi", ["kind": "quiz", "topicId": topicId]) }
        }
    }

    private func load(_ regenerate: Bool = false) {
        seq += 1
        let n = seq
        loading = true; error = nil; checked = false; answers = [:]; marks = [:]; grades = [:]
        Task {
            do {
                let q: Quiz = try await Engine.call("quiz", topicId, regenerate)
                if seq == n { quiz = q }
            } catch {
                if seq == n { self.error = (error as? EngineError)?.isCancelled == true ? "Quiz cancelled." : error.localizedDescription }
            }
            if seq == n { loading = false }
        }
    }

    private func cancel(_ kind: String) {
        Task { try? await Engine.run("cancelAi", ["kind": kind, "topicId": topicId]) }
    }

    private func questions(_ qs: [QuizQuestion]) -> some View {
        let shortIdx = qs.indices.filter { qs[$0].type == "short" }
        let correct = qs.indices.reduce(0) { a, i in
            let q = qs[i]
            if q.type == "mc" { return a + (answers[i].flatMap(Int.init).map { JSONValue.number(Double($0)) == q.answer } == true ? 1 : 0) }
            return a + ((grades[i]?.correct ?? marks[i]) == true ? 1 : 0)
        }
        let allMarked = shortIdx.allSatisfy { marks[$0] != nil || grades[$0] != nil }
        let score = qs.isEmpty ? 0 : Double(correct) / Double(qs.count)

        return VStack(alignment: .leading, spacing: 18) {
            ForEach(Array(qs.enumerated()), id: \.offset) { i, q in
                VStack(alignment: .leading, spacing: 8) {
                    Text("\(i + 1). \(q.q)").font(Typeset.serif(14, weight: .medium)).fixedSize(horizontal: false, vertical: true)
                    if q.type == "mc" {
                        Picker("", selection: Binding(get: { answers[i] ?? "" }, set: { answers[i] = $0 })) {
                            ForEach(Array((q.options ?? []).enumerated()), id: \.offset) { k, o in
                                Text(o).foregroundStyle(checked ? (q.answer == .number(Double(k)) ? Ink.ok : answers[i] == String(k) ? Ink.due : Ink.ink) : Ink.ink).tag(String(k))
                            }
                        }
                        .pickerStyle(.radioGroup)
                        .labelsHidden()
                        .disabled(checked)
                        if checked, let why = q.why { Text(why).font(Typeset.small).foregroundStyle(Ink.ink2) }
                    } else {
                        TextField("Your answer", text: Binding(get: { answers[i] ?? "" }, set: { answers[i] = $0 }), axis: .vertical)
                            .lineLimit(2...5).textFieldStyle(.roundedBorder).disabled(checked)
                        if checked { shortMark(i, q) }
                    }
                }
            }
            HStack(spacing: 10) {
                if !checked { Button("Check Answers") { checked = true }.buttonStyle(.primary) }
                if checked && !shortIdx.isEmpty && !allMarked {
                    Button { Task { await gradeAi(qs, shortIdx) } } label: {
                        if grading { ProgressView().controlSize(.small) } else { Label("Grade Short Answers with AI", systemImage: "sparkles") }
                    }
                    .disabled(grading)
                }
                if grading { Button("Cancel") { cancel("grade") }.buttonStyle(.borderless) }
                if checked && allMarked {
                    Text("Score \(correct)/\(qs.count) (\(Int((score * 100).rounded()))%)").monospacedDigit()
                    Button(scoreLabel) { onScore(score) }.buttonStyle(.primary)
                }
                Spacer()
                Button { load(true) } label: { Label("New Questions", systemImage: "arrow.clockwise") }.buttonStyle(.borderless)
            }
        }
    }

    @ViewBuilder private func shortMark(_ i: Int, _ q: QuizQuestion) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            (Text("Answer key: ").foregroundStyle(Ink.ink2) + Text(q.answer.text)).font(Typeset.small)
            if let g = grades[i] {
                Text("\(g.correct ? "Graded correct" : "Graded not correct"): \(g.feedback)").font(Typeset.small).foregroundStyle(g.correct ? Ink.ink : Ink.due)
            } else {
                HStack(spacing: 8) {
                    Toggle("I got it", isOn: Binding(get: { marks[i] == true }, set: { if $0 { marks[i] = true } }))
                    Toggle("Not quite", isOn: Binding(get: { marks[i] == false }, set: { if $0 { marks[i] = false } }))
                }
                .toggleStyle(.button)
                .controlSize(.small)
            }
        }
    }

    private func gradeAi(_ qs: [QuizQuestion], _ idx: [Int]) async {
        grading = true
        defer { grading = false }
        do {
            let items = idx.map { ["q": qs[$0].q, "key": qs[$0].answer.text, "answer": answers[$0] ?? ""] }
            let res: [Grade] = try await Engine.call("grade", topicId, items)
            for (k, i) in idx.enumerated() where k < res.count { grades[i] = res[k] }
        } catch {
            if (error as? EngineError)?.isCancelled != true { self.error = error.localizedDescription }
        }
    }
}
