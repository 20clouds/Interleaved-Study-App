// The Settings window (⌘,): the AI provider (an API key in the Keychain, or a Claude scheduled task with its
// mailbox), the model and its test, request sizes, study and Pomodoro timings, notifications, appearance and data.
import SwiftUI

struct SettingsView: View {
    var body: some View {
        TabView {
            AISettings().tabItem { Label("AI", systemImage: "sparkles") }
            StudySettings().tabItem { Label("Study", systemImage: "timer") }
            NotificationSettings().tabItem { Label("Notifications", systemImage: "bell") }
            GeneralSettings().tabItem { Label("General", systemImage: "gearshape") }
        }
        .buttonStyle(ActionButtonStyle(prominent: false, size: .small))
        .frame(width: 640)
        .frame(minHeight: 460, maxHeight: 820)
    }
}

/// A help line under a setting.
private struct Help: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View { Text(text).font(Typeset.small).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
}

/// A text setting that saves on Return or when it loses focus.
private struct TextSetting: View {
    let label: String
    let value: String
    var prompt = ""
    let onSave: (String) -> Void
    @State private var text = ""
    @FocusState private var focused: Bool
    var body: some View {
        LabeledContent(label) {
            TextField(label, text: $text, prompt: Text(prompt))
                .labelsHidden()
                .multilineTextAlignment(.trailing)
                .field(.small)
                .frame(minWidth: 300)
                .focused($focused)
        }
            .onAppear { text = value }
            .onChange(of: value) { text = value }
            .onSubmit(commit)
            .onChange(of: focused) { if !focused { commit() } }
    }
    private func commit() {
        let t = text.trimmingCharacters(in: .whitespaces)
        if t != value { onSave(t) }
    }
}

/// A number setting in a form row.
private struct NumberSetting: View {
    let label: String
    let value: Double
    var range: ClosedRange<Double> = 0...1_000_000
    var suffix = ""
    let onSave: (Double) -> Void
    var body: some View {
        LabeledContent(label) {
            HStack(spacing: 6) {
                NumberField(value: value, range: range, width: 70, onSave: onSave)
                if !suffix.isEmpty { Text(suffix).foregroundStyle(.secondary) }
            }
        }
    }
}

/// A secret typed once and saved to the Keychain; afterwards only "saved" is shown.
private struct SecretSetting: View {
    @Environment(AppModel.self) private var app
    let name: String
    let label: String
    let placeholder: String
    let saved: Bool
    var onChange: () async -> Void = {}
    @State private var value = ""

    var body: some View {
        LabeledContent {
            HStack {
                SecureField(label, text: $value, prompt: Text(saved ? "Paste a new one to replace it" : placeholder)).labelsHidden().field(.small)
                Button("Save") {
                    let v = value
                    Task {
                        let ok: Void? = await app.act("Saved") {
                            let r: [String: JSONValue] = try await Engine.call("setKey", v, name)
                            if r["ok"] != .bool(true) { throw EngineError(message: r["error"]?.text ?? "The key was not saved.") }
                        }
                        if ok != nil { value = ""; await app.refreshBoot(); await onChange() }
                    }
                }
                .disabled(value.trimmingCharacters(in: .whitespaces).isEmpty)
                if saved {
                    Button("Remove", role: .destructive) {
                        Task { await app.act("Removed") { try await Engine.run("clearKey", name) }; await app.refreshBoot(); await onChange() }
                    }
                }
            }
        } label: {
            HStack {
                Text(label)
                if saved { Text("Saved in Keychain").font(Typeset.caption).foregroundStyle(Ink.ok) }
            }
        }
    }
}

// MARK: - AI

private struct AISettings: View {
    @Environment(AppModel.self) private var app
    @State private var model = ""
    @State private var testing = false
    @State private var test: TestResult?

    var body: some View {
        if let s = app.settings, let boot = app.boot {
            let ai = s.ai
            let routine = ai.provider == "routine"
            let info = Providers.info(ai.provider)
            Form {
                Section {
                    Picker("Provider", selection: Binding(get: { ai.provider }, set: { switchProvider($0, ai) })) {
                        ForEach(Providers.all) { p in Text(p.label + (boot.keys[p.id] == true ? " (saved)" : "")).tag(p.id) }
                    }
                    Help("Each provider keeps its own key. Switching keeps the other keys saved and changes the model list.")
                    if !routine {
                        SecretSetting(name: ai.provider, label: info.keyName, placeholder: "Paste your API key (\(info.keyPlaceholder))", saved: boot.hasKey)
                        HStack(spacing: 4) {
                            Help(boot.hasKey ? "A key is saved in the macOS Keychain." : "No key saved yet.")
                            Link("Get a key", destination: URL(string: info.keyUrl)!).font(Typeset.small)
                        }
                        switch ai.provider {
                        case "gemini": Help("Gemini has a free API tier: create a key in Google AI Studio with just a Google account, no credit card needed. The free tier has lower rate limits, and Gemma models on this API are free tier only. Privacy: Google says paid-tier content is not used to improve its products, which implies free-tier content may be.")
                        case "anthropic": Help("Claude API usage is billed per token through the Claude Console, separately from a Claude.ai subscription. Anthropic does not train on API content by default.")
                        default: Help("OpenAI API usage is billed per token through the OpenAI Platform, separately from a ChatGPT subscription. Requests are sent with storage turned off, and OpenAI does not train on API content by default.")
                        }
                    }
                }
                if routine {
                    RoutineSetup(testing: testing, test: test) { runTest(ai) }
                } else {
                    modelSection(ai, boot)
                }
                filesSection(routine)
                Section("Requests") {
                    Help(routine
                         ? "Lessons are cut into windows; each window is one request, and requests travel together in runs. If the task stops answering or hits its limit, everything pauses and you get a notification; Continue picks up exactly where it stopped, without redoing finished work."
                         : "The app sends one request at a time, as fast as \(info.shortLabel) allows. If the provider’s rate limit is reached, everything pauses and you get a notification; Continue picks up exactly where it stopped, without redoing finished work.")
                    if routine {
                        NumberSetting(label: "Window size", value: ai.routine.windowTokens, range: 2000...20000, suffix: "input tokens") { app.saveSettings(["ai": ["routine": ["windowTokens": $0]]]) }
                    } else {
                        NumberSetting(label: "Window size", value: ai.windowTokens, range: 1000...8000, suffix: "input tokens") { app.saveSettings(["ai": ["windowTokens": $0]]) }
                    }
                    NumberSetting(label: "Max output per request", value: ai.maxOutputTokens, range: 300...8000, suffix: "tokens") { app.saveSettings(["ai": ["maxOutputTokens": $0]]) }
                    Toggle("Keep separate cached results per model", isOn: Binding(get: { ai.includeModelInCacheKey }, set: { app.saveSettings(["ai": ["includeModelInCacheKey": $0]]) }))
                    Help("Off: switching models reuses earlier results and saves tokens.")
                }
            }
            .formStyle(.grouped)
            .onAppear { model = ai.model }
            .onChange(of: ai.model) { model = ai.model }
        }
    }

    private func switchProvider(_ p: String, _ ai: Settings.AI) {
        guard p != ai.provider else { return }
        test = nil
        app.saveSettings(["ai": ["provider": p, "model": ai.lastModels[p] ?? Providers.presets[p]?.first ?? "", "lastModels": [ai.provider: ai.model]]])
        Task { try? await Task.sleep(for: .milliseconds(200)); await app.refreshBoot() }
    }

    private func chooseModel(_ m: String, _ ai: Settings.AI) {
        let t = m.trimmingCharacters(in: .whitespaces)
        model = t
        guard !t.isEmpty, t != ai.model else { return }
        test = nil
        app.saveSettings(["ai": ["model": t]])
    }

    private func runTest(_ ai: Settings.AI) {
        testing = true
        test = nil
        let m = model.trimmingCharacters(in: .whitespaces)
        Task {
            defer { testing = false }
            do {
                if ai.provider != "routine" && m != ai.model { let _: Settings = try await Engine.call("setSettings", ["ai": ["model": m]]) }
                test = try await Engine.call("testModel", m)
                await app.refreshBoot()
            } catch { app.show(error) }
        }
    }

    @ViewBuilder private func modelSection(_ ai: Settings.AI, _ boot: Bootstrap) -> some View {
        let presets = Providers.presets[ai.provider] ?? []
        let think = Providers.thinkLabel[ai.provider] ?? ""
        Section("Model") {
            Picker("Preset", selection: Binding(get: { presets.contains(model) ? model : "" }, set: { if !$0.isEmpty { chooseModel($0, ai) } })) {
                Text("Other model ID…").tag("")
                ForEach(presets, id: \.self) { Text($0).tag($0) }
            }
            LabeledContent("Model ID") {
                HStack {
                    TextField("Model ID", text: $model).labelsHidden().field(.small).onSubmit { chooseModel(model, ai) }
                    Button { runTest(ai) } label: { if testing { ProgressView().controlSize(.small) } else { Text("Test Model") } }
                        .disabled(testing || model.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            if let test { TestOutcome(test: test) }
            if let mc = boot.modelCaps[ai.model] {
                let yn = { (v: Bool?) in v == true ? "yes" : v == false ? "no" : "not tested" }
                Help("Recorded for \(ai.model): endpoint \(mc.api), system instruction \(yn(mc.system)), JSON mode \(yn(mc.json)), \(think.lowercased()) \(yn(mc.thinkMinimal))"
                     + (ai.provider == "gemini" && mc.thinkLevel == "low" ? " (level low used)" : "")
                     + (ai.provider == "gemini" ? ", structured output \(yn(mc.schema))" : "")
                     + (ai.provider == "anthropic" ? ", refusal fallback \(yn(mc.fallback))" : "")
                     + (mc.testedAt.map { ", tested \(date($0).formatted(date: .abbreviated, time: .shortened))" } ?? "") + ".")
            }
            if ai.provider == "gemini" {
                Picker("Endpoint", selection: Binding(get: { ai.endpoint }, set: { app.saveSettings(["ai": ["endpoint": $0]]) })) {
                    Text("Automatic").tag("auto")
                    Text("generateContent").tag("generateContent")
                    Text("Interactions API").tag("interactions")
                }
                let gt = Providers.geminiThinking(ai.model)
                Picker("Thinking", selection: Binding(get: { Providers.geminiLevel(ai) }, set: { app.saveSettings(["ai": ["thinking": [ai.model: $0]]]) })) {
                    ForEach(gt.levels, id: \.self) { l in Text(levelLabel(ai.model, l, gt.levels, gt.dflt)).tag(l) }
                }
                Help((ai.model.lowercased().hasPrefix("gemma-4") ? "Gemma can only turn thinking off or on. Off answers sooner. " : "More thinking gives the model longer to reason before it answers, so each request is slower and uses more output tokens. ")
                     + (gt.known ? "Only the levels \(ai.model) accepts are listed, saved for this model." : "This model isn’t in Google’s list, so every level is offered; one it rejects is skipped for that request."))
            } else {
                Toggle("Ask for \(think.lowercased())", isOn: Binding(get: { ai.minimalThinking }, set: { app.saveSettings(["ai": ["minimalThinking": $0]]) }))
                Help("Fewer output tokens; dropped automatically if the model refuses it.")
            }
            TextSetting(label: "Base URL (advanced)", value: ai.baseUrls[ai.provider] ?? "", prompt: Providers.defaultBaseUrls[ai.provider] ?? "") { v in
                let url = v.hasSuffix("/") ? String(v.dropLast()) : v
                app.saveSettings(["ai": ["baseUrls": [ai.provider: url]]])
            }
            .id(ai.provider)
        }
    }

    private func levelLabel(_ model: String, _ l: String, _ levels: [String], _ dflt: String?) -> String {
        if model.lowercased().hasPrefix("gemma-4") { return l == "minimal" ? "Off" : "On" }
        let name = l.capitalized + (l == levels.first ? ", fastest" : l == levels.last ? ", slowest" : "")
        return l == dflt ? "\(name) (model default)" : name
    }

    @ViewBuilder private func filesSection(_ routine: Bool) -> some View {
        Section("Lesson files") {
            if let files = app.fileSupport {
                Help(files.types.contains { $0.kind == "file" }
                     ? "\(files.provider) gets PDFs\(routine ? "" : ", Word files") and pictures as the files themselves: it reads the text and sees every page, so tables, equations, figures and scans count. Other types go as text."
                     : "Only Gemini and the Claude scheduled task are sent the files themselves. This provider gets the text the Mac extracts.")
                // Picture formats always share one answer, so they share one row.
                let pics: Set = ["png", "jpg", "webp"]
                let rows = files.types.filter { !pics.contains($0.ext) || $0.ext == files.types.first(where: { pics.contains($0.ext) })?.ext }
                ForEach(rows, id: \.ext) { t in
                    let isPic = pics.contains(t.ext)
                    LabeledContent {
                        VStack(alignment: .trailing, spacing: 2) {
                            DeliveryTag(delivery: Delivery(kind: t.kind, label: t.label, detail: t.detail))
                            Text(t.detail + (t.maxBytes.map { " Up to \(Int($0 / 1_048_576)) MB" + (t.maxPages.map { " and \($0) pages" } ?? "") + "." } ?? ""))
                                .font(Typeset.small).foregroundStyle(.secondary).multilineTextAlignment(.trailing)
                        }
                    } label: {
                        Text(isPic ? "Pictures" : FileTypes.name(t.ext)) + Text(isPic ? "  .png .jpg .webp" : "  .\(t.ext)").foregroundStyle(.secondary)
                    }
                }
            }
            if routine { Help("Files go to the mailbox on a branch of their own, deleted once the run is over. GitHub may keep deleted data for a while, and the run's session on claude.ai keeps what it read, so use a mailbox only you can see.") }
        }
    }
}

private struct TestOutcome: View {
    let test: TestResult
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if test.ok {
                Text("Working with ") + Text(test.model).bold() + Text(" via \(test.api) in \(Int(test.latencyMs)) ms.") + Text(test.reply.map { " Reply: \($0)" } ?? "").foregroundStyle(.secondary)
            } else {
                Text(test.error ?? "The test failed.").foregroundStyle(Ink.due).textSelection(.enabled)
            }
            ForEach(test.notes, id: \.self) { Help($0) }
        }
    }
}

/// Setting up the Claude scheduled task. It is created on claude.ai (only there can its API trigger and token be
/// made); the app needs its address, its token, and a private GitHub repository the answers come back through.
private struct RoutineSetup: View {
    @Environment(AppModel.self) private var app
    let testing: Bool
    let test: TestResult?
    let runTest: () -> Void
    @State private var info: RoutineInfo?
    @State private var creating = false

    var body: some View {
        if let s = app.settings, let boot = app.boot {
            let r = s.ai.routine
            let ready = !r.repo.isEmpty && !r.fireUrl.isEmpty && boot.keys["routine"] == true
            Section {
                Help("Runs the AI work as a Claude scheduled task on your Claude plan, so no API key is billed. Requests are sent in batches of up to \(Int(r.batchMax)); each batch is one run and takes a minute or two. Claude allows 30 starts an hour for one task, and the app holds the next run when that is close. There is no daily count of runs: what runs out is your plan’s usage, which these runs share with everything else you do in Claude.")
            }
            Section("1. Make the mailbox") {
                Help("A private GitHub repository carries the requests to Claude and the answers back. Your lesson text passes through it, so it has to be private; the app checks that before sending anything and clears each batch once it is read.")
                if let info {
                    if info.github == "gh" { Help("GitHub access: using your GitHub CLI sign-in.") }
                    else if info.github == nil { Help("GitHub access: none found. Sign in with the GitHub CLI (gh auth login) and reopen Settings, or paste a token with the “repo” permission below.") }
                    if info.github != "gh" { SecretSetting(name: "github", label: "GitHub token", placeholder: "ghp_… or github_pat_…", saved: boot.keys["github"] == true) { await load() } }
                }
                TextSetting(label: "Mailbox repository", value: r.repo, prompt: "owner/name") { app.saveSettings(["ai": ["routine": ["repo": $0]]]) }
                Button { Task { await createRepo() } } label: {
                    if creating { ProgressView().controlSize(.small) } else { Label(r.repo.isEmpty ? "Create a Private Mailbox for Me" : "Create Another", systemImage: "plus") }
                }
                .disabled(creating || info?.github == nil)
            }
            Section("2. Create the scheduled task on claude.ai") {
                Text("Open ") + Text("[claude.ai/code/routines](https://claude.ai/code/routines)") + Text(" and click **New task** (choose **Set up manually** if asked). Paste the prompt below, pick **API** as the trigger, and under **Advanced settings** choose the mailbox repository\(r.repo.isEmpty ? "" : " (\(r.repo))"). When you click **Create** the token is shown once: copy it. The task URL is in the task’s **Edit** dialog, in the API trigger’s panel (**Fire URL**). A smaller model such as Sonnet uses less of your plan.")
                VStack(alignment: .leading, spacing: 8) {
                    Text(info?.prompt ?? "…").font(.system(size: 12, design: .monospaced)).textSelection(.enabled).lineLimit(8)
                    Button { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(info?.prompt ?? "", forType: .string); app.say("Prompt copied") } label: {
                        Label("Copy Prompt", systemImage: "doc.on.doc")
                    }
                    .disabled(info == nil)
                }
            }
            Section("3. Connect it") {
                TextSetting(label: "Task URL", value: r.fireUrl, prompt: "https://api.anthropic.com/v1/claude_code/routines/trig_…/fire") { app.saveSettings(["ai": ["routine": ["fireUrl": $0]]]) }
                SecretSetting(name: "routine", label: "Task token", placeholder: "sk-ant-oat01-…", saved: boot.keys["routine"] == true)
            }
            Section("4. Check the setup") {
                HStack {
                    Button { runTest() } label: { if testing { ProgressView().controlSize(.small) } else { Text("Check Setup") } }.disabled(testing)
                    Help(ready ? "Checks the mailbox and the fields without starting a run." : "Finish the steps above first.")
                }
                if let test {
                    if test.ok { Text("Ready. The first lesson you organize is the real test.") } else { Text(test.error ?? "").foregroundStyle(Ink.due).textSelection(.enabled) }
                    ForEach(test.notes, id: \.self) { Help($0) }
                }
            }
            Section("Runs") {
                NumberSetting(label: "Requests per run", value: r.batchMax, range: 1...20) { app.saveSettings(["ai": ["routine": ["batchMax": $0]]]) }
                NumberSetting(label: "Shortest time between runs", value: r.gapMinutes, range: 0...240, suffix: "min") { app.saveSettings(["ai": ["routine": ["gapMinutes": $0]]]) }
                NumberSetting(label: "Give up waiting after", value: r.timeoutMinutes, range: 5...180, suffix: "min") { app.saveSettings(["ai": ["routine": ["timeoutMinutes": $0]]]) }
                Help("A run that takes longer than this pauses the queue; its answers are still collected when they arrive, and Continue never pays for the same request twice.")
            }
            .task { await load() }
        }
    }

    private func load() async { info = try? await Engine.call("routineInfo") }

    private func createRepo() async {
        creating = true
        defer { creating = false }
        do {
            let r: [String: JSONValue] = try await Engine.call("routineCreateRepo")
            if r["ok"] != .bool(true) { throw EngineError(message: r["error"]?.text ?? "The mailbox was not created.") }
            await app.refreshBoot()
            app.say("Mailbox ready: \(r["repo"]?.text ?? "")")
        } catch { app.show(error) }
    }
}

// MARK: - Study

private struct StudySettings: View {
    @Environment(AppModel.self) private var app
    var body: some View {
        if let s = app.settings {
            Form {
                Section("Study") {
                    NumberSetting(label: "Minutes per topic (timed mode)", value: s.study.minutes, range: 1...240) { app.saveSettings(["study": ["minutes": $0]]) }
                    NumberSetting(label: "Reading pace for suggested times", value: s.study.pace, range: 30...400, suffix: "words/min") { app.saveSettings(["study": ["pace": $0]]) }
                    Toggle("Use each topic’s suggested time in timed mode", isOn: Binding(get: { s.study.useSuggested }, set: { app.saveSettings(["study": ["useSuggested": $0]]) }))
                }
                Section("Pomodoro") {
                    NumberSetting(label: "Work", value: s.pomo.work, range: 1...240, suffix: "min") { app.saveSettings(["pomo": ["work": $0]]) }
                    NumberSetting(label: "Break", value: s.pomo.brk, range: 1...120, suffix: "min") { app.saveSettings(["pomo": ["brk": $0]]) }
                    NumberSetting(label: "Long break", value: s.pomo.long, range: 1...240, suffix: "min") { app.saveSettings(["pomo": ["long": $0]]) }
                    NumberSetting(label: "Work blocks before a long break", value: s.pomo.cycles, range: 1...20) { app.saveSettings(["pomo": ["cycles": $0]]) }
                    Picker("Topic length", selection: Binding(get: { s.pomo.perTopic }, set: { app.saveSettings(["pomo": ["perTopic": $0]]) })) {
                        Text("1 Pomodoro per topic").tag("one")
                        Text("N Pomodoros per topic").tag("n")
                        Text("Custom minutes per topic").tag("minutes")
                    }
                    if s.pomo.perTopic == "n" { NumberSetting(label: "Pomodoros per topic", value: s.pomo.n, range: 1...20) { app.saveSettings(["pomo": ["n": $0]]) } }
                    if s.pomo.perTopic == "minutes" { NumberSetting(label: "Minutes per topic", value: s.pomo.minutes, range: 1...600, suffix: "split into work blocks") { app.saveSettings(["pomo": ["minutes": $0]]) } }
                    Toggle("Start the suggested topic automatically when a break ends", isOn: Binding(get: { s.pomo.autoStartNext }, set: { app.saveSettings(["pomo": ["autoStartNext": $0]]) }))
                }
            }
            .formStyle(.grouped)
        }
    }
}

// MARK: - Notifications

private struct NotificationSettings: View {
    @Environment(AppModel.self) private var app
    var body: some View {
        if let s = app.settings {
            Form {
                Section {
                    Picker("Sound", selection: Binding(get: { s.notify.sound }, set: { app.saveSettings(["notify": ["sound": $0]]); NSSound(named: $0)?.play() })) {
                        ForEach(Providers.sounds, id: \.self) { Text($0).tag($0) }
                    }
                    Toggle("Bounce the Dock icon", isOn: Binding(get: { s.notify.bounce }, set: { app.saveSettings(["notify": ["bounce": $0]]) }))
                    Toggle("Badge the Dock icon", isOn: Binding(get: { s.notify.badge }, set: { app.saveSettings(["notify": ["badge": $0]]) }))
                    Button("Send a Test Notification") { app.perform { try await Engine.run("testNotification") } }
                    Help("If nothing appears, allow notifications for Interleaved Study in System Settings, Notifications. The Dock bounces and badges only while the app is in the background.")
                }
            }
            .formStyle(.grouped)
        }
    }
}

// MARK: - General

private struct GeneralSettings: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        if let s = app.settings, let boot = app.boot {
            Form {
                Section {
                    Picker("Appearance", selection: Binding(get: { s.theme }, set: { app.saveSettings(["theme": $0]) })) {
                        Text("System").tag("system"); Text("Light").tag("light"); Text("Dark").tag("dark")
                    }
                    .pickerStyle(.segmented)
                }
                Section("Data") {
                    Help("Storage: SQLite database on this Mac (WAL, full-text search).")
                    HStack {
                        Button { app.perform { if let p: String = try await Engine.call("exportBackup") as String? { app.say("Backup saved to \(p)") } } } label: { Label("Export Backup…", systemImage: "square.and.arrow.up") }
                        Button { Task { await restore() } } label: { Label("Import Backup…", systemImage: "square.and.arrow.down") }
                        Button { NSWorkspace.shared.open(URL(fileURLWithPath: boot.dataPath)) } label: { Label("Open Data Folder", systemImage: "folder") }
                    }
                    Help("Backups contain everything except keys and tokens. Importing replaces all data and keeps your current keys and scheduled task setup; a safety copy of the current data is written to the data folder first.")
                }
            }
            .formStyle(.grouped)
        }
    }

    private func restore() async {
        do {
            let r: [String: JSONValue]? = try await Engine.call("importBackup")
            guard let r else { return }
            if r["ok"] != .bool(true) { throw EngineError(message: r["error"]?.text ?? "The backup could not be restored.") }
            await app.refreshBoot()
            app.say("Backup restored")
        } catch { app.show(error) }
    }
}
