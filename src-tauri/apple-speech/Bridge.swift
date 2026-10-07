import Foundation
import Speech
import AVFoundation
import CoreMedia
import OSLog

// Callbacks contain bounded UTF-8 JSON. The Rust callback copies bytes before
// returning; identifiers never expose a Rust object pointer to asynchronous Swift.
public typealias MimiSpeechCallback = @convention(c) (UInt64, UnsafePointer<UInt8>?, Int) -> Int32

private enum BridgeError: Int, Error {
    case unavailable = 1, assetsNotInstalled, invalidLocale, incompatibleFormat
    case invalidPCM, inputOverflow, closed, outputLimit, invalidResult
    case reservationLimit, resourcesUnavailable, serviceUnavailable, downloadCancelled
    case downloadNetwork, downloadStorage, statusUnavailable, downloadTimeout
    case assetsDownloading
}

private let resourceLogger = Logger(subsystem: "app.yuxino.mimi", category: "AppleSpeechResources")

@available(macOS 26.0, *)
private func resourceStatusLabel(_ status: AssetInventory.Status) -> String {
    switch status {
    case .unsupported: return "unsupported"
    case .supported: return "supported"
    case .downloading: return "downloading"
    case .installed: return "installed"
    @unknown default: return "unknown"
    }
}

private func logResourceFailure(_ error: any Error, localeCode: String, attempt: Int) {
    let value = error as NSError
    let code = (error as? BridgeError)?.rawValue ?? value.code
    let allowed = ["SFSpeechErrorDomain", "NSOSStatusErrorDomain", "NSCocoaErrorDomain", "NSPOSIXErrorDomain", "NSURLErrorDomain"]
    let domain = error is BridgeError ? "MimiAppleSpeech" : (allowed.contains(value.domain) ? value.domain : "SpeechFrameworkError")
    resourceLogger.notice("prepare_failure locale=\(localeCode, privacy: .public) attempt=\(attempt) domain=\(domain, privacy: .public) code=\(code)")
}

// Inspect only public error domains/codes. Never send descriptions or userInfo
// across the bridge: they can contain local paths and system-private context.
@available(macOS 26.0, *)
private func resourceFailure(_ error: any Error) -> any Error {
    if error is BridgeError { return error }
    if error is CancellationError { return BridgeError.downloadCancelled }
    var value = error as NSError
    var fallback: (any Error)?
    for _ in 0..<4 {
        if value.domain == NSCocoaErrorDomain {
            switch value.code {
            case NSXPCConnectionInterrupted, NSXPCConnectionInvalid: return BridgeError.serviceUnavailable
            case NSFileWriteOutOfSpaceError: return BridgeError.downloadStorage
            case NSUserCancelledError: return BridgeError.downloadCancelled
            default: break
            }
        } else if value.domain == NSURLErrorDomain {
            switch value.code {
            case NSURLErrorCancelled: return BridgeError.downloadCancelled
            case NSURLErrorTimedOut: return BridgeError.downloadTimeout
            case NSURLErrorCannotFindHost, NSURLErrorCannotConnectToHost,
                 NSURLErrorNetworkConnectionLost, NSURLErrorDNSLookupFailed,
                 NSURLErrorNotConnectedToInternet, NSURLErrorInternationalRoamingOff,
                 NSURLErrorDataNotAllowed: return BridgeError.downloadNetwork
            default: break
            }
        } else if value.domain == NSPOSIXErrorDomain && value.code == 28 {
            return BridgeError.downloadStorage
        } else if value.domain == SFSpeechErrorDomain {
            switch value.code {
            case SFSpeechError.Code.tooManyAssetLocalesAllocated.rawValue: return BridgeError.reservationLimit
            case SFSpeechError.Code.insufficientResources.rawValue: return BridgeError.resourcesUnavailable
            case SFSpeechError.Code.cannotAllocateUnsupportedLocale.rawValue: return BridgeError.invalidLocale
            case SFSpeechError.Code.timeout.rawValue: return BridgeError.downloadTimeout
            case SFSpeechError.Code.internalServiceError.rawValue: fallback = BridgeError.serviceUnavailable
            default: break
            }
        }
        guard let underlying = value.userInfo[NSUnderlyingErrorKey] as? NSError else { break }
        value = underlying
    }
    return fallback ?? error
}

@available(macOS 26.0, *)
private func prepareResources(locale: Locale) async throws {
    // This locale was resolved from SpeechTranscriber's supported catalogue.
    // Diagnostics are limited to this explicit preparation, never audio/text.
    let localeCode = locale.identifier(.bcp47)
    // A fresh module/request can recover a transient Speech service disconnect.
    // Only this explicit action retries; capability checks never download.
    for attempt in 0..<2 {
        try Task.checkCancellation()
        resourceLogger.notice("prepare_begin locale=\(localeCode, privacy: .public) attempt=\(attempt)")
        let transcriber = SpeechTranscriber(locale: locale, preset: .timeIndexedProgressiveTranscription)
        do {
            let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber])
            let requestIsNil = request == nil ? 1 : 0
            resourceLogger.notice("prepare_request locale=\(localeCode, privacy: .public) attempt=\(attempt) request_is_nil=\(requestIsNil)")
            if let request {
                resourceLogger.notice("download_begin locale=\(localeCode, privacy: .public) attempt=\(attempt)")
                try await request.downloadAndInstall()
                resourceLogger.notice("download_returned locale=\(localeCode, privacy: .public) attempt=\(attempt)")
            }
            // Installation completion and the service's status cache can settle
            // separately. Recheck briefly without re-downloading or claiming
            // that the global installedLocales list proves module readiness.
            for check in 0..<4 {
                try Task.checkCancellation()
                let status = await AssetInventory.status(forModules: [transcriber])
                let statusLabel = resourceStatusLabel(status)
                resourceLogger.notice("prepare_status locale=\(localeCode, privacy: .public) attempt=\(attempt) check=\(check) status=\(statusLabel, privacy: .public)")
                if status == .installed { return }
                if check < 3 { try await Task.sleep(for: .milliseconds(250)) }
            }
            try Task.checkCancellation()
            let installedLocales = await SpeechTranscriber.installedLocales
            let localeListed = installedLocales.contains { $0.identifier(.bcp47) == localeCode } ? 1 : 0
            resourceLogger.notice("prepare_inventory locale=\(localeCode, privacy: .public) attempt=\(attempt) locale_listed_installed=\(localeListed)")
            try Task.checkCancellation()
            let formats = await transcriber.availableCompatibleAudioFormats
            let formatCount = formats.count
            resourceLogger.notice("prepare_formats locale=\(localeCode, privacy: .public) attempt=\(attempt) count=\(formatCount)")
            throw BridgeError.statusUnavailable
        } catch {
            logResourceFailure(error, localeCode: localeCode, attempt: attempt)
            let failure = resourceFailure(error)
            if attempt == 0, (failure as? BridgeError) == .serviceUnavailable {
                try await Task.sleep(for: .milliseconds(300))
                continue
            }
            throw failure
        }
    }
}

private struct LocaleResourceStatus: Sendable {
    let identifier: String
    let status: String
}

@available(macOS 26.0, *)
private func resourceStatuses(locales: [Locale]) async throws -> [LocaleResourceStatus] {
    var values: [LocaleResourceStatus] = []
    // Reduce serial catalogue latency without issuing an unbounded number of
    // Speech service requests at once. Rust still enforces the query deadline.
    for first in stride(from: 0, to: locales.count, by: 4) {
        try Task.checkCancellation()
        let batch = locales[first..<min(first + 4, locales.count)]
        let results = try await withThrowingTaskGroup(of: LocaleResourceStatus.self) { group in
            for locale in batch {
                group.addTask {
                    try Task.checkCancellation()
                    let transcriber = SpeechTranscriber(locale: locale, preset: .timeIndexedProgressiveTranscription)
                    let status = await AssetInventory.status(forModules: [transcriber])
                    try Task.checkCancellation()
                    return LocaleResourceStatus(
                        identifier: locale.identifier(.bcp47),
                        status: resourceStatusLabel(status)
                    )
                }
            }
            var results: [LocaleResourceStatus] = []
            for try await result in group { results.append(result) }
            return results
        }
        values.append(contentsOf: results)
    }
    return values
}

private final class Output: @unchecked Sendable {
    let identifier: UInt64
    let callback: MimiSpeechCallback
    private let lock = NSRecursiveLock()
    private var closed = false

    init(identifier: UInt64, callback: @escaping MimiSpeechCallback) {
        self.identifier = identifier
        self.callback = callback
    }

    func send(_ fields: [String: Any]) throws {
        let data = try JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])
        guard data.count <= 262_144 else { throw BridgeError.outputLimit }
        lock.lock()
        defer { lock.unlock() }
        guard !closed else { throw BridgeError.closed }
        let accepted = data.withUnsafeBytes { callback(identifier, $0.bindMemory(to: UInt8.self).baseAddress, $0.count) }
        if accepted == 0 { closed = true; throw BridgeError.closed }
    }

    func fail(_ error: any Error) {
        let value = error as NSError
        let code = (error as? BridgeError)?.rawValue ?? value.code
        let known = ["SFSpeechErrorDomain", "NSOSStatusErrorDomain", "NSCocoaErrorDomain", "NSPOSIXErrorDomain", "NSURLErrorDomain"]
        let domain = error is BridgeError ? "MimiAppleSpeech" : (known.contains(value.domain) ? value.domain : "SpeechFrameworkError")
        try? send(["event": "error", "domain": domain, "code": code])
    }

    func close() { lock.lock(); closed = true; lock.unlock() }
}

@available(macOS 26.0, *)
private final class Session: @unchecked Sendable {
    let input = PCMQueue()
    let output: Output
    private let lock = NSLock()
    private var task: Task<Void, Never>?
    private var analyzer: SpeechAnalyzer?
    private var cancelled = false

    init(output: Output) { self.output = output }

    func begin(localeName: String) {
        let running = Task { await run(localeName: localeName) }
        lock.lock()
        if cancelled { lock.unlock(); running.cancel() }
        else { task = running; lock.unlock() }
    }

    private func installAnalyzer(_ value: SpeechAnalyzer) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        if cancelled { return false }
        analyzer = value
        return true
    }

    private func clearHandles() -> SpeechAnalyzer? {
        lock.lock()
        let current = analyzer
        analyzer = nil
        task = nil
        lock.unlock()
        return current
    }

    func cancel() {
        lock.lock()
        cancelled = true
        let running = task
        let current = analyzer
        task = nil
        analyzer = nil
        lock.unlock()
        output.close()
        input.cancel()
        running?.cancel()
        if let current { Task { await current.cancelAndFinishNow() } }
    }

    private func run(localeName: String) async {
        defer {
            _ = clearHandles()
            Registry.shared.removeSession(output.identifier)
        }
        do {
            guard SpeechTranscriber.isAvailable else { throw BridgeError.unavailable }
            guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: localeName)) else {
                throw BridgeError.invalidLocale
            }
            let transcriber = SpeechTranscriber(locale: locale, preset: .timeIndexedProgressiveTranscription)
            switch await AssetInventory.status(forModules: [transcriber]) {
            case .installed: break
            case .supported: throw BridgeError.assetsNotInstalled
            case .downloading: throw BridgeError.assetsDownloading
            case .unsupported: throw BridgeError.statusUnavailable
            @unknown default: throw BridgeError.statusUnavailable
            }
            try Task.checkCancellation()
            let formats = await transcriber.availableCompatibleAudioFormats
            guard let format = formats.first(where: {
                $0.sampleRate == 16_000 && $0.channelCount == 1 && $0.commonFormat == .pcmFormatInt16
            }) else { throw BridgeError.incompatibleFormat }
            let analyzer = SpeechAnalyzer(modules: [transcriber], options: .init(priority: .userInitiated, modelRetention: .whileInUse))
            guard installAnalyzer(analyzer) else { throw CancellationError() }
            try await analyzer.prepareToAnalyze(in: format)
            try Task.checkCancellation()
            let sequence = PCMSequence(queue: input, format: format)
            try await analyzer.start(inputSequence: sequence)
            try output.send(["event": "ready"])
            // This result task owns only one latest preview and a final watermark.
            // Results have stable native start times; every preview replaces text.
            let reader = Task {
                do {
                    try await forwardResults(transcriber: transcriber, output: output)
                    // A result stream ending before EOF cannot produce the
                    // remaining audio. Wake the input wait instead of hanging.
                    if !sequence.completion.isFinished { throw BridgeError.closed }
                } catch {
                    if !(error is CancellationError) { output.fail(error) }
                    output.close()
                    input.cancel()
                    sequence.completion.finish(error: error)
                    Task { await analyzer.cancelAndFinishNow() }
                    throw error
                }
            }
            do {
                // Wait until the parent closes this source's input, then flush.
                try await sequence.completion.wait()
                try Task.checkCancellation()
                try await analyzer.finalizeAndFinishThroughEndOfInput()
                try await reader.value
                try output.send(["event": "done"])
            } catch {
                input.cancel()
                reader.cancel()
                // Rust imposes the externally observable deadline and removes
                // callback routing immediately on cancellation; never block it.
                Task { await analyzer.cancelAndFinishNow() }
                throw error
            }
        } catch {
            input.cancel()
            if let current = clearHandles() { Task { await current.cancelAndFinishNow() } }
            if !(error is CancellationError) { output.fail(error) }
        }
        output.close()
    }
}

private final class EndSignal: @unchecked Sendable {
    private let lock = NSLock()
    private var result: Result<Void, any Error>?
    private var waiter: CheckedContinuation<Void, any Error>?
    var isFinished: Bool { lock.lock(); defer { lock.unlock() }; return result != nil }
    func finish(error: (any Error)? = nil) {
        lock.lock()
        guard result == nil else { lock.unlock(); return }
        let outcome: Result<Void, any Error> = error.map { .failure($0) } ?? .success(())
        result = outcome
        let waiting = waiter
        waiter = nil
        lock.unlock()
        waiting?.resume(with: outcome)
    }
    func wait() async throws {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                lock.lock()
                if let result { lock.unlock(); continuation.resume(with: result) }
                else if Task.isCancelled { lock.unlock(); continuation.resume(throwing: CancellationError()) }
                else { waiter = continuation; lock.unlock() }
            }
        } onCancel: { self.finish(error: CancellationError()) }
    }
}

@available(macOS 26.0, *)
private struct PCMSequence: AsyncSequence, Sendable {
    typealias Element = AnalyzerInput
    let queue: PCMQueue
    let format: AVAudioFormat
    let completion = EndSignal()
    func makeAsyncIterator() -> Iterator { Iterator(queue: queue, format: format, completion: completion) }
    struct Iterator: AsyncIteratorProtocol {
        let queue: PCMQueue
        let format: AVAudioFormat
        let completion: EndSignal
        var frames: Int64 = 0
        mutating func next() async throws -> AnalyzerInput? {
            do { return try await nextInput() }
            catch { completion.finish(error: error); throw error }
        }
        private mutating func nextInput() async throws -> AnalyzerInput? {
            guard let data = await queue.next() else { completion.finish(); return nil }
            try Task.checkCancellation()
            guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(data.count / 2)),
                  let destination = buffer.int16ChannelData?[0] else { throw BridgeError.incompatibleFormat }
            buffer.frameLength = AVAudioFrameCount(data.count / 2)
            data.withUnsafeBytes { source in
                destination.withMemoryRebound(to: UInt8.self, capacity: data.count) { target in
                    target.update(from: source.bindMemory(to: UInt8.self).baseAddress!, count: data.count)
                }
            }
            let result = AnalyzerInput(buffer: buffer, bufferStartTime: CMTime(value: frames, timescale: 16_000))
            frames += Int64(buffer.frameLength)
            return result
        }
    }
}

@available(macOS 26.0, *)
private func forwardResults(transcriber: SpeechTranscriber, output: Output) async throws {
    var finalizedThrough: CMTime?
    var previous: (start: CMTime, end: CMTime, text: String)?
    for try await result in transcriber.results {
        try Task.checkCancellation()
        let start = result.range.start
        let end = CMTimeRangeGetEnd(result.range)
        let startMS = CMTimeGetSeconds(start) * 1_000
        let endMS = CMTimeGetSeconds(end) * 1_000
        guard startMS.isFinite, endMS.isFinite, startMS >= 0, endMS >= startMS else { throw BridgeError.invalidResult }
        if let through = finalizedThrough, CMTimeCompare(end, through) <= 0 { continue }
        let text = String(result.text.characters)
        guard text.utf8.count <= 65_536 else { throw BridgeError.outputLimit }
        if !result.isFinal, let previous, previous.text == text,
           CMTimeCompare(previous.start, start) == 0, CMTimeCompare(previous.end, end) == 0 { continue }
        try output.send(["event": "result", "text": text, "start_ms": startMS, "end_ms": endMS, "final": result.isFinal])
        if result.isFinal { finalizedThrough = end; previous = nil }
        else { previous = (start, end, text) }
    }
}

@available(macOS 26.0, *)
private final class Registry: @unchecked Sendable {
    static let shared = Registry()
    private let lock = NSLock()
    private var sessions: [UInt64: Session] = [:]
    private var requests: [UInt64: Task<Void, Never>] = [:]
    func addSession(_ value: Session, identifier: UInt64) { lock.lock(); sessions[identifier] = value; lock.unlock() }
    func session(_ identifier: UInt64) -> Session? { lock.lock(); defer { lock.unlock() }; return sessions[identifier] }
    func removeSession(_ identifier: UInt64) { lock.lock(); sessions.removeValue(forKey: identifier); lock.unlock() }
    func request(_ identifier: UInt64, operation: @escaping @Sendable () async -> Void) {
        // Hold the lock through task registration so immediate completion cannot
        // leave an already-finished task retained in the dictionary.
        lock.lock()
        requests[identifier] = Task { await operation(); self.removeRequest(identifier) }
        lock.unlock()
    }
    func removeRequest(_ identifier: UInt64) { lock.lock(); requests.removeValue(forKey: identifier); lock.unlock() }
    func cancel(_ identifier: UInt64) {
        lock.lock()
        let session = sessions.removeValue(forKey: identifier)
        let request = requests.removeValue(forKey: identifier)
        lock.unlock()
        session?.cancel()
        request?.cancel()
    }
}

@_cdecl("mimi_apple_speech_query")
public func mimiAppleSpeechQuery(_ identifier: UInt64, _ callback: @escaping MimiSpeechCallback) {
    let output = Output(identifier: identifier, callback: callback)
    if #available(macOS 26.0, *) {
        Registry.shared.request(identifier) {
            do {
                guard SpeechTranscriber.isAvailable else {
                    try output.send(["event": "capabilities", "available": false, "locales": []]); return
                }
                let locales = await SpeechTranscriber.supportedLocales
                let statuses = try await resourceStatuses(locales: locales)
                let values: [[String: Any]] = statuses.map { status in
                    ["identifier": status.identifier, "status": status.status]
                }
                try output.send(["event": "capabilities", "available": true, "locales": values])
            } catch { if !(error is CancellationError) { output.fail(error) } }
        }
    } else { try? output.send(["event": "capabilities", "available": false, "locales": []]) }
}

@_cdecl("mimi_apple_speech_prepare")
public func mimiAppleSpeechPrepare(_ identifier: UInt64, _ localePointer: UnsafePointer<CChar>, _ callback: @escaping MimiSpeechCallback) {
    let localeName = String(cString: localePointer)
    let output = Output(identifier: identifier, callback: callback)
    if #available(macOS 26.0, *) {
        Registry.shared.request(identifier) {
            do {
                guard SpeechTranscriber.isAvailable else { throw BridgeError.unavailable }
                guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: localeName)) else { throw BridgeError.invalidLocale }
                // Only this explicit user-triggered operation may reserve/download.
                try await prepareResources(locale: locale)
                try output.send(["event": "prepared"])
            } catch {
                // A framework cancellation is an actionable result. A cancelled
                // Rust request has already removed its callback route instead.
                if !Task.isCancelled { output.fail(resourceFailure(error)) }
            }
        }
    } else { output.fail(BridgeError.unavailable) }
}

@_cdecl("mimi_apple_speech_start")
public func mimiAppleSpeechStart(_ identifier: UInt64, _ localePointer: UnsafePointer<CChar>, _ callback: @escaping MimiSpeechCallback) {
    let output = Output(identifier: identifier, callback: callback)
    if #available(macOS 26.0, *) {
        let session = Session(output: output)
        Registry.shared.addSession(session, identifier: identifier)
        session.begin(localeName: String(cString: localePointer))
    } else { output.fail(BridgeError.unavailable) }
}

@_cdecl("mimi_apple_speech_push")
public func mimiAppleSpeechPush(_ identifier: UInt64, _ data: UnsafePointer<UInt8>?, _ count: Int) -> Int32 {
    guard count > 0, count <= 32_000, count.isMultiple(of: 2), let data else { return PCMQueue.PushResult.invalid.rawValue }
    if #available(macOS 26.0, *), let session = Registry.shared.session(identifier) {
        return session.input.push(Data(bytes: data, count: count)).rawValue
    }
    return PCMQueue.PushResult.closed.rawValue
}

@_cdecl("mimi_apple_speech_finish")
public func mimiAppleSpeechFinish(_ identifier: UInt64) {
    if #available(macOS 26.0, *) { Registry.shared.session(identifier)?.input.finish() }
}

@_cdecl("mimi_apple_speech_cancel")
public func mimiAppleSpeechCancel(_ identifier: UInt64) {
    if #available(macOS 26.0, *) { Registry.shared.cancel(identifier) }
}
