import ApplicationServices
import Foundation
import ImageIO
import UniformTypeIdentifiers

private enum CaptionCaptureError: Error, CustomStringConvertible {
    case failed(String)
    var description: String {
        switch self { case .failed(let label): return label }
    }
}

private let systemCaption = "Mimi system audio test: can you see the shared screen?"

private func axAttribute(_ element: AXUIElement, _ name: CFString) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name, &value) == .success else { return nil }
    return value
}

func axFrame(_ element: AXUIElement) -> CGRect? {
    guard let rawPosition = axAttribute(element, kAXPositionAttribute as CFString),
          let rawSize = axAttribute(element, kAXSizeAttribute as CFString),
          CFGetTypeID(rawPosition) == AXValueGetTypeID(), CFGetTypeID(rawSize) == AXValueGetTypeID()
    else { return nil }
    var origin = CGPoint.zero
    var size = CGSize.zero
    guard AXValueGetValue(rawPosition as! AXValue, .cgPoint, &origin),
          AXValueGetValue(rawSize as! AXValue, .cgSize, &size), size.width > 0, size.height > 0
    else { return nil }
    // AXPosition and CGWindow bounds both use Quartz global top-left points.
    return CGRect(origin: origin, size: size)
}

private func sameFrame(_ a: CGRect, _ b: CGRect) -> Bool {
    abs(a.minX - b.minX) <= 1 && abs(a.minY - b.minY) <= 1 &&
        abs(a.width - b.width) <= 1 && abs(a.height - b.height) <= 1
}

private func windowInfo(_ windowID: CGWindowID, _ frame: CGRect) throws -> [String: Any] {
    guard windowID != kCGNullWindowID,
          let rows = CGWindowListCopyWindowInfo(.optionIncludingWindow, windowID) as? [[String: Any]],
          let row = rows.first(where: { ($0[kCGWindowNumber as String] as? NSNumber)?.uint32Value == windowID }),
          let bounds = row[kCGWindowBounds as String] as? [String: Any],
          let actual = CGRect(dictionaryRepresentation: bounds as CFDictionary), sameFrame(actual, frame)
    else { throw CaptionCaptureError.failed("caption_window_geometry_unavailable") }
    return row
}

func visibleAXMatches(pid: pid_t, frame: CGRect, matching: (AXUIElement) -> Bool) throws
    -> [(element: AXUIElement, ax: CGRect, crop: CGRect)] {
    let app = AXUIElementCreateApplication(pid)
    guard let windows = axAttribute(app, kAXWindowsAttribute as CFString) as? [AXUIElement] else {
        throw CaptionCaptureError.failed("caption_ax_windows_unavailable")
    }
    let matches = windows.filter { axFrame($0).map { sameFrame($0, frame) } ?? false }
    guard matches.count == 1 else { throw CaptionCaptureError.failed("caption_ax_window_not_unique") }
    var pending: [(AXUIElement, Int)] = [(matches[0], 0)]
    var visited = 0
    var found: [(element: AXUIElement, ax: CGRect, crop: CGRect)] = []
    while let (element, depth) = pending.popLast() {
        visited += 1
        guard visited <= 4096 else { throw CaptionCaptureError.failed("caption_ax_tree_limit") }
        if matching(element), let bounds = axFrame(element) {
            let visible = bounds.intersection(frame.insetBy(dx: 4, dy: 4))
            if !visible.isNull && visible.width >= 8 && visible.height >= 8 { found.append((element, bounds, visible)) }
        }
        if depth < 20, let children = axAttribute(element, kAXChildrenAttribute as CFString) as? [AXUIElement] {
            pending.append(contentsOf: children.map { ($0, depth + 1) })
        }
    }
    // WebKit groups and their children can expose the same name. Prefer the
    // tight visible rectangle within the requested native window.
    return found.sorted { $0.crop.width * $0.crop.height < $1.crop.width * $1.crop.height }
}

private func readWindowImage(_ windowID: CGWindowID, _ frame: CGRect, isolateAttachedWindows: Bool = false) throws -> CGImage {
    _ = try windowInfo(windowID, frame)
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("mimi-caption-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    defer { try? FileManager.default.removeItem(at: directory) }
    let path = directory.appendingPathComponent("window.png")
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    process.arguments = ["-x", "-o"] + (isolateAttachedWindows ? ["-a"] : []) + ["-l\(windowID)", path.path]
    process.standardOutput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
    let finished = DispatchSemaphore(value: 0)
    process.terminationHandler = { _ in finished.signal() }
    try process.run()
    guard finished.wait(timeout: .now() + 5) == .success else {
        process.terminate()
        if finished.wait(timeout: .now() + 1) == .timedOut {
            kill(process.processIdentifier, SIGKILL)
            _ = finished.wait(timeout: .now() + 1)
        }
        throw CaptionCaptureError.failed("caption_single_window_capture_timeout")
    }
    // ImageIO can decode lazily. Own the PNG bytes after the scratch file is removed.
    guard process.terminationStatus == 0,
          let bytes = try? Data(contentsOf: path),
          let source = CGImageSourceCreateWithData(bytes as CFData, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else { throw CaptionCaptureError.failed("caption_single_window_capture_failed") }
    _ = try windowInfo(windowID, frame)
    return image
}

private func readWindow(_ windowID: CGWindowID, _ frame: CGRect, isolateAttachedWindows: Bool = false) throws -> CGImage {
    let image = try readWindowImage(windowID, frame, isolateAttachedWindows: isolateAttachedWindows)
    let sx = CGFloat(image.width) / frame.width
    let sy = CGFloat(image.height) / frame.height
    guard sx >= 1, sy >= 1, abs(sx - sy) <= max(1 / frame.width, 1 / frame.height) else {
        throw CaptionCaptureError.failed("caption_capture_scale_mismatch")
    }
    return image
}

func pixelRect(_ crop: CGRect, _ frame: CGRect, _ image: CGImage) throws -> CGRect {
    guard !crop.isNull, crop.width > 0, crop.height > 0, frame.contains(crop) else {
        throw CaptionCaptureError.failed("caption_crop_outside_target_window")
    }
    let sx = CGFloat(image.width) / frame.width
    let sy = CGFloat(image.height) / frame.height
    // CGImage cropping uses first-row/first-pixel coordinates, with no AppKit
    // bottom-left conversion. Keep native Retina pixels rather than resampling.
    return CGRect(x: (crop.minX - frame.minX) * sx, y: (crop.minY - frame.minY) * sy,
                  width: crop.width * sx, height: crop.height * sy).integral
        .intersection(CGRect(x: 0, y: 0, width: image.width, height: image.height))
}

private func save(_ image: CGImage, frame: CGRect, crop: CGRect, output: URL, stage: String) throws {
    guard stage.range(of: "^[A-Za-z0-9_-]{1,64}$", options: .regularExpression) != nil,
          let clipped = image.cropping(to: try pixelRect(crop, frame, image))
    else { throw CaptionCaptureError.failed("caption_capture_stage_or_crop_invalid") }
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
    let path = output.appendingPathComponent(stage + ".png")
    guard let destination = CGImageDestinationCreateWithURL(path as CFURL, UTType.png.identifier as CFString, 1, nil) else {
        throw CaptionCaptureError.failed("caption_png_destination_failed")
    }
    let properties = [kCGImagePropertyDPIWidth: 72 * CGFloat(image.width) / frame.width,
                      kCGImagePropertyDPIHeight: 72 * CGFloat(image.height) / frame.height] as CFDictionary
    CGImageDestinationAddImage(destination, clipped, properties)
    guard CGImageDestinationFinalize(destination) else { throw CaptionCaptureError.failed("caption_png_write_failed") }
    print("caption_capture stage=\(stage) pixels=\(clipped.width)x\(clipped.height) crop=\(crop)")
}

private struct StrokeResult {
    var point: CGPoint?
    let rect: CGRect
    let background: Int
    let maxBrightness: Int
    let maxAlpha: UInt8
    var thresholdHits = 0
    var neighborHits = 0
}

private func stroke(in image: CGImage, frame: CGRect, crop: CGRect) throws -> StrokeResult {
    let rect = try pixelRect(crop.insetBy(dx: 1, dy: 1), frame, image)
    guard let lane = image.cropping(to: rect) else { throw CaptionCaptureError.failed("caption_pixel_crop_failed") }
    let width = lane.width, height = lane.height
    var pixels = [UInt8](repeating: 0, count: width * height * 4)
    try pixels.withUnsafeMutableBytes { bytes in
        guard let context = CGContext(data: bytes.baseAddress, width: width, height: height, bitsPerComponent: 8,
                                      bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue)
        else { throw CaptionCaptureError.failed("caption_pixel_buffer_failed") }
        context.draw(lane, in: CGRect(x: 0, y: 0, width: width, height: height))
    }
    func brightness(_ index: Int) -> Int {
        let offset = index * 4
        return (Int(pixels[offset]) + Int(pixels[offset + 1]) + Int(pixels[offset + 2])) / 3
    }
    // The histogram gives the same upper median as sorting every pixel. Only
    // the local RGBA buffer is needed; failed AX candidates retain scalar stats.
    var histogram = [Int](repeating: 0, count: 256)
    var maxBrightness = 0
    var maxAlpha: UInt8 = 0
    for index in 0..<(width * height) {
        let value = brightness(index)
        histogram[value] += 1
        maxBrightness = max(maxBrightness, value)
        maxAlpha = max(maxAlpha, pixels[index * 4 + 3])
    }
    var background = 0, cumulative = 0
    for (value, count) in histogram.enumerated() {
        cumulative += count
        if cumulative > width * height / 2 { background = value; break }
    }
    var result = StrokeResult(point: nil, rect: rect, background: background, maxBrightness: maxBrightness, maxAlpha: maxAlpha)
    guard background <= 120, width >= 6, height >= 6 else { return result }
    var best: (score: Double, x: Int, y: Int, contrast: Int)?
    for y in 2..<(height - 2) {
        for x in 2..<(width - 2) {
            let i = y * width + x
            let level = brightness(i)
            let r = pixels[i * 4], g = pixels[i * 4 + 1], b = pixels[i * 4 + 2]
            guard level >= 160, level - background >= 60,
                  Int(max(r, max(g, b))) - Int(min(r, min(g, b))) <= 24 else { continue }
            result.thresholdHits += 1
            guard [i - 1, i + 1, i - width, i + width].contains(where: { brightness($0) >= 140 }) else { continue }
            result.neighborHits += 1
            let score = pow(Double(x) - Double(width - 1) / 2, 2) + pow(Double(y) - Double(height - 1) / 2, 2)
            if best == nil || score < best!.score { best = (score, x, y, level - background) }
        }
    }
    guard let best else { return result }
    let point = CGPoint(x: frame.minX + (rect.minX + CGFloat(best.x) + 0.5) * frame.width / CGFloat(image.width),
                        y: frame.minY + (rect.minY + CGFloat(best.y) + 0.5) * frame.height / CGFloat(image.height))
    print("caption_stroke point=\(point) contrast=\(best.contrast) nativeScale=\(CGFloat(image.width) / frame.width)")
    result.point = point
    return result
}

/// Both returned coordinates use Quartz global logical points, top-left origin.
/// `crop` is the visible original-language AX lane, not a window-relative rectangle.
/// `output` is a directory; stage.png contains only this lane at native resolution.
func locateCaption(pid: pid_t, windowID: CGWindowID, frame: CGRect, output: URL, stage: String) throws -> (point: CGPoint, crop: CGRect) {
    let frames = try visibleAXMatches(pid: pid, frame: frame) { element in
        [kAXValueAttribute, kAXTitleAttribute, kAXDescriptionAttribute].contains {
            (axAttribute(element, $0 as CFString) as? String) == systemCaption
        }
    }
    return try locateTextStroke(pid: pid, windowID: windowID, frame: frame,
                                candidates: frames.map { ($0.ax, $0.crop) }, output: output, stage: stage)
}

/// Candidates must be visible text from this window's AX tree. Keep text/role
/// selection with each scene; pixel verification and capture stay independent.
func locateTextStroke(pid: pid_t, windowID: CGWindowID, frame: CGRect, candidates frames: [(ax: CGRect, crop: CGRect)],
                      isolateAttachedWindows: Bool = false, output: URL, stage: String) throws -> (point: CGPoint, crop: CGRect) {
    let info = try windowInfo(windowID, frame)
    guard (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid else {
        throw CaptionCaptureError.failed("caption_window_pid_mismatch")
    }
    guard !frames.isEmpty else { throw CaptionCaptureError.failed("target_ax_text_not_found stage=\(stage)") }
    let image = try readWindow(windowID, frame, isolateAttachedWindows: isolateAttachedWindows)
    var failures: [StrokeResult] = []
    for candidate in frames {
        let result = try stroke(in: image, frame: frame, crop: candidate.crop)
        if let point = result.point {
            try save(image, frame: frame, crop: candidate.crop, output: output, stage: stage)
            return (point, candidate.crop)
        }
        failures.append(result)
    }
    // Diagnose the exact rejected image. No fresh window capture or weaker
    // pixel predicate is substituted for the original failed stroke check.
    print("caption_capture_failure root=\(frame) imagePixels=\(image.width)x\(image.height) scaleX=\(CGFloat(image.width) / frame.width) scaleY=\(CGFloat(image.height) / frame.height) bits=\(image.bitsPerComponent)/\(image.bitsPerPixel) alphaInfo=\(image.alphaInfo.rawValue) bitmapInfo=\(image.bitmapInfo.rawValue) visibleAXMatches=\(frames.count)")
    func saveFailure(_ crop: CGRect, suffix: String) {
        do { try save(image, frame: frame, crop: crop, output: output, stage: stage + suffix) }
        catch { print("caption_failure_png suffix=\(suffix) error=\(error)") }
    }
    saveFailure(frame, suffix: "-root-failed")
    for (index, candidate) in frames.enumerated() {
        let sample = failures[index]
        print("caption_pixel_failure index=\(index) ax=\(candidate.ax) visibleCrop=\(candidate.crop) samplePixels=\(sample.rect) background=\(sample.background) maxBrightness=\(sample.maxBrightness) maxAlpha=\(sample.maxAlpha) allTransparent=\(sample.maxAlpha == 0) thresholdHits=\(sample.thresholdHits) neighborHits=\(sample.neighborHits)")
        saveFailure(candidate.crop, suffix: "-ax-\(index)-failed")
    }
    throw CaptionCaptureError.failed("target_has_no_verified_stroke stage=\(stage)")
}

/// `frame` and `crop` are Quartz global logical points. Translate both after a
/// native drag; the same windowID is captured without reading any other app.
/// Isolate attached surfaces when the expanded panel extends beyond its parent.
func captureFrame(windowID: CGWindowID, frame: CGRect, crop: CGRect, isolateAttachedWindows: Bool = false, output: URL, stage: String) throws {
    try save(readWindow(windowID, frame, isolateAttachedWindows: isolateAttachedWindows), frame: frame, crop: crop, output: output, stage: stage)
}

struct GlyphWitness {
    let point: CGPoint?
    let neighborPixels: Int
    let background: Int
    let maxAlpha: UInt8
    let width: Int
    let height: Int
    var visible: Bool { point != nil && neighborPixels >= 8 }
    var json: [String: Any] {
        ["visibleGlyph": visible, "neighborPixels": neighborPixels, "background": background,
         "maxAlpha": maxAlpha, "pixelWidth": width, "pixelHeight": height]
    }
}

/// Keep the original single-window pixels, including a transparent child that
/// macOS may trim to 1x1. The parent and desktop must never enter its evidence.
func captureIsolatedWindow(windowID: CGWindowID, frame: CGRect, output: URL, stage: String) throws -> CGImage {
    let image = try readWindowImage(windowID, frame, isolateAttachedWindows: true)
    guard stage.range(of: "^[A-Za-z0-9_-]{1,64}$", options: .regularExpression) != nil,
          let destination = CGImageDestinationCreateWithURL(output.appendingPathComponent(stage + ".png") as CFURL,
                                                           UTType.png.identifier as CFString, 1, nil) else {
        throw CaptionCaptureError.failed("control_witness_destination_invalid")
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else { throw CaptionCaptureError.failed("control_witness_write_failed") }
    return image
}

func captureGlyphWitness(windowID: CGWindowID, frame: CGRect, crop: CGRect, output: URL, stage: String) throws -> GlyphWitness {
    let image = try captureIsolatedWindow(windowID: windowID, frame: frame, output: output, stage: stage)
    return try glyphWitness(in: image, frame: frame, crop: crop)
}

func glyphWitness(in image: CGImage, frame: CGRect, crop: CGRect) throws -> GlyphWitness {
    let sx = CGFloat(image.width) / frame.width, sy = CGFloat(image.height) / frame.height
    if sx >= 1, sy >= 1, abs(sx - sy) <= max(1 / frame.width, 1 / frame.height) {
        let result = try stroke(in: image, frame: frame, crop: crop)
        return GlyphWitness(point: result.point, neighborPixels: result.neighborHits, background: result.background,
                            maxAlpha: result.maxAlpha, width: image.width, height: image.height)
    }
    // Only transparency explains an accepted size mismatch. An opaque or
    // partly painted cropped window must fail instead of being called hidden.
    guard try !windowHasPaintedPixels(image) else {
        throw CaptionCaptureError.failed("control_witness_nontransparent_scale_mismatch")
    }
    return GlyphWitness(point: nil, neighborPixels: 0, background: 0, maxAlpha: 0, width: image.width, height: image.height)
}

/// WindowServer can trim transparent margins. Whole-window visibility does
/// not need to assume any mapping between these pixels and the logical frame.
func windowHasPaintedPixels(_ image: CGImage) throws -> Bool {
    var rgba = [UInt8](repeating: 0, count: image.width * image.height * 4)
    try rgba.withUnsafeMutableBytes { bytes in
        guard let context = CGContext(data: bytes.baseAddress, width: image.width, height: image.height, bitsPerComponent: 8,
                                      bytesPerRow: image.width * 4, space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else {
            throw CaptionCaptureError.failed("control_witness_alpha_buffer_failed")
        }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
    }
    return stride(from: 3, to: rgba.count, by: 4).contains { rgba[$0] != 0 }
}
