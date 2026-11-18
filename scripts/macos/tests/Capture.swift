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

private func axFrame(_ element: AXUIElement) -> CGRect? {
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

private func captionFrames(pid: pid_t, frame: CGRect) throws -> [CGRect] {
    let app = AXUIElementCreateApplication(pid)
    guard let windows = axAttribute(app, kAXWindowsAttribute as CFString) as? [AXUIElement] else {
        throw CaptionCaptureError.failed("caption_ax_windows_unavailable")
    }
    let matches = windows.filter { axFrame($0).map { sameFrame($0, frame) } ?? false }
    guard matches.count == 1 else { throw CaptionCaptureError.failed("caption_ax_window_not_unique") }
    var pending: [(AXUIElement, Int)] = [(matches[0], 0)]
    var visited = 0
    var found: [CGRect] = []
    while let (element, depth) = pending.popLast() {
        visited += 1
        guard visited <= 4096 else { throw CaptionCaptureError.failed("caption_ax_tree_limit") }
        let named = [kAXValueAttribute, kAXTitleAttribute, kAXDescriptionAttribute].contains {
            (axAttribute(element, $0 as CFString) as? String) == systemCaption
        }
        if named, let bounds = axFrame(element) {
            let visible = bounds.intersection(frame.insetBy(dx: 4, dy: 4))
            if !visible.isNull && visible.width >= 8 && visible.height >= 8 { found.append(visible) }
        }
        if depth < 20, let children = axAttribute(element, kAXChildrenAttribute as CFString) as? [AXUIElement] {
            pending.append(contentsOf: children.map { ($0, depth + 1) })
        }
    }
    // A WebKit group and its text can expose the same name. Prefer the tight
    // text rectangle; never substitute another source, translation or blank area.
    return found.sorted { $0.width * $0.height < $1.width * $1.height }
}

private func readWindow(_ windowID: CGWindowID, _ frame: CGRect) throws -> CGImage {
    _ = try windowInfo(windowID, frame)
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("mimi-caption-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    defer { try? FileManager.default.removeItem(at: directory) }
    let path = directory.appendingPathComponent("window.png")
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    process.arguments = ["-x", "-o", "-l\(windowID)", path.path]
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
    guard process.terminationStatus == 0,
          let source = CGImageSourceCreateWithURL(path as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else { throw CaptionCaptureError.failed("caption_single_window_capture_failed") }
    _ = try windowInfo(windowID, frame)
    let sx = CGFloat(image.width) / frame.width
    let sy = CGFloat(image.height) / frame.height
    guard sx >= 1, sy >= 1, abs(sx - sy) <= max(1 / frame.width, 1 / frame.height) else {
        throw CaptionCaptureError.failed("caption_capture_scale_mismatch")
    }
    return image
}

private func pixelRect(_ crop: CGRect, _ frame: CGRect, _ image: CGImage) throws -> CGRect {
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

private func stroke(in image: CGImage, frame: CGRect, crop: CGRect) throws -> CGPoint? {
    let rect = try pixelRect(crop.insetBy(dx: 1, dy: 1), frame, image)
    guard let lane = image.cropping(to: rect) else { return nil }
    let width = lane.width, height = lane.height
    var pixels = [UInt8](repeating: 0, count: width * height * 4)
    try pixels.withUnsafeMutableBytes { bytes in
        guard let context = CGContext(data: bytes.baseAddress, width: width, height: height, bitsPerComponent: 8,
                                      bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue)
        else { throw CaptionCaptureError.failed("caption_pixel_buffer_failed") }
        context.draw(lane, in: CGRect(x: 0, y: 0, width: width, height: height))
    }
    var levels = [Int](repeating: 0, count: width * height)
    for index in levels.indices {
        let offset = index * 4
        levels[index] = (Int(pixels[offset]) + Int(pixels[offset + 1]) + Int(pixels[offset + 2])) / 3
    }
    let background = levels.sorted()[levels.count / 2]
    guard background <= 120, width >= 6, height >= 6 else { return nil }
    var best: (score: Double, x: Int, y: Int, contrast: Int)?
    for y in 2..<(height - 2) {
        for x in 2..<(width - 2) {
            let i = y * width + x
            let channels = [pixels[i * 4], pixels[i * 4 + 1], pixels[i * 4 + 2]]
            guard levels[i] >= 160, levels[i] - background >= 60,
                  Int(channels.max()!) - Int(channels.min()!) <= 24,
                  [i - 1, i + 1, i - width, i + width].contains(where: { levels[$0] >= 140 })
            else { continue }
            let score = pow(Double(x) - Double(width - 1) / 2, 2) + pow(Double(y) - Double(height - 1) / 2, 2)
            if best == nil || score < best!.score { best = (score, x, y, levels[i] - background) }
        }
    }
    guard let best else { return nil }
    let point = CGPoint(x: frame.minX + (rect.minX + CGFloat(best.x) + 0.5) * frame.width / CGFloat(image.width),
                        y: frame.minY + (rect.minY + CGFloat(best.y) + 0.5) * frame.height / CGFloat(image.height))
    print("caption_stroke point=\(point) contrast=\(best.contrast) nativeScale=\(CGFloat(image.width) / frame.width)")
    return point
}

/// Both returned coordinates use Quartz global logical points, top-left origin.
/// `crop` is the visible original-language AX lane, not a window-relative rectangle.
/// `output` is a directory; stage.png contains only this lane at native resolution.
func locateCaption(pid: pid_t, windowID: CGWindowID, frame: CGRect, output: URL, stage: String) throws -> (point: CGPoint, crop: CGRect) {
    let info = try windowInfo(windowID, frame)
    guard (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid else {
        throw CaptionCaptureError.failed("caption_window_pid_mismatch")
    }
    let frames = try captionFrames(pid: pid, frame: frame)
    guard !frames.isEmpty else { throw CaptionCaptureError.failed("caption_original_ax_lane_not_found") }
    let image = try readWindow(windowID, frame)
    for crop in frames {
        if let point = try stroke(in: image, frame: frame, crop: crop) {
            try save(image, frame: frame, crop: crop, output: output, stage: stage)
            return (point, crop)
        }
    }
    throw CaptionCaptureError.failed("caption_original_has_no_verified_stroke")
}

/// `frame` and `crop` are Quartz global logical points. Translate both after a
/// native drag; the same windowID is captured without reading any other app.
func captureFrame(windowID: CGWindowID, frame: CGRect, crop: CGRect, output: URL, stage: String) throws {
    try save(readWindow(windowID, frame), frame: frame, crop: crop, output: output, stage: stage)
}
