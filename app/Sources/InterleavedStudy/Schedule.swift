// What the app works out from the data for display: mastery and status (the same formulas as engine/src/sched.rs),
// streaks with rest days, day keys, and the way times and durations are written.
import Foundation

let DAY: Double = 86_400_000

enum Status: String, CaseIterable, Identifiable {
    case new, progress, review, solid
    var id: String { rawValue }
    var label: String {
        switch self {
        case .new: "New"
        case .progress: "In progress"
        case .review: "Needs review"
        case .solid: "Solid"
        }
    }
}

extension Topic {
    func recall(_ now: Double) -> Double {
        guard let last, let stab, stab > 0 else { return 0 }
        return pow(0.9, max(0, (now - last) / DAY) / stab)
    }

    func mastery(_ now: Double) -> Int {
        if let override { return Int(override.rounded()) }
        guard last != nil, let stab, stab > 0 else { return 0 }
        let c = solidMark == true ? 1 : 0.6 + 0.4 * min(1, Double(reviews.count) / 3)
        return Int((100 * recall(now) * (evid ?? 0) * c).rounded())
    }

    func status(_ now: Double) -> Status {
        guard let due else { return .new }
        if mastery(now) >= 80 { return .solid }
        return due <= now ? .review : .progress
    }

    /// Why the topic is due, in plain words.
    func why(_ now: Double) -> String {
        var parts: [String] = []
        if let due {
            let d = (due - now) / DAY
            parts.append(d <= 0 ? "Overdue by \(fixed1(abs(d))) days." : "Due in \(fixed1(d)) days.")
            if let lr = reviews.last?.rating { parts.append("Last rated \(lr.rawValue); interval \(fixed1(stab ?? 0)) days.") }
            else if let stab { parts.append("Interval \(fixed1(stab)) days (set manually).") }
        } else {
            parts.append("New, not studied yet.")
        }
        if let override { parts.append("Score set manually to \(Int(override.rounded())).") }
        return parts.joined(separator: " ")
    }
}

func fixed1(_ v: Double) -> String { String(format: "%.1f", v) }

// MARK: - Days

enum Days {
    private static let fmt: DateFormatter = {
        let f = DateFormatter()
        f.calendar = Calendar(identifier: .gregorian)
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd"
        return f
    }()
    static func key(_ d: Date = Date()) -> String { fmt.string(from: d) }
    static func date(_ k: String) -> Date { fmt.date(from: k) ?? Date() }
    static func add(_ k: String, _ n: Int) -> String { key(Calendar.current.date(byAdding: .day, value: n, to: date(k)) ?? date(k)) }
    static func between(_ a: String, _ b: String) -> Int {
        Calendar.current.dateComponents([.day], from: date(a), to: date(b)).day ?? 0
    }
    /// Days from today to `k`: "today", "tomorrow", "in N days", "N days ago".
    static func countdown(_ k: String) -> String {
        let d = between(key(), k)
        if d == 0 { return "today" }
        if d == 1 { return "tomorrow" }
        return d < 0 ? "\(-d) days ago" : "in \(d) days"
    }
}

// MARK: - Streak

struct Streak {
    var current = 0 // calendar days in the streak (studied days plus allowed rest days)
    var restUsed = 0
    var best = 0

    /// A limited number of rest days per rolling 7 days keeps the streak alive; today not studied yet breaks nothing.
    init(_ studied: Set<String>, today: String = Days.key(), restPerWeek: Int) {
        let (cur, rests) = Self.walk(studied, from: studied.contains(today) ? today : Days.add(today, -1), restPerWeek: restPerWeek)
        current = cur
        restUsed = rests
        best = max(cur, studied.map { Self.walk(studied, from: $0, restPerWeek: restPerWeek).0 }.max() ?? 0)
    }

    private static func walk(_ days: Set<String>, from start: String, restPerWeek: Int) -> (Int, Int) {
        var cursor = start, current = 0, studied = 0
        var rests: [String] = [], pending: [String] = []
        for _ in 0..<3660 {
            if days.contains(cursor) {
                current += 1 + pending.count
                rests += pending
                pending = []
                studied += 1
            } else {
                let window = (rests + pending).filter { abs(Days.between($0, cursor)) < 7 }
                if window.count < restPerWeek && studied > 0 { pending.append(cursor) } else { break }
            }
            cursor = Days.add(cursor, -1)
        }
        return (current, rests.count)
    }
}

// MARK: - Writing numbers and times

func plural(_ n: Int, _ word: String, _ words: String? = nil) -> String { "\(n) \(n == 1 ? word : words ?? word + "s")" }

func listNames(_ names: [String]) -> String {
    guard names.count > 1 else { return names.joined() }
    return names.dropLast().joined(separator: ", ") + " and " + names.last!
}

func fmtMin(_ m: Double) -> String {
    if m <= 0 { return "0m" }
    if m < 1 { return "<1m" }
    if m < 60 { return "\(Int(m.rounded()))m" }
    let h = Int(m / 60), r = Int(m.truncatingRemainder(dividingBy: 60).rounded())
    return r > 0 ? "\(h)h \(r)m" : "\(h)h"
}

func fmtClock(_ ms: Double) -> String {
    let s = max(0, Int((ms / 1000).rounded(.up)))
    let h = s / 3600, m = (s % 3600) / 60, sec = s % 60
    return h > 0 ? String(format: "%d:%02d:%02d", h, m, sec) : String(format: "%d:%02d", m, sec)
}

func fmtRel(_ t: Double?, _ now: Double) -> String {
    guard let t else { return "" }
    let d = (t - now) / DAY, a = abs(d)
    if a < 1.0 / 24 { return d < 0 ? "just now" : "now" }
    if a < 1 { let h = Int((a * 24).rounded()); return d < 0 ? "\(h)h ago" : "in \(h)h" }
    if a < 60 { let n = Int(a.rounded()); return d < 0 ? "\(n)d ago" : "in \(n)d" }
    return date(t).formatted(.dateTime.month(.abbreviated).day())
}

func date(_ ms: Double) -> Date { Date(timeIntervalSince1970: ms / 1000) }
var nowMs: Double { Date().timeIntervalSince1970 * 1000 }

extension Date {
    var ms: Double { timeIntervalSince1970 * 1000 }
}

// MARK: - Providers and models (as offered in Settings)

struct ProviderInfo: Identifiable {
    let id: String, label: String, keyName: String, keyUrl: String, keyPlaceholder: String
    var shortLabel: String { label.replacingOccurrences(of: #" \(.*\)$"#, with: "", options: .regularExpression) }
}

enum Providers {
    static let all: [ProviderInfo] = [
        .init(id: "gemini", label: "Google Gemini (free tier available)", keyName: "Google AI Studio API key", keyUrl: "https://aistudio.google.com/apikey", keyPlaceholder: "AIza…"),
        .init(id: "anthropic", label: "Anthropic Claude", keyName: "Claude API key", keyUrl: "https://platform.claude.com/settings/keys", keyPlaceholder: "sk-ant-…"),
        .init(id: "openai", label: "OpenAI (ChatGPT)", keyName: "OpenAI API key", keyUrl: "https://platform.openai.com/api-keys", keyPlaceholder: "sk-…"),
        .init(id: "routine", label: "Claude scheduled task (uses your Claude plan)", keyName: "scheduled task token", keyUrl: "https://claude.ai/code/routines", keyPlaceholder: "sk-ant-oat01-…"),
    ]
    static func info(_ id: String) -> ProviderInfo { all.first { $0.id == id } ?? all[0] }

    /// Preset model IDs per provider; the first is the default when switching to it.
    static let presets: [String: [String]] = [
        "gemini": ["gemini-3.5-flash-lite", "gemma-4-31b-it", "gemma-4-26b-a4b-it", "gemini-3.8-flash", "gemini-flash-latest"],
        "anthropic": ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5", "claude-fable-5-1"],
        "openai": ["gpt-6-luna", "gpt-6.1-sol", "gpt-6-astra", "gpt-5.6-terra", "gpt-5.6-luna"],
        "routine": ["claude-scheduled-task"],
    ]

    static let defaultBaseUrls = ["gemini": "https://generativelanguage.googleapis.com", "anthropic": "https://api.anthropic.com", "openai": "https://api.openai.com", "routine": ""]

    static let thinkLabel = ["gemini": "Minimal thinking", "anthropic": "Low effort", "openai": "Low reasoning effort", "routine": "Low effort"]

    static let sounds = ["Glass", "Ping", "Basso", "Blow", "Bottle", "Frog", "Funk", "Hero", "Morse", "Pop", "Purr", "Sosumi", "Submarine", "Tink"]

    private static let allLevels = ["minimal", "low", "medium", "high"], noMinimal = ["low", "medium", "high"]
    /// The thinking levels each Gemini model accepts, fastest first, and the level it uses when none is sent
    /// (from the Gemini API thinking docs, checked 2026-11-02).
    private static let gemini: [String: (levels: [String], dflt: String?)] = [
        "gemini-3.5-flash-lite": (allLevels, "minimal"), "gemini-3.8-flash": (noMinimal, "medium"), "gemini-flash-latest": (noMinimal, nil),
        "gemini-3.6-flash": (allLevels, "medium"), "gemini-4-argon": (noMinimal, "medium"), "gemini-3.1-pro-preview": (noMinimal, "high"),
        "gemini-3.1-flash-lite-image": (["minimal", "high"], "minimal"), "gemini-3-flash-preview": (allLevels, "high"),
        "gemini-3-pro-preview": (["low", "high"], "high"), "gemini-2.5-pro": (noMinimal, nil), "gemini-2.5-flash": (noMinimal, nil),
        "gemini-2.5-flash-lite": (noMinimal, nil),
    ]

    /// What a Gemini model accepts; Gemma 4 has only an off/on switch, and an unknown model is offered every level.
    static func geminiThinking(_ model: String) -> (levels: [String], dflt: String?, known: Bool) {
        if let g = gemini[model] { return (g.levels, g.dflt, true) }
        if model.lowercased().hasPrefix("gemma-4") { return (["minimal", "high"], nil, true) }
        return (allLevels, nil, false)
    }

    /// The level sent to a Gemini model: the one chosen for it while the model accepts it, else its fastest.
    static func geminiLevel(_ ai: Settings.AI) -> String {
        let levels = geminiThinking(ai.model).levels
        if let chosen = ai.thinking[ai.model], levels.contains(chosen) { return chosen }
        return levels[0]
    }
}
