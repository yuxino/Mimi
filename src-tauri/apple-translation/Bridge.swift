import AppKit
import Foundation
import SwiftUI
import Translation

// No text or native error descriptions enter diagnostics. Callback buffers are
// borrowed only until the callback returns; Rust copies successful text results.
public typealias MimiTranslationCallback = @convention(c) (UInt64, UnsafePointer<UInt8>?, Int) -> Int32

private enum TranslationBridgeError: Int, Error {
    case unavailable = 1, assetsMissing, unsupportedLanguage, invalidInput
    case cancelled, busy, invalidResult, failed
}

private enum TranslationOperation: Sendable {
    case query
    case status(String, String)
    case prepare(String, String, String)
    case translate(String, String, String)

    var isPreparation: Bool {
        if case .prepare = self { return true }
        return false
    }
}

private final class TranslationRequest: @unchecked Sendable {
    let identifier: UInt64
    let operation: TranslationOperation
    private let callback: MimiTranslationCallback
    private let lock = NSRecursiveLock()
    private var cancelled = false
    private var deliveryClosed = false
    private var completed = false
    private var task: Task<Void, Never>?

    init(_ identifier: UInt64, _ operation: TranslationOperation, _ callback: @escaping MimiTranslationCallback) {
        self.identifier = identifier
        self.operation = operation
        self.callback = callback
    }

    func setTask(_ value: Task<Void, Never>) {
        lock.lock()
        if completed || cancelled { lock.unlock(); value.cancel(); return }
        task = value
        lock.unlock()
    }

    func checkCancellation() throws {
        lock.lock()
        let stopped = cancelled
        lock.unlock()
        if stopped || Task.isCancelled { throw TranslationBridgeError.cancelled }
    }

    func send(_ fields: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: fields), data.count <= 262_144 else {
            fail(.invalidResult)
            return
        }
        lock.lock()
        defer { lock.unlock() }
        guard !deliveryClosed else { return }
        deliveryClosed = true
        _ = data.withUnsafeBytes { callback(identifier, $0.bindMemory(to: UInt8.self).baseAddress, $0.count) }
    }

    func fail(_ error: TranslationBridgeError) {
        send(["event": "error", "code": error.rawValue])
    }

    func finish() {
        lock.lock()
        completed = true
        task = nil
        deliveryClosed = true
        lock.unlock()
        TranslationRequests.shared.finish(self)
    }

    func cancel(suppressOutput: Bool) {
        lock.lock()
        cancelled = true
        if suppressOutput { deliveryClosed = true }
        let running = task
        lock.unlock()
        running?.cancel()
        if #available(macOS 26.0, *) {
            Task { @MainActor in TranslationWork.shared.cancel(identifier) }
        }
    }
}

private final class TranslationRequests: @unchecked Sendable {
    static let shared = TranslationRequests()
    private let lock = NSLock()
    private var requests: [UInt64: TranslationRequest] = [:]

    func insert(_ request: TranslationRequest) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard requests.count < 8, requests[request.identifier] == nil,
              !request.operation.isPreparation || !requests.values.contains(where: { $0.operation.isPreparation }) else { return false }
        requests[request.identifier] = request
        return true
    }

    func finish(_ request: TranslationRequest) {
        lock.lock()
        if requests[request.identifier] === request { requests.removeValue(forKey: request.identifier) }
        lock.unlock()
    }

    func cancel(_ identifier: UInt64) {
        lock.lock()
        let request = requests[identifier]
        lock.unlock()
        // Keep capacity reserved until the framework task actually exits. A
        // succession of Rust timeouts cannot create unbounded native work.
        request?.cancel(suppressOutput: true)
    }
}

private protocol InstalledTranslationSession: Sendable {
    var canRequestDownloads: Bool { get }
    func isReady() async -> Bool
    func prepare() async throws
    func cancel()
}

@available(macOS 26.0, *)
private final class TranslationSessionHandle: InstalledTranslationSession, @unchecked Sendable {
    // Translation's SDK class predates Sendable annotations. Each handle owns
    // exactly one operation; the only concurrent access is the framework's
    // explicit cancel operation. No second translation shares this session.
    private let session: TranslationSession
    init(_ session: TranslationSession) { self.session = session }
    var canRequestDownloads: Bool { session.canRequestDownloads }
    func cancel() { session.cancel() }
    func isReady() async -> Bool { await session.isReady }
    func translate(_ text: String) async throws -> TranslationSession.Response {
        try await session.translate(text)
    }
    func prepare() async throws { try await session.prepareTranslation() }
}

@available(macOS 26.0, *)
private func installedTranslationStatus(
    _ availability: LanguageAvailability.Status,
    session: any InstalledTranslationSession,
    checkCancellation: @Sendable () throws -> Void
) async throws -> LanguageAvailability.Status {
    try Task.checkCancellation()
    try checkCancellation()
    guard !session.canRequestDownloads else { throw TranslationBridgeError.failed }
    switch availability {
    case .unsupported: return .unsupported
    case .installed, .supported: break
    @unknown default: throw TranslationBridgeError.failed
    }
    return try await withTaskCancellationHandler {
        try Task.checkCancellation()
        try checkCancellation()
        let ready = await session.isReady()
        try Task.checkCancellation()
        try checkCancellation()
        if ready { return .installed }
        // Give an initially unready session one initialization attempt before
        // reporting missing resources. It cannot request downloads or display
        // consent UI; Rust retains the request's existing deadline.
        do {
            try await session.prepare()
        } catch {
            try Task.checkCancellation()
            try checkCancellation()
            switch error {
            case TranslationError.notInstalled: return .supported
            default: throw error
            }
        }
        try Task.checkCancellation()
        try checkCancellation()
        let prepared = await session.isReady()
        try Task.checkCancellation()
        try checkCancellation()
        return prepared ? .installed : .supported
    } onCancel: {
        session.cancel()
    }
}

@available(macOS 26.0, *)
@MainActor
private final class PreparationState: NSObject, NSWindowDelegate {
    let request: TranslationRequest
    let panel: NSPanel
    var continuation: CheckedContinuation<Void, any Error>?
    var session: TranslationSessionHandle?
    var started = false
    var closing = false

    init(request: TranslationRequest, panel: NSPanel, continuation: CheckedContinuation<Void, any Error>) {
        self.request = request
        self.panel = panel
        self.continuation = continuation
    }

    func windowWillClose(_ notification: Notification) {
        if !closing { request.cancel(suppressOutput: false) }
    }
}

private func preparationInstruction(_ uiLanguage: String) -> String {
    switch uiLanguage {
    case "zh": "请按照系统提示下载或启用翻译语言包。"
    case "zh-TW": "請依照系統提示下載或啟用翻譯語言套件。"
    case "ja": "システムの案内に従って翻訳言語をダウンロード、または有効にしてください。"
    case "de": "Folgen Sie dem macOS-Dialog, um Übersetzungssprachen zu laden oder zu aktivieren."
    case "fr": "Suivez les indications de macOS pour télécharger ou activer les langues de traduction."
    case "ko": "macOS 안내에 따라 번역 언어를 다운로드하거나 활성화하세요."
    case "th": "ทำตามคำแนะนำของ macOS เพื่อดาวน์โหลดหรือเปิดใช้ภาษาสำหรับการแปล"
    case "ru": "Следуйте указаниям macOS, чтобы загрузить или включить языки перевода."
    default: "Follow the macOS prompt to download or enable translation languages."
    }
}

private func preparationCancelLabel(_ uiLanguage: String) -> String {
    switch uiLanguage {
    case "zh": "取消"
    case "zh-TW": "取消"
    case "ja": "キャンセル"
    case "de": "Abbrechen"
    case "fr": "Annuler"
    case "ko": "취소"
    case "th": "ยกเลิก"
    case "ru": "Отмена"
    default: "Cancel"
    }
}

@available(macOS 26.0, *)
@MainActor
private struct PreparationView: View {
    let request: TranslationRequest
    let source: Locale.Language
    let target: Locale.Language
    let uiLanguage: String

    private var instruction: String { preparationInstruction(uiLanguage) }
    private var cancelLabel: String { preparationCancelLabel(uiLanguage) }
    private var pair: String {
        let locale = Locale(identifier: uiLanguage)
        let sourceName = locale.localizedString(forIdentifier: source.minimalIdentifier) ?? source.minimalIdentifier
        let targetName = locale.localizedString(forIdentifier: target.minimalIdentifier) ?? target.minimalIdentifier
        return "\(sourceName) → \(targetName)"
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(pair).font(.headline)
            Text(instruction).fixedSize(horizontal: false, vertical: true)
            HStack {
                ProgressView().controlSize(.small)
                Spacer()
                Button(cancelLabel) { request.cancel(suppressOutput: false) }.keyboardShortcut(.cancelAction)
            }
        }
        .padding(24)
        .frame(width: 370)
        .translationTask(source: source, target: target) { session in
            await TranslationWork.shared.prepareSession(session, request: request, source: source, target: target)
        }
    }
}

@available(macOS 26.0, *)
@MainActor
private final class TranslationWork {
    static let shared = TranslationWork()
    private var sessions: [UInt64: TranslationSessionHandle] = [:]
    private var preparations: [UInt64: PreparationState] = [:]

    func cancel(_ identifier: UInt64) {
        sessions[identifier]?.cancel()
        guard let preparation = preparations[identifier] else { return }
        preparation.session?.cancel()
        preparation.closing = true
        preparation.panel.close()
        if !preparation.started {
            completePreparation(identifier, result: .failure(TranslationBridgeError.cancelled))
        }
    }

    nonisolated private func status(_ source: Locale.Language, _ target: Locale.Language, request: TranslationRequest) async throws -> LanguageAvailability.Status {
        let availability = await LanguageAvailability().status(from: source, to: target)
        try request.checkCancellation()
        guard availability != .unsupported else { return .unsupported }
        let session = TranslationSessionHandle(TranslationSession(installedSource: source, target: target))
        defer { session.cancel() }
        return try await installedTranslationStatus(availability, session: session) { try request.checkCancellation() }
    }

    nonisolated private func supportedLanguageIDs() async -> [String] {
        await LanguageAvailability().supportedLanguages.map(\.minimalIdentifier).sorted()
    }

    private func pair(_ source: String, _ target: String) -> (Locale.Language, Locale.Language) {
        (Locale.Language(identifier: source), Locale.Language(identifier: target))
    }

    func run(_ request: TranslationRequest) async {
        defer {
            sessions.removeValue(forKey: request.identifier)?.cancel()
            request.finish()
        }
        do {
            try request.checkCancellation()
            switch request.operation {
            case .query:
                let languages = await supportedLanguageIDs()
                try request.checkCancellation()
                guard languages.count <= 256 else { throw TranslationBridgeError.invalidResult }
                request.send(["event": "capabilities", "available": !languages.isEmpty, "languages": languages])
            case let .status(source, target):
                let (source, target) = pair(source, target)
                let value = try await status(source, target, request: request)
                try request.checkCancellation()
                let name: String
                switch value {
                case .installed: name = "installed"
                case .supported: name = "supported"
                case .unsupported: name = "unsupported"
                @unknown default: throw TranslationBridgeError.failed
                }
                request.send(["event": "status", "status": name])
            case let .prepare(source, target, uiLanguage):
                let (source, target) = pair(source, target)
                switch try await status(source, target, request: request) {
                case .installed: break
                case .supported:
                    try request.checkCancellation()
                    try await showPreparation(request, source: source, target: target, uiLanguage: uiLanguage)
                case .unsupported: throw TranslationBridgeError.unsupportedLanguage
                @unknown default: throw TranslationBridgeError.failed
                }
                try request.checkCancellation()
                guard try await status(source, target, request: request) == .installed else { throw TranslationBridgeError.assetsMissing }
                request.send(["event": "prepared"])
            case let .translate(source, target, text):
                let (source, target) = pair(source, target)
                let availability = await LanguageAvailability().status(from: source, to: target)
                try request.checkCancellation()
                guard availability != .unsupported else { throw TranslationBridgeError.unsupportedLanguage }
                // One installed-only session owns both readiness and the
                // translation, so cancelling a probe cannot retire its state.
                let session = TranslationSessionHandle(TranslationSession(installedSource: source, target: target))
                sessions[request.identifier] = session
                switch try await installedTranslationStatus(availability, session: session, checkCancellation: { try request.checkCancellation() }) {
                case .installed: break
                case .supported: throw TranslationBridgeError.assetsMissing
                case .unsupported: throw TranslationBridgeError.unsupportedLanguage
                @unknown default: throw TranslationBridgeError.failed
                }
                try request.checkCancellation()
                let result = try await session.translate(text)
                try request.checkCancellation()
                guard !result.targetText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                      result.targetText.utf8.count <= 65_536 else { throw TranslationBridgeError.invalidResult }
                request.send(["event": "translation", "text": result.targetText])
            }
        } catch {
            // Some framework cancellation paths surface a generic service
            // error. Preserve the user's cancellation without exposing it as
            // a retryable native failure.
            do {
                try request.checkCancellation()
                request.fail(Self.safeError(error))
            } catch {
                request.fail(.cancelled)
            }
        }
    }

    private func showPreparation(_ request: TranslationRequest, source: Locale.Language, target: Locale.Language, uiLanguage: String) async throws {
        try await withCheckedThrowingContinuation { continuation in
            let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 418, height: 170),
                                styleMask: [.titled, .closable], backing: .buffered, defer: false)
            panel.title = "Apple Translation"
            panel.isReleasedWhenClosed = false
            panel.hidesOnDeactivate = false
            panel.contentView = NSHostingView(rootView: PreparationView(request: request, source: source, target: target, uiLanguage: uiLanguage))
            let state = PreparationState(request: request, panel: panel, continuation: continuation)
            panel.delegate = state
            preparations[request.identifier] = state
            panel.center()
            panel.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
        }
    }

    func prepareSession(_ session: TranslationSession, request: TranslationRequest, source: Locale.Language, target: Locale.Language) async {
        guard let preparation = preparations[request.identifier], !preparation.started else { session.cancel(); return }
        preparation.started = true
        let handle = TranslationSessionHandle(session)
        preparation.session = handle
        do {
            try request.checkCancellation()
            try await handle.prepare()
            try request.checkCancellation()
            guard try await status(source, target, request: request) == .installed else { throw TranslationBridgeError.assetsMissing }
            completePreparation(request.identifier, result: .success(()))
        } catch {
            completePreparation(request.identifier, result: .failure(error))
        }
    }

    private func completePreparation(_ identifier: UInt64, result: Result<Void, any Error>) {
        guard let preparation = preparations.removeValue(forKey: identifier) else { return }
        preparation.closing = true
        preparation.session?.cancel()
        preparation.panel.close()
        let continuation = preparation.continuation
        preparation.continuation = nil
        continuation?.resume(with: result)
    }

    private static func safeError(_ error: any Error) -> TranslationBridgeError {
        if let error = error as? TranslationBridgeError { return error }
        if error is CancellationError { return .cancelled }
        switch error {
        case TranslationError.notInstalled: return .assetsMissing
        case TranslationError.alreadyCancelled: return .cancelled
        case TranslationError.unsupportedSourceLanguage, TranslationError.unsupportedTargetLanguage,
             TranslationError.unsupportedLanguagePairing, TranslationError.unableToIdentifyLanguage: return .unsupportedLanguage
        case TranslationError.nothingToTranslate: return .invalidInput
        default: return .failed
        }
    }
}

private func submit(_ identifier: UInt64, _ operation: TranslationOperation, _ callback: @escaping MimiTranslationCallback) {
    let request = TranslationRequest(identifier, operation, callback)
    guard #available(macOS 26.0, *) else {
        switch operation {
        case .query: request.send(["event": "capabilities", "available": false, "languages": []])
        case .status: request.send(["event": "status", "status": "unavailable"])
        default: request.fail(.unavailable)
        }
        return
    }
    guard TranslationRequests.shared.insert(request) else { request.fail(.busy); return }
    // Reserve before dispatching, so immediate Rust cancellation can always
    // mark this operation before its MainActor task creates a session/window.
    request.setTask(Task { @MainActor in await TranslationWork.shared.run(request) })
}

private func validLanguage(_ value: String) -> Bool {
    !value.isEmpty && value.lowercased() != "auto" && value.utf8.count <= 64
        && value.utf8.allSatisfy { (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || $0 == 45 || $0 == 95 }
}

private func languagePair(_ source: UnsafePointer<CChar>, _ target: UnsafePointer<CChar>) -> (String, String)? {
    let source = String(cString: source)
    let target = String(cString: target)
    return validLanguage(source) && validLanguage(target) ? (source, target) : nil
}

private func invalid(_ identifier: UInt64, _ callback: @escaping MimiTranslationCallback) {
    TranslationRequest(identifier, .query, callback).fail(.invalidInput)
}

@_cdecl("mimi_apple_translation_query")
public func mimiAppleTranslationQuery(_ identifier: UInt64, _ callback: @escaping MimiTranslationCallback) {
    submit(identifier, .query, callback)
}

@_cdecl("mimi_apple_translation_status")
public func mimiAppleTranslationStatus(_ identifier: UInt64, _ source: UnsafePointer<CChar>, _ target: UnsafePointer<CChar>, _ callback: @escaping MimiTranslationCallback) {
    guard let (source, target) = languagePair(source, target) else { invalid(identifier, callback); return }
    submit(identifier, .status(source, target), callback)
}

private func supportsPreparationUiLanguage(_ language: String) -> Bool {
    ["en", "zh", "zh-TW", "ja", "de", "fr", "ko", "th", "ru"].contains(language)
}

@_cdecl("mimi_apple_translation_prepare")
public func mimiAppleTranslationPrepare(_ identifier: UInt64, _ source: UnsafePointer<CChar>, _ target: UnsafePointer<CChar>, _ uiLanguage: UnsafePointer<CChar>, _ callback: @escaping MimiTranslationCallback) {
    let language = String(cString: uiLanguage)
    guard let (source, target) = languagePair(source, target), supportsPreparationUiLanguage(language) else { invalid(identifier, callback); return }
    submit(identifier, .prepare(source, target, language), callback)
}

@_cdecl("mimi_apple_translation_translate")
public func mimiAppleTranslationTranslate(_ identifier: UInt64, _ source: UnsafePointer<CChar>, _ target: UnsafePointer<CChar>, _ bytes: UnsafePointer<UInt8>?, _ length: Int, _ callback: @escaping MimiTranslationCallback) {
    guard let (source, target) = languagePair(source, target), let bytes, length > 0, length <= 65_536,
          let text = String(bytes: UnsafeBufferPointer(start: bytes, count: length), encoding: .utf8),
          !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { invalid(identifier, callback); return }
    submit(identifier, .translate(source, target, text), callback)
}

@_cdecl("mimi_apple_translation_cancel")
public func mimiAppleTranslationCancel(_ identifier: UInt64) {
    TranslationRequests.shared.cancel(identifier)
}

#if MIMI_TRANSLATION_TESTS
private final class TestCallbacks: @unchecked Sendable {
    static let shared = TestCallbacks()
    private let lock = NSLock()
    private var results: [Data] = []
    func append(_ data: Data) { lock.lock(); results.append(data); lock.unlock() }
    var count: Int { lock.lock(); defer { lock.unlock() }; return results.count }
}

private let testCallback: MimiTranslationCallback = { _, bytes, length in
    guard let bytes else { return 0 }
    TestCallbacks.shared.append(Data(bytes: bytes, count: length))
    return 1
}

private final class TestInstalledTranslationSession: InstalledTranslationSession, @unchecked Sendable {
    let canRequestDownloads: Bool
    private let lock = NSLock()
    private var readiness: [Bool]
    private var operations: [String] = []
    private let prepareError: (any Error)?
    private let onReady: @Sendable (Int) -> Void
    private let onPrepare: @Sendable () -> Void

    init(_ readiness: [Bool], canRequestDownloads: Bool = false, prepareError: (any Error)? = nil,
         onReady: @escaping @Sendable (Int) -> Void = { _ in }, onPrepare: @escaping @Sendable () -> Void = {}) {
        self.readiness = readiness
        self.canRequestDownloads = canRequestDownloads
        self.prepareError = prepareError
        self.onReady = onReady
        self.onPrepare = onPrepare
    }

    var calls: [String] { lock.withLock { operations } }
    func isReady() async -> Bool {
        let (ready, check) = lock.withLock {
            operations.append("ready")
            return (readiness.isEmpty ? false : readiness.removeFirst(), operations.filter { $0 == "ready" }.count)
        }
        onReady(check)
        return ready
    }
    func prepare() async throws {
        lock.withLock { operations.append("prepare") }
        onPrepare()
        if let prepareError { throw prepareError }
    }
    func cancel() { lock.withLock { operations.append("cancel") } }
}

private final class TestTranslationCancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var cancelled = false
    func cancel() { lock.withLock { cancelled = true } }
    func check() throws {
        if lock.withLock({ cancelled }) { throw TranslationBridgeError.cancelled }
    }
}

private enum TestTranslationFailure: Error { case serviceUnavailable }

@available(macOS 26.0, *)
@MainActor
func runTranslationReadinessTests() async throws {
    // A cold installed-only session may need initialization even when global
    // inventory has not caught up after launch. Neither path can show UI.
    for availability in [LanguageAvailability.Status.installed, .supported] {
        let cold = TestInstalledTranslationSession([false, true])
        let status = try await installedTranslationStatus(availability, session: cold, checkCancellation: {})
        precondition(status == .installed, "Cold installed sessions must recover without settings preparation")
        precondition(cold.calls == ["ready", "prepare", "ready"])

        let ready = TestInstalledTranslationSession([true])
        let readyStatus = try await installedTranslationStatus(availability, session: ready, checkCancellation: {})
        precondition(readyStatus == .installed)
        precondition(ready.calls == ["ready"], "Ready sessions must not prepare again")
    }

    let missing = TestInstalledTranslationSession([false, false])
    let missingStatus = try await installedTranslationStatus(.installed, session: missing, checkCancellation: {})
    precondition(missingStatus == .supported)
    precondition(missing.calls == ["ready", "prepare", "ready"], "Preparation must have a single bounded attempt")

    let removed = TestInstalledTranslationSession([false], prepareError: TranslationError.notInstalled)
    let removedStatus = try await installedTranslationStatus(.supported, session: removed, checkCancellation: {})
    precondition(removedStatus == .supported)
    precondition(removed.calls == ["ready", "prepare"])

    let failed = TestInstalledTranslationSession([false], prepareError: TestTranslationFailure.serviceUnavailable)
    do {
        _ = try await installedTranslationStatus(.installed, session: failed, checkCancellation: {})
        preconditionFailure("Service failures must not be reported as missing assets")
    } catch TestTranslationFailure.serviceUnavailable {}
    precondition(failed.calls == ["ready", "prepare"])

    let unsupported = TestInstalledTranslationSession([true])
    let unsupportedStatus = try await installedTranslationStatus(.unsupported, session: unsupported, checkCancellation: {})
    precondition(unsupportedStatus == .unsupported && unsupported.calls.isEmpty)

    let downloading = TestInstalledTranslationSession([true], canRequestDownloads: true)
    do {
        _ = try await installedTranslationStatus(.installed, session: downloading, checkCancellation: {})
        preconditionFailure("Automatic recovery must reject sessions that can request downloads")
    } catch TranslationBridgeError.failed {}
    precondition(downloading.calls.isEmpty)

    let cancelled = TestTranslationCancellation()
    cancelled.cancel()
    let unused = TestInstalledTranslationSession([true])
    do {
        _ = try await installedTranslationStatus(.installed, session: unused, checkCancellation: { try cancelled.check() })
        preconditionFailure("Cancelled requests must not begin native work")
    } catch TranslationBridgeError.cancelled {}
    precondition(unused.calls.isEmpty)

    for cancelDuringPrepare in [false, true] {
        let cancellation = TestTranslationCancellation()
        let interrupted = TestInstalledTranslationSession([false, true],
            onReady: { _ in if !cancelDuringPrepare { cancellation.cancel() } },
            onPrepare: { if cancelDuringPrepare { cancellation.cancel() } })
        do {
            _ = try await installedTranslationStatus(.installed, session: interrupted, checkCancellation: { try cancellation.check() })
            preconditionFailure("Cancellation must be rechecked after every native await")
        } catch TranslationBridgeError.cancelled {}
        precondition(interrupted.calls == (cancelDuringPrepare ? ["ready", "prepare"] : ["ready"]))
    }

    let finalCancellation = TestTranslationCancellation()
    let finalCheck = TestInstalledTranslationSession([false, true], onReady: { check in
        if check == 2 { finalCancellation.cancel() }
    })
    do {
        _ = try await installedTranslationStatus(.installed, session: finalCheck, checkCancellation: { try finalCancellation.check() })
        preconditionFailure("A cancelled final readiness check must not publish installation")
    } catch TranslationBridgeError.cancelled {}
    precondition(finalCheck.calls == ["ready", "prepare", "ready"])

    let taskCancelled = TestInstalledTranslationSession([false], onReady: { _ in
        withUnsafeCurrentTask { task in task?.cancel() }
    })
    let task = Task { try await installedTranslationStatus(.installed, session: taskCancelled, checkCancellation: {}) }
    do {
        _ = try await task.value
        preconditionFailure("Task cancellation must stop preparation and cancel its native session")
    } catch is CancellationError {}
    precondition(taskCancelled.calls == ["ready", "cancel"])
}

// These tests exercise the native lifetime boundary without initializing a
// TranslationSession, showing UI, querying assets, or accessing the network.
@MainActor
func runTranslationBridgeInvariantTests() throws {
    for language in ["en", "zh", "zh-TW", "ja", "de", "fr", "ko", "th", "ru"] {
        precondition(supportsPreparationUiLanguage(language))
    }
    for language in ["system", "zh-Hant", "zh_tw", "fr-CA", "invalid", ""] {
        precondition(!supportsPreparationUiLanguage(language))
    }
    precondition(preparationInstruction("zh-TW") == "請依照系統提示下載或啟用翻譯語言套件。")
    precondition(preparationInstruction("zh-TW") != preparationInstruction("zh"))
    precondition(preparationCancelLabel("zh-TW") == "取消")
    precondition(preparationInstruction("th") != preparationInstruction("en"))
    precondition(preparationCancelLabel("th") == "ยกเลิก")
    precondition(preparationInstruction("ru") != preparationInstruction("en"))
    precondition(preparationCancelLabel("ru") == "Отмена")
    let registry = TranslationRequests()
    let requests = (1...8).map { TranslationRequest(UInt64($0), .query, testCallback) }
    for request in requests { precondition(registry.insert(request)) }
    let extra = TranslationRequest(9, .query, testCallback)
    precondition(!registry.insert(extra))
    registry.cancel(1)
    precondition(!registry.insert(extra), "Cancellation must retain capacity until native work exits")
    registry.finish(requests[0])
    precondition(registry.insert(extra))
    for request in requests { registry.finish(request) }
    registry.finish(extra)

    let first = TranslationRequest(20, .prepare("en", "ja", "en"), testCallback)
    let second = TranslationRequest(21, .prepare("en", "zh", "en"), testCallback)
    precondition(registry.insert(first))
    precondition(!registry.insert(second), "Only one preparation window may exist")
    registry.finish(first)
    precondition(registry.insert(second))
    registry.finish(second)

    let cancelled = TranslationRequest(30, .query, testCallback)
    cancelled.cancel(suppressOutput: true)
    let pending = Task<Void, Never> {}
    cancelled.setTask(pending)
    precondition(pending.isCancelled, "Cancellation before dispatch must cancel the eventual task")
    let previous = TestCallbacks.shared.count
    cancelled.fail(.failed)
    precondition(TestCallbacks.shared.count == previous, "Dropped callers must not receive late results")
    do { try cancelled.checkCancellation(); preconditionFailure("Cancelled request continued") }
    catch TranslationBridgeError.cancelled { }

    let single = TranslationRequest(31, .query, testCallback)
    single.fail(.invalidInput)
    single.fail(.failed)
    precondition(TestCallbacks.shared.count == previous + 1, "Only one terminal callback is allowed")
    precondition(validLanguage("zh-Hans") && validLanguage("en_US"))
    precondition(!validLanguage("auto") && !validLanguage("") && !validLanguage("en\n") && !validLanguage(String(repeating: "a", count: 65)))
}
#endif
