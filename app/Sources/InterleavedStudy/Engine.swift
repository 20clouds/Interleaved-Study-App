// The bridge to the Rust engine: one `call` per API method (JSON in, JSON out), events from the engine, and the
// engine's questions to the app (confirmation sheets and open/save panels), answered on the main thread.
import AppKit
import CEngine
import UniformTypeIdentifiers

struct EngineError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
    /// The AI request was cancelled (by the person, or because the view that wanted it went away).
    var isCancelled: Bool { message == "Cancelled" }
}

private final class Reply {
    let done: (Bool, Data) -> Void
    init(_ done: @escaping (Bool, Data) -> Void) { self.done = done }
}

enum Engine {
    /// Receives every engine event on the main thread: its name and JSON payload.
    @MainActor static var onEvent: ((String, Data) -> Void)?

    static func start(dataDir: URL) -> Bool {
        dataDir.path.withCString { dir in
            is_start(dir, { name, payload in
                let n = String(cString: name!)
                let d = Data(String(cString: payload!).utf8)
                DispatchQueue.main.async { MainActor.assumeIsolated { Engine.onEvent?(n, d) } }
            }, { request in
                let req = (try? JSONSerialization.jsonObject(with: Data(String(cString: request!).utf8))) as? [String: Any] ?? [:]
                let answer = Host.answer(req)
                guard let data = try? JSONSerialization.data(withJSONObject: answer, options: .fragmentsAllowed) else { return nil }
                return strdup(String(decoding: data, as: UTF8.self))
            })
        }
    }

    static func stop() { is_stop() }

    /// Run one API method. Arguments are JSON values: strings, numbers, booleans, nil, arrays, dictionaries, or
    /// anything Encodable.
    static func raw(_ method: String, _ args: [Any?]) async throws -> Data {
        let argData = try JSONSerialization.data(withJSONObject: args.map(jsonValue), options: .fragmentsAllowed)
        let argString = String(decoding: argData, as: UTF8.self)
        return try await withCheckedThrowingContinuation { cont in
            let box = Unmanaged.passRetained(Reply { ok, data in
                if ok { cont.resume(returning: data) } else {
                    let msg = (try? JSONDecoder().decode(String.self, from: data)) ?? "The engine did not answer."
                    cont.resume(throwing: EngineError(message: msg))
                }
            })
            is_call(method, argString, box.toOpaque()) { ctx, ok, json in
                let reply = Unmanaged<Reply>.fromOpaque(ctx!).takeRetainedValue()
                reply.done(ok, Data(String(cString: json!).utf8))
            }
        }
    }

    static func call<T: Decodable>(_ method: String, _ args: Any?...) async throws -> T {
        let data = try await raw(method, args)
        do { return try JSONDecoder().decode(T.self, from: data) } catch {
            NSLog("Interleaved Study: could not read the answer to %@: %@", method, String(describing: error))
            throw EngineError(message: "The app could not read the engine’s answer to \(method).")
        }
    }

    static func run(_ method: String, _ args: Any?...) async throws {
        _ = try await raw(method, args)
    }

    private static func jsonValue(_ v: Any?) -> Any {
        switch v {
        case nil: return NSNull()
        case let e as Encodable:
            if e is String || e is Bool || e is Double || e is Int { return e }
            guard let d = try? JSONEncoder().encode(AnyEncodable(e)), let o = try? JSONSerialization.jsonObject(with: d, options: .fragmentsAllowed) else { return NSNull() }
            return o
        case let a as [Any?]: return a.map(jsonValue)
        case let o as [String: Any?]: return o.mapValues(jsonValue)
        case let x?: return x
        }
    }
}

private struct AnyEncodable: Encodable {
    let value: Encodable
    init(_ v: Encodable) { value = v }
    func encode(to encoder: Encoder) throws { try value.encode(to: encoder) }
}

/// The engine's questions. Each runs on the main thread while the engine thread that asked waits for the answer.
enum Host {
    static func answer(_ req: [String: Any]) -> Any {
        let sem = DispatchSemaphore(value: 0)
        var result: Any = NSNull()
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                present(req) { result = $0; sem.signal() }
            }
        }
        sem.wait()
        return result
    }

    @MainActor private static func present(_ req: [String: Any], done: @escaping (Any) -> Void) {
        let str = { (k: String) in req[k] as? String ?? "" }
        switch str("op") {
        case "confirm":
            confirm(message: str("message"), detail: str("detail"), ok: str("ok"), danger: req["danger"] as? Bool ?? false) { done($0) }
        case "open":
            let p = NSOpenPanel()
            p.title = str("title")
            p.canChooseFiles = !(req["folders"] as? Bool ?? false)
            p.canChooseDirectories = req["folders"] as? Bool ?? false
            p.allowsMultipleSelection = req["multiple"] as? Bool ?? false
            if let types = req["types"] as? [String] { p.allowedContentTypes = types.compactMap { UTType(filenameExtension: $0) } }
            sheet(p) { ok in done(ok ? p.urls.map(\.path) : []) }
        case "save":
            let p = NSSavePanel()
            p.title = str("title")
            p.nameFieldStringValue = str("name")
            if let types = req["types"] as? [String] { p.allowedContentTypes = types.compactMap { UTType(filenameExtension: $0) } }
            sheet(p) { ok in done(ok ? [p.url?.path].compactMap { $0 } : []) }
        default:
            done(NSNull())
        }
    }

    @MainActor private static func sheet(_ p: NSSavePanel, done: @escaping (Bool) -> Void) {
        // On the sheet open in the window, if any: a window's second sheet waits until its first one closes.
        if let w = NSApp.keyWindow ?? NSApp.mainWindow {
            p.beginSheetModal(for: w.attachedSheet ?? w) { done($0 == .OK) }
        } else {
            done(p.runModal() == .OK)
        }
    }

    /// A sheet on the window. A destructive action is never the Return-key default: Cancel is.
    @MainActor static func confirm(message: String, detail: String = "", ok: String, danger: Bool = false, done: @escaping (Bool) -> Void) {
        let a = NSAlert()
        a.messageText = message
        a.informativeText = detail
        a.alertStyle = danger ? .warning : .informational
        let okButton = a.addButton(withTitle: ok.isEmpty ? "OK" : ok)
        let cancel = a.addButton(withTitle: "Cancel")
        if danger {
            okButton.hasDestructiveAction = true
            okButton.keyEquivalent = ""
            cancel.keyEquivalent = "\r"
        }
        let finish = { (r: NSApplication.ModalResponse) in done(r == .alertFirstButtonReturn) }
        if let w = NSApp.mainWindow ?? NSApp.keyWindow { a.beginSheetModal(for: w, completionHandler: finish) } else { finish(a.runModal()) }
    }

    @MainActor static func confirm(message: String, detail: String = "", ok: String, danger: Bool = false) async -> Bool {
        await withCheckedContinuation { c in confirm(message: message, detail: detail, ok: ok, danger: danger) { c.resume(returning: $0) } }
    }
}
