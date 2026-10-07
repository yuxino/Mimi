import Foundation

private struct Reply: Decodable, Sendable {
    let event: String
    let available: Bool?
    let languages: [String]?
    let status: String?
    let text: String?
    let code: Int?
}

private final class ProbeReplies: @unchecked Sendable {
    static let shared = ProbeReplies()
    private let lock = NSLock()
    private var next: UInt64 = 100
    private var routes: [UInt64: CheckedContinuation<Data, Never>] = [:]

    func register(_ continuation: CheckedContinuation<Data, Never>) -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        next += 1
        routes[next] = continuation
        return next
    }

    func finish(_ identifier: UInt64, _ data: Data) {
        lock.lock()
        let continuation = routes.removeValue(forKey: identifier)
        lock.unlock()
        continuation?.resume(returning: data)
    }
}

private let probeCallback: MimiTranslationCallback = { identifier, bytes, length in
    guard let bytes, length > 0, length <= 262_144 else { return 0 }
    ProbeReplies.shared.finish(identifier, Data(bytes: bytes, count: length))
    return 1
}

@MainActor
private func reply(_ start: (UInt64) -> Void) async throws -> Reply {
    let data = await withCheckedContinuation { continuation in
        let identifier = ProbeReplies.shared.register(continuation)
        start(identifier)
        Task {
            try? await Task.sleep(for: .seconds(15))
            mimiAppleTranslationCancel(identifier)
            ProbeReplies.shared.finish(identifier, Data(#"{"event":"timeout"}"#.utf8))
        }
    }
    return try JSONDecoder().decode(Reply.self, from: data)
}

@main
private struct BridgeTests {
    @MainActor
    static func main() async throws {
        try runTranslationBridgeInvariantTests()
        print("native_invariants=passed")
        guard CommandLine.arguments.contains("--probe-installed") else { return }
        // Opt-in synthetic proof only. No preparation call is reachable from
        // this executable, so unsupported/uninstalled pairs cannot download.
        let capabilities = try await reply { mimiAppleTranslationQuery($0, probeCallback) }
        print("capabilities_event=\(capabilities.event) available=\(capabilities.available ?? false) languages=\(capabilities.languages?.count ?? 0)")
        guard capabilities.available == true else { return }
        for target in ["zh-Hans", "ja", "fr"] {
            let status = try await reply { identifier in
                "en".withCString { source in
                    target.withCString { target in
                        mimiAppleTranslationStatus(identifier, source, target, probeCallback)
                    }
                }
            }
            print("pair=en->\(target) event=\(status.event) status=\(status.status ?? "unknown") code=\(status.code ?? 0)")
            guard status.status == "installed" else { continue }
            let fixture = "The library opens at nine."
            let translated = try await reply { identifier in
                "en".withCString { source in
                    target.withCString { target in
                        Array(fixture.utf8).withUnsafeBufferPointer { bytes in
                            mimiAppleTranslationTranslate(identifier, source, target, bytes.baseAddress, bytes.count, probeCallback)
                        }
                    }
                }
            }
            let changed = translated.text.map { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && $0 != fixture } ?? false
            print("translation_event=\(translated.event) code=\(translated.code ?? 0) output_bytes=\(translated.text?.utf8.count ?? 0) changed_nonempty=\(changed)")
            if changed { return }
        }
        print("installed_translation_proof=unavailable")
    }
}
