// The engine's data as Swift values. Field names follow the engine's JSON (engine/src/types.rs).
import Foundation

enum Rating: String, Codable, CaseIterable, Identifiable {
    case lost, shaky, okay, solid
    var id: String { rawValue }
    var label: String { rawValue.capitalized }
    var help: String {
        switch self {
        case .lost: "Could not recall the main ideas"
        case .shaky: "Partly, with gaps"
        case .okay: "Mostly, with some effort"
        case .solid: "Clearly and confidently"
        }
    }
}

struct Review: Codable, Hashable {
    var at: Double
    var rating: Rating
    var selfRating: Rating?
    var minutes: Double
    var quiz: Double?
    var note: String?
    enum CodingKeys: String, CodingKey { case at, rating, selfRating = "self", minutes, quiz, note }
}

struct Course: Codable, Identifiable, Hashable {
    var id: String
    var name: String
    var color: Int
    var createdAt: Double
}

struct Doc: Codable, Identifiable, Hashable {
    var id: String
    var courseId: String
    var title: String
    var fileName: String
    var ext: String
    var words: Int
    var windows: Int
    var winsDone: Int
    var status: String // awaiting | queued | organized | needs | error
    var error: String?
    var importNote: String?
    var fileHash: String?
    var fileBytes: Double?
    var pages: Int?
    var needsReading: Bool?
    var readByAi: Bool?

    /// The lecture deck as the student named the file (the lesson title for hand-made lessons).
    var deckName: String {
        if fileName.isEmpty || ext == "manual" { return title }
        return (fileName as NSString).deletingPathExtension
    }
}

struct Topic: Codable, Identifiable, Hashable {
    var id: String
    var docId: String
    var courseId: String
    var unit: String
    var name: String
    var ord: Double
    var start: Double?
    var end: Double?
    var words: Int
    var minutes: Double
    var summary: String
    var terms: [String]
    var prereqs: [String]
    var difficulty: Double?
    var provisional: Bool
    var manual: Bool?
    var notes: String
    // Schedule
    var stab: Double?
    var due: Double?
    var last: Double?
    var evid: Double?
    var reviews: [Review]
    var spent: Double
    var override: Double?
    var solidMark: Bool?

    func isDue(_ now: Double) -> Bool { (due ?? .infinity) <= now }
}

struct Deadline: Codable, Identifiable, Hashable {
    var id: String
    var title: String
    var date: String // YYYY-MM-DD
    var courseId: String?
    var docId: String?
    var topicIds: [String]?
    var notes: String?
    var key: String
}

struct DayGoal: Codable, Hashable {
    var kind: String // minutes | topics
    var value: Double
}

struct Daily: Codable, Identifiable, Hashable {
    var id: String
    var minutes: Double
    var topics: Double
    var goal: DayGoal?
}

struct MasterySnapshot: Codable, Identifiable, Hashable {
    var id: String
    var avg: Double
}

struct DataSnapshot: Codable {
    var courses: [Course]
    var docs: [Doc]
    var topics: [Topic]
    var deadlines: [Deadline]
    var daily: [Daily]
    var snapshots: [MasterySnapshot]
}

enum Pool: Codable, Hashable {
    case topic([String]), sandbox([String]), unit(docId: String, unit: String), lesson(docId: String), course([String]), global

    private enum K: String, CodingKey { case kind, ids, docId, unit, courseIds }
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: K.self)
        switch try c.decode(String.self, forKey: .kind) {
        case "topic": self = .topic(try c.decode([String].self, forKey: .ids))
        case "sandbox": self = .sandbox(try c.decode([String].self, forKey: .ids))
        case "unit": self = .unit(docId: try c.decode(String.self, forKey: .docId), unit: try c.decode(String.self, forKey: .unit))
        case "lesson": self = .lesson(docId: try c.decode(String.self, forKey: .docId))
        case "course": self = .course(try c.decode([String].self, forKey: .courseIds))
        default: self = .global
        }
    }
    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: K.self)
        switch self {
        case .topic(let ids): try c.encode("topic", forKey: .kind); try c.encode(ids, forKey: .ids)
        case .sandbox(let ids): try c.encode("sandbox", forKey: .kind); try c.encode(ids, forKey: .ids)
        case .unit(let d, let u): try c.encode("unit", forKey: .kind); try c.encode(d, forKey: .docId); try c.encode(u, forKey: .unit)
        case .lesson(let d): try c.encode("lesson", forKey: .kind); try c.encode(d, forKey: .docId)
        case .course(let ids): try c.encode("course", forKey: .kind); try c.encode(ids, forKey: .courseIds)
        case .global: try c.encode("global", forKey: .kind)
        }
    }
    var isSandbox: Bool { if case .sandbox = self { true } else { false } }
}

struct Suggestion: Codable, Hashable, Identifiable {
    var topicId: String
    var name: String
    var score: Double
    var reason: String
    var id: String { topicId }
}

struct RibbonBlock: Codable, Hashable {
    var topicId: String
    var name: String
    var courseId: String
    var state: String // rated | skipped | current
    var rating: Rating?
    var minutes: Double
}

struct PendingReview: Codable, Hashable {
    var minutes: Double
}

struct SessionState: Codable, Equatable {
    var mode: String // timed | pomodoro
    var pool: Pool
    var poolLabel: String
    var sandbox: Bool
    var phase: String // work | break | rate | next | done
    var breakKind: String?
    var topicId: String?
    var topicName: String
    var courseId: String?
    var endAt: Double
    var pausedAt: Double?
    var blockIndex: Int
    var blocksForTopic: Int
    var blockPlan: [Double]
    var blocks: [RibbonBlock]
    var proposal: Suggestion?
    var sandboxReviews: [PendingReview]
    var message: String?

    var timing: Bool { phase == "work" || phase == "break" }
    var paused: Bool { pausedAt != nil }
    func left(_ now: Date) -> Double { max(0, endAt - (pausedAt ?? now.timeIntervalSince1970 * 1000)) }
}

struct RunInfo: Codable, Equatable {
    var requests: Int
    var firedAt: Double
    var url: String?
    var state: String // waiting | sending | holding
    var nextFireAt: Double?
}

struct RateLimitInfo: Codable, Equatable {
    var resetAt: Double?
    var message: String
    var provider: String
    var model: String
}

struct FailedJob: Codable, Equatable, Identifiable {
    var id: String
    var label: String
    var error: String
}

struct QueueStatus: Codable, Equatable {
    var userPaused: Bool
    var pausedUntil: Double?
    var pauseReason: String?
    var running: String?
    var runningId: String?
    var queued: Int
    var failed: [FailedJob]
    var docsTotal: Int
    var docsDone: Int
    var chunksTotal: Int
    var chunksDone: Int
    var rateLimit: RateLimitInfo?
    var noKey: Bool
    var run: RunInfo?

    var busy: Bool { running != nil || queued > 0 }
}

struct ImportChange: Codable, Hashable {
    var path: String
    var docId: String
    var title: String
    var withHistory: Int
}

struct PathError: Codable, Hashable {
    var path: String
    var error: String
}

struct ImportResult: Codable {
    var added: Int
    var unchanged: Int
    var replaced: Int
    var needsConfirm: [ImportChange]
    var errors: [PathError]
    var notes: [String]
    var docIds: [String]
}

struct Estimate: Codable {
    var docs: Int
    var requests: Int
    var cached: Int
    var tokens: Double
}

struct ModelCaps: Codable {
    var api: String
    var system: Bool?
    var json: Bool?
    var thinkMinimal: Bool?
    var thinkLevel: String?
    var fallback: Bool?
    var schema: Bool?
    var testedAt: Double?
}

struct TestResult: Codable {
    var ok: Bool
    var model: String
    var api: String
    var latencyMs: Double
    var reply: String?
    var error: String?
    var notes: [String]
}

struct Settings: Codable, Equatable {
    struct Routine: Codable, Equatable {
        var fireUrl: String
        var repo: String
        var batchMax: Double
        var gapMinutes: Double
        var timeoutMinutes: Double
        var windowTokens: Double
    }
    struct AI: Codable, Equatable {
        var provider: String
        var model: String
        var lastModels: [String: String]
        var endpoint: String
        var baseUrls: [String: String]
        var includeModelInCacheKey: Bool
        var windowTokens: Double
        var maxOutputTokens: Double
        var minimalThinking: Bool
        var thinking: [String: String]
        var routine: Routine
    }
    struct Study: Codable, Equatable { var minutes: Double; var useSuggested: Bool; var pace: Double }
    struct Pomo: Codable, Equatable {
        var work: Double, brk: Double, long: Double, cycles: Double
        var perTopic: String // one | n | minutes
        var n: Double, minutes: Double
        var autoStartNext: Bool
    }
    struct Notify: Codable, Equatable { var sound: String; var bounce: Bool; var badge: Bool }
    struct Goal: Codable, Equatable { var kind: String; var value: Double; var restDaysPerWeek: Double }
    var ai: AI
    var study: Study
    var pomo: Pomo
    var notify: Notify
    var goal: Goal
    var theme: String // system | light | dark
}

struct Bootstrap: Codable {
    var settings: Settings
    var hasKey: Bool
    var keys: [String: Bool]
    var dataPath: String
    var modelCaps: [String: ModelCaps]
}

struct QuizQuestion: Codable, Hashable {
    var type: String // mc | short
    var q: String
    var options: [String]?
    var answer: JSONValue
    var why: String?
}

struct Quiz: Codable {
    var questions: [QuizQuestion]
}

struct Grade: Codable, Hashable {
    var correct: Bool
    var feedback: String
}

struct TopicText: Codable {
    var paras: [String]
    var excerpt: String?
}

struct FileSupport: Codable, Hashable {
    var ext: String
    var kind: String // file | text | extracted | unsupported
    var label: String
    var detail: String
    var maxBytes: Double?
    var maxPages: Int?
}

struct FileSupportTable: Codable, Equatable {
    var provider: String
    var model: String
    var types: [FileSupport]
}

struct SheetData: Codable {
    struct Sheet: Codable { var name: String; var rows: [[JSONValue]] }
    var path: String
    var sheets: [Sheet]
}

struct ExtractedDeadline: Codable, Hashable {
    var title: String
    var date: String
    var courseName: String?
    var notes: String
}

struct DeadlineExtract: Codable {
    var path: String
    var converter: String
    var converterNote: String?
    var items: [ExtractedDeadline]
    var dropped: Int
    var chunks: Int
}

struct RoutineInfo: Codable {
    var prompt: String
    var github: String?
}

/// Any JSON value, for spreadsheet cells and quiz answers.
enum JSONValue: Codable, Hashable {
    case null, bool(Bool), number(Double), string(String), other

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else { self = .other }
    }
    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null, .other: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n): try c.encode(n)
        case .string(let s): try c.encode(s)
        }
    }
    /// The value as a spreadsheet shows it.
    var text: String {
        switch self {
        case .null, .other: ""
        case .bool(let b): b ? "TRUE" : "FALSE"
        case .number(let n): n == n.rounded() && abs(n) < 1e15 ? String(Int(n)) : String(n)
        case .string(let s): s
        }
    }
    var isEmpty: Bool { text.trimmingCharacters(in: .whitespaces).isEmpty }
}
