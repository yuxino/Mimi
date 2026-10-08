import Foundation
import Darwin
import MLX
import MLXAudioCore
import MLXAudioSTT

// Only the private protocol goes to stdout. Dependency print/debug output and
// raw exceptions must never enter Mimi's diagnostics or subtitle transport.
final class Output {
    let handle = FileHandle(fileDescriptor: dup(STDOUT_FILENO), closeOnDealloc: true)
    init() { dup2(STDERR_FILENO, STDOUT_FILENO) }
    func send(_ value: [String: Any]) throws {
        var bytes = try JSONSerialization.data(withJSONObject: value)
        guard bytes.count <= 64 * 1024 else { throw WorkerError.invalid }
        bytes.append(10)
        try handle.write(contentsOf: bytes)
    }
}

enum WorkerError: Error { case invalid }

// readLine() has no bound. This reader rejects oversized private messages and
// retains at most one message plus a small input chunk.
struct Input {
    var bytes = Data()
    mutating func next() throws -> [String: Any]? {
        while true {
            if let end = bytes.firstIndex(of: 10) {
                let line = bytes[..<end]
                bytes.removeSubrange(...end)
                guard line.count <= 256 * 1024,
                      let value = try JSONSerialization.jsonObject(with: line) as? [String: Any]
                else { throw WorkerError.invalid }
                return value
            }
            guard bytes.count <= 256 * 1024 else { throw WorkerError.invalid }
            // FileHandle.read(upToCount:) may wait to fill its request on a
            // pipe. POSIX read returns currently available bytes, so a small
            // commit/ping never waits for more audio or stdin EOF.
            var chunk = [UInt8](repeating: 0, count: 4096)
            let count = Darwin.read(STDIN_FILENO, &chunk, chunk.count)
            if count < 0 && errno == EINTR { continue }
            guard count >= 0 else { throw WorkerError.invalid }
            guard count > 0 else {
                guard bytes.isEmpty else { throw WorkerError.invalid }
                return nil
            }
            bytes.append(contentsOf: chunk.prefix(count))
        }
    }
}

@main
struct MimiLocalSpeech {
    static func main() async {
        let output = Output()
        do {
            let arguments = CommandLine.arguments
            guard arguments.count == 5 else { throw WorkerError.invalid }
            let kind = arguments[1]
            let directory = URL(fileURLWithPath: arguments[2], isDirectory: true)
            let language = arguments[3] == "auto" ? nil : arguments[3]
            GPU.metallib = URL(fileURLWithPath: arguments[4])
            Memory.cacheLimit = 128 * 1024 * 1024
            guard kind == "qwen-small" || kind == "qwen-standard" else { throw WorkerError.invalid }
            // Load local files only; never invoke the runtime's downloader.
            let qwen = try await Qwen3ASRModel.fromModelDirectory(directory)
            // Compile common GPU kernels before capture starts.
            _ = qwen.generate(audio: MLXArray(Array(repeating: Float(0), count: 16_000)), maxTokens: 1)
            try output.send(["type": "ready"])
            var input = Input()
            var samples: [Float] = []
            var id: UInt64 = 0
            var revision: UInt64 = 0
            var sampleCount = 0
            var lastPreviewCount = 0
            var previousPreview = ""
            while let command = try input.next() {
                guard let type = command["type"] as? String else { throw WorkerError.invalid }
                switch type {
                case "start":
                    guard let turn = command["id"] as? UInt64, let rev = command["revision"] as? UInt64 else { throw WorkerError.invalid }
                    id = turn; revision = rev; sampleCount = 0; lastPreviewCount = 0
                    samples.removeAll(keepingCapacity: true); previousPreview = ""
                case "audio":
                    guard id > 0, let encoded = command["audio"] as? String,
                          let pcm = Data(base64Encoded: encoded), pcm.count <= 128 * 1024,
                          pcm.count.isMultiple(of: 2), sampleCount + pcm.count / 2 <= 128_000
                    else { throw WorkerError.invalid }
                    let next = stride(from: 0, to: pcm.count, by: 2).map { index in
                        Float(Int16(bitPattern: UInt16(pcm[index]) | UInt16(pcm[index + 1]) << 8)) / 32768
                    }
                    sampleCount += next.count
                    samples.append(contentsOf: next)
                    var preview: String?
                    if sampleCount - lastPreviewCount >= 25_600 {
                        lastPreviewCount = sampleCount
                        preview = qwen.generate(audio: MLXArray(samples), maxTokens: 384, language: language).text
                    }
                    if let preview, preview != previousPreview, !preview.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        guard preview.utf8.count <= 32 * 1024 else { throw WorkerError.invalid }
                        previousPreview = preview
                        var event: [String: Any] = ["type": "draft", "id": id, "revision": revision, "text": preview]
                        if let language { event["language"] = language }
                        try output.send(event)
                    }
                case "commit":
                    guard id > 0 else { throw WorkerError.invalid }
                    var text = ""
                    var detected = language
                    if !samples.isEmpty {
                        let result = qwen.generate(audio: MLXArray(samples), maxTokens: 384, language: language)
                        text = result.text
                        detected = language ?? languageCode(result.language)
                    }
                    guard text.utf8.count <= 32 * 1024 else { throw WorkerError.invalid }
                    var event: [String: Any] = ["type": "final", "id": id, "revision": revision, "text": text]
                    if let detected { event["language"] = detected }
                    try output.send(event)
                    id = 0; samples.removeAll(keepingCapacity: true)
                case "clear":
                    id = 0; samples.removeAll(keepingCapacity: true)
                case "ping": try output.send(["type": "pong", "id": command["id"] ?? 0])
                case "finish":
                    try output.send(["type": "finished"])
                    return
                default: throw WorkerError.invalid
                }
            }
        } catch {
            try? output.send(["type": "error", "label": "local_model_runtime_failed"])
        }
    }

    static func languageCode(_ value: String?) -> String? {
        guard let value else { return nil }
        let names = ["chinese": "zh", "english": "en", "japanese": "ja", "korean": "ko", "cantonese": "yue",
                     "arabic": "ar", "german": "de", "french": "fr", "spanish": "es", "portuguese": "pt", "indonesian": "id",
                     "italian": "it", "russian": "ru", "thai": "th", "vietnamese": "vi", "turkish": "tr", "hindi": "hi",
                     "malay": "ms", "dutch": "nl", "swedish": "sv", "danish": "da", "finnish": "fi", "polish": "pl",
                     "czech": "cs", "filipino": "tl", "persian": "fa", "greek": "el", "hungarian": "hu", "macedonian": "mk", "romanian": "ro"]
        return names[value.lowercased()]
    }
}
