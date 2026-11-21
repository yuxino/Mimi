import AppKit
import AVFoundation
import CoreMedia
import Darwin
import ScreenCaptureKit

private struct RecordingError: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

private func hostSeconds() -> Double { CMTimeGetSeconds(CMClockGetTime(CMClockGetHostTimeClock())) }
private func timeJSON(_ value: CMTime) -> [String: Any] {
    ["value": value.value, "timescale": value.timescale, "seconds": CMTimeGetSeconds(value)]
}
private func rectJSON(_ value: CGRect) -> [String: Double] {
    ["x": value.minX, "y": value.minY, "width": value.width, "height": value.height]
}
private func writeJSON(_ value: [String: Any], _ path: URL) throws {
    try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys]).write(to: path, options: .atomic)
}
private func nativeFrame(_ id: CGWindowID, pid: pid_t) throws -> CGRect {
    guard let rows = CGWindowListCopyWindowInfo(.optionIncludingWindow, id) as? [[String: Any]],
          let row = rows.first(where: { ($0[kCGWindowNumber as String] as? NSNumber)?.uint32Value == id }),
          (row[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid,
          let raw = row[kCGWindowBounds as String] as? NSDictionary,
          let bounds = CGRect(dictionaryRepresentation: raw) else { throw RecordingError("owned_window_unavailable") }
    return bounds
}

// All capture/writer state belongs to this serial callback queue. No frame is
// synthesized: only image-bearing native ScreenCaptureKit buffers are appended.
private final class Recorder: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    let queue = DispatchQueue(label: "mimi.qa.window-video")
    let writer: AVAssetWriter
    let input: AVAssetWriterInput
    let output: URL
    let dimensions: (width: Int, height: Int)
    var firstPTS: CMTime?
    var lastPTS: CMTime?
    var firstClock: [String: Any]?
    var lastClock: [String: Any]?
    var frames: [[String: Any]] = []
    var statuses: [String: Int] = [:]
    var missingImage = 0
    var backpressure = 0
    var invalidPTS = 0
    var failure: String?

    init(output: URL, width: Int, height: Int) throws {
        self.output = output
        dimensions = (width, height)
        writer = try AVAssetWriter(outputURL: output.appendingPathComponent("capture.mov"), fileType: .mov)
        input = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height,
            AVVideoCompressionPropertiesKey: [AVVideoExpectedSourceFrameRateKey: 60,
                AVVideoAverageBitRateKey: max(2_000_000, width * height * 12),
                AVVideoMaxKeyFrameIntervalKey: 60, AVVideoAllowFrameReorderingKey: false],
        ])
        input.expectsMediaDataInRealTime = true
        guard writer.canAdd(input) else { throw RecordingError("video_writer_cannot_add_input") }
        writer.add(input)
        guard writer.startWriting() else { throw RecordingError("video_writer_start_failed") }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        queue.async { self.failure = "native_stream_stopped: \(error.localizedDescription)" }
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer buffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, failure == nil else { return }
        let callbackUptime = ProcessInfo.processInfo.systemUptime
        let callbackHost = hostSeconds()
        let attachments = CMSampleBufferGetSampleAttachmentsArray(buffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]]
        let attachment = attachments?.first ?? [:]
        let status = (attachment[.status] as? NSNumber)?.intValue ?? -1
        statuses[String(status), default: 0] += 1
        guard CMSampleBufferIsValid(buffer), CMSampleBufferDataIsReady(buffer),
              let image = CMSampleBufferGetImageBuffer(buffer),
              status == SCFrameStatus.complete.rawValue || status == SCFrameStatus.idle.rawValue || status == SCFrameStatus.started.rawValue else {
            missingImage += 1
            return
        }
        guard CVPixelBufferGetWidth(image) == dimensions.width, CVPixelBufferGetHeight(image) == dimensions.height else {
            failure = "native_frame_dimensions_differ_from_fixed_crop"
            return
        }
        let pts = CMSampleBufferGetPresentationTimeStamp(buffer)
        guard pts.isNumeric, lastPTS.map({ CMTimeCompare(pts, $0) > 0 }) ?? true else { invalidPTS += 1; return }
        guard input.isReadyForMoreMediaData else { backpressure += 1; return }
        if firstPTS == nil { writer.startSession(atSourceTime: pts) }
        guard input.append(buffer) else { failure = "native_sample_append_failed"; return }
        let ticks = (attachment[.displayTime] as? NSNumber)?.uint64Value
        var clock: [String: Any] = ["callbackSystemUptime": callbackUptime, "callbackHostClockSeconds": callbackHost,
                                  "nativePTS": timeJSON(pts), "status": status,
                                  "pixelWidth": CVPixelBufferGetWidth(image), "pixelHeight": CVPixelBufferGetHeight(image)]
        if let ticks {
            clock["displayMachTicks"] = ticks
            clock["displayHostClockSeconds"] = CMTimeGetSeconds(CMClockMakeHostTimeFromSystemUnits(ticks))
        }
        if firstPTS == nil {
            firstPTS = pts
            firstClock = clock
            do { try writeJSON(clock, output.appendingPathComponent("ready")) }
            catch { failure = "ready_marker_write_failed" }
        }
        lastPTS = pts
        lastClock = clock
        frames.append(clock)
    }

    func state() -> (ready: Bool, failure: String?) { queue.sync { (firstPTS != nil, failure) } }

    func summary() -> [String: Any] {
        queue.sync {
            var result: [String: Any] = ["firstFrame": firstClock ?? [:], "lastFrame": lastClock ?? [:],
                "nativeSamplesWritten": frames.count, "frames": frames, "nativeStatuses": statuses,
                "samplesWithoutUsableImage": missingImage, "encoderBackpressureDrops": backpressure,
                "invalidTimestampDrops": invalidPTS]
            if let failure { result["captureFailure"] = failure }
            return result
        }
    }

    func finish() async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            queue.async {
                guard let last = self.lastPTS else {
                    self.writer.cancelWriting()
                    continuation.resume(throwing: RecordingError("no_native_video_frames"))
                    return
                }
                self.writer.endSession(atSourceTime: CMTimeAdd(last, CMTime(value: 1, timescale: 60)))
                self.input.markAsFinished()
                self.writer.finishWriting {
                    self.queue.async {
                        guard self.writer.status == .completed else {
                            continuation.resume(throwing: RecordingError("video_finalize_failed: \(self.writer.error?.localizedDescription ?? "unknown")"))
                            return
                        }
                        continuation.resume()
                    }
                }
            }
        }
    }
}

private func inspectMovie(_ path: URL) async throws -> [String: Any] {
    let asset = AVURLAsset(url: path)
    let video = try await asset.loadTracks(withMediaType: .video)
    let audio = try await asset.loadTracks(withMediaType: .audio)
    guard video.count == 1, audio.isEmpty else { throw RecordingError("expected_exactly_one_video_track_and_no_audio") }
    let reader = try AVAssetReader(asset: asset)
    let output = AVAssetReaderTrackOutput(track: video[0], outputSettings: nil)
    reader.add(output)
    guard reader.startReading() else { throw RecordingError("recorded_video_read_failed") }
    var times: [Double] = []
    while let sample = output.copyNextSampleBuffer() { times.append(CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample))) }
    guard reader.status == .completed, times.count >= 2 else { throw RecordingError("recorded_video_samples_unavailable") }
    let intervals = zip(times.dropFirst(), times).map(-)
    guard intervals.allSatisfy({ $0 > 0 }) else { throw RecordingError("recorded_video_timestamp_order_invalid") }
    let sorted = intervals.sorted()
    let span = times.last! - times.first!
    let fps = Double(times.count - 1) / span
    let p95 = sorted[min(sorted.count - 1, Int(Double(sorted.count - 1) * 0.95))]
    return ["videoSampleCount": times.count, "audioTrackCount": audio.count, "firstVideoPTS": times.first!,
            "lastVideoPTS": times.last!, "measuredFramesPerSecond": fps, "maximumGapSeconds": sorted.last!,
            "p95GapSeconds": p95, "gapsOver34ms": intervals.filter { $0 > 0.034 }.count,
            "cadenceAtLeastAbout30FPS": fps >= 29 && p95 <= 0.035,
            "encodedSamplePTSSeconds": times]
}

@main
private enum WindowRecorder {
    static func main() async {
        do { try await run() } catch {
            FileHandle.standardError.write(Data("window_video_failed: \(error)\n".utf8))
            exit(1)
        }
    }

    static func run() async throws {
        let args = CommandLine.arguments
        guard ProcessInfo.processInfo.environment["RUNNER_ENVIRONMENT"] == "github-hosted",
              ProcessInfo.processInfo.environment["MIMI_UI_TEST"] == "1", CGPreflightScreenCaptureAccess(),
              args.count == 5, let pid = pid_t(args[1]), pid > 0,
              let rootID = CGWindowID(args[2]), let controlID = CGWindowID(args[3]), rootID != controlID,
              let app = NSRunningApplication(processIdentifier: pid),
              ["app.yuxino.mimi", "app.yuxino.mimi.dev"].contains(app.bundleIdentifier ?? "") else {
            throw RecordingError("hosted_owned_ui_test_pid_window_ids_and_capture_permission_required")
        }
        let output = URL(fileURLWithPath: args[4], isDirectory: true)
        let resources = try output.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
        guard resources.isDirectory == true, resources.isSymbolicLink != true,
              ["ready", "stop", "capture.mov", "metadata.json"].allSatisfy({ !FileManager.default.fileExists(atPath: output.appendingPathComponent($0).path) }) else {
            throw RecordingError("existing_caller_owned_empty_output_directory_required")
        }
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
        guard let root = content.windows.first(where: { $0.windowID == rootID }),
              let control = content.windows.first(where: { $0.windowID == controlID }),
              root.owningApplication?.processID == pid, control.owningApplication?.processID == pid,
              root.owningApplication?.bundleIdentifier == app.bundleIdentifier,
              control.owningApplication?.bundleIdentifier == app.bundleIdentifier else {
            throw RecordingError("both_recorded_windows_must_belong_to_the_named_mimi")
        }
        let suffix = app.bundleIdentifier == "app.yuxino.mimi.dev" ? " (dev)" : ""
        guard root.title == "Mimi Subtitles" + suffix, control.title == "Mimi" + suffix else {
            throw RecordingError("expected_test_caption_and_language_windows")
        }
        let crop = root.frame.union(control.frame)
        let displays = content.displays.filter { $0.frame.contains(crop) }
        guard displays.count == 1 else { throw RecordingError("both_windows_must_fit_one_display") }
        let display = displays[0]
        // Inclusion (not exclusion) prevents any other application's pixels,
        // desktop or Dock from entering even the transparent overlay regions.
        let filter = SCContentFilter(display: display, including: [root, control])
        filter.includeMenuBar = false
        let scale = CGFloat(filter.pointPixelScale)
        guard scale >= 1, scale <= 4 else { throw RecordingError("unexpected_native_pixel_scale") }
        let config = SCStreamConfiguration()
        config.sourceRect = crop.offsetBy(dx: -display.frame.minX, dy: -display.frame.minY)
        config.width = Int(ceil(crop.width * scale / 2)) * 2
        config.height = Int(ceil(crop.height * scale / 2)) * 2
        config.minimumFrameInterval = CMTime(value: 1, timescale: 60)
        config.queueDepth = 4
        config.pixelFormat = kCVPixelFormatType_32BGRA
        config.captureResolution = .best
        let background = CGColor(gray: 0, alpha: 1)
        config.backgroundColor = background
        defer { withExtendedLifetime(background) {} }
        config.includeChildWindows = false
        config.capturesAudio = false
        config.captureMicrophone = false
        config.showsCursor = true
        config.showMouseClicks = false
        let recorder = try Recorder(output: output, width: config.width, height: config.height)
        let stream = SCStream(filter: filter, configuration: config, delegate: recorder)
        try stream.addStreamOutput(recorder, type: .screen, sampleHandlerQueue: recorder.queue)
        let startup = ProcessInfo.processInfo.systemUptime
        var metadata: [String: Any] = ["qualification": "isolated synthetic UI QA only", "pid": pid,
            "rootWindowID": rootID, "controlWindowID": controlID, "cropQuartzGlobalPoints": rectJSON(crop),
            "sourceRectDisplayPoints": rectJSON(config.sourceRect), "outputWidth": config.width, "outputHeight": config.height,
            "pointPixelScale": scale, "requestedFramesPerSecond": 60, "audioEnabled": false,
            "includedWindowIDs": [rootID, controlID], "includeChildWindows": false,
            "startedSystemUptime": startup, "startedHostClockSeconds": hostSeconds()]
        var timebase = mach_timebase_info_data_t()
        mach_timebase_info(&timebase)
        metadata["machTimebaseNumer"] = timebase.numer
        metadata["machTimebaseDenom"] = timebase.denom
        signal(SIGTERM, SIG_IGN)
        let termination = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .global())
        termination.setEventHandler { try? Data().write(to: output.appendingPathComponent("stop")) }
        termination.resume()
        defer { termination.cancel() }
        try await stream.startCapture()
        var stopReason = "stop-marker"
        while !FileManager.default.fileExists(atPath: output.appendingPathComponent("stop").path) {
            let state = recorder.state()
            if let failure = state.failure { stopReason = failure; break }
            if !state.ready, ProcessInfo.processInfo.systemUptime - startup > 8 { stopReason = "first_native_frame_timeout"; break }
            if ProcessInfo.processInfo.systemUptime - startup > 45 { stopReason = "recording_duration_cap"; break }
            do {
                if try nativeFrame(rootID, pid: pid) != root.frame || !crop.contains(nativeFrame(controlID, pid: pid)) {
                    stopReason = "recorded_windows_moved_outside_fixed_crop"; break
                }
            } catch { stopReason = "owned_window_unavailable"; break }
            try await Task.sleep(nanoseconds: 25_000_000)
        }
        metadata["stopRequestedSystemUptime"] = ProcessInfo.processInfo.systemUptime
        metadata["stopRequestedHostClockSeconds"] = hostSeconds()
        metadata["stopReason"] = stopReason
        let watchdog = DispatchWorkItem {
            FileHandle.standardError.write(Data("window_video_failed: finalization_exceeded_8s\n".utf8))
            exit(1)
        }
        DispatchQueue.global().asyncAfter(deadline: .now() + 8, execute: watchdog)
        defer { watchdog.cancel() }
        do { try await stream.stopCapture() }
        catch {
            metadata["stopCaptureFailure"] = String(describing: error)
            try? stream.removeStreamOutput(recorder, type: .screen)
        }
        do { try await recorder.finish() }
        catch {
            metadata.merge(recorder.summary()) { _, new in new }
            metadata["finalizationFailure"] = String(describing: error)
            try writeJSON(metadata, output.appendingPathComponent("metadata.json"))
            throw error
        }
        metadata.merge(recorder.summary()) { _, new in new }
        metadata["finishedSystemUptime"] = ProcessInfo.processInfo.systemUptime
        metadata["finishedHostClockSeconds"] = hostSeconds()
        let movie: [String: Any]
        do { movie = try await inspectMovie(output.appendingPathComponent("capture.mov")) }
        catch {
            metadata["videoInspectionFailure"] = String(describing: error)
            try writeJSON(metadata, output.appendingPathComponent("metadata.json"))
            throw error
        }
        metadata["encodedVideo"] = movie
        try writeJSON(metadata, output.appendingPathComponent("metadata.json"))
        guard stopReason == "stop-marker", metadata["captureFailure"] == nil, metadata["stopCaptureFailure"] == nil,
              movie["videoSampleCount"] as? Int == metadata["nativeSamplesWritten"] as? Int,
              movie["cadenceAtLeastAbout30FPS"] as? Bool == true else {
            throw RecordingError("video_or_30fps_cadence_not_qualified_see_metadata")
        }
        print("window_video_finalized native_frames=\(metadata["nativeSamplesWritten"] ?? 0) actual_fps=\(movie["measuredFramesPerSecond"] ?? 0) audio_tracks=0")
    }
}
