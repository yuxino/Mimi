import CoreGraphics
import Foundation
import Vision

private struct ReadoutError: Error, CustomStringConvertible {
    let description: String
    init(_ label: String) { description = label }
}

private let measurements = Set(["接口", "186ms", "427ms"])
private let translating = "翻译中"
private let watched = measurements.union([translating, "翻译耗时"])

private func readStatus(in image: CGImage) throws -> Set<String> {
    // The original PNG is retained. Enlarge the small text for OCR and draw
    // the private window's alpha over black, without capturing the desktop.
    let width = image.width * 4, height = image.height * 4
    guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else {
        throw ReadoutError("status_bitmap_unavailable")
    }
    let bounds = CGRect(x: 0, y: 0, width: width, height: height)
    context.setFillColor(gray: 0, alpha: 1)
    context.fill(bounds)
    context.interpolationQuality = .high
    context.draw(image, in: bounds)
    guard let enlarged = context.makeImage() else { throw ReadoutError("status_bitmap_unavailable") }
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.recognitionLanguages = ["zh-Hans", "en-US"]
    request.usesLanguageCorrection = false
    request.minimumTextHeight = 0.002
    try VNImageRequestHandler(cgImage: enlarged).perform([request])
    let text = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
        .joined().lowercased().filter { !$0.isWhitespace }
    return watched.filter { text.contains($0) }
}

func verifyStatusReadout(rootID: CGWindowID, root: CGRect, childID: CGWindowID, pause: CGRect, fixture: URL, output: URL,
                         acknowledged: (Int) -> Bool, caption: (String) throws -> CGRect?,
                         compactBounds: () throws -> CGRect) throws {
    let pointer = try OverlayTestPointer()
    defer { pointer.restore() }
    let display = CGDisplayBounds(CGMainDisplayID())
    let outside = CGPoint(x: display.minX + 8, y: display.minY + 8)
    guard !root.contains(outside), !(try compactBounds()).contains(outside) else { throw ReadoutError("outside_point_unavailable") }
    let child = try compactBounds()
    // The wide fixture places its numeric group between the capsule and the
    // first toolbar button. A second-line or overlapping reading must fail.
    let row = CGRect(x: child.maxX, y: pause.minY, width: pause.minX - child.maxX, height: pause.height)
    guard abs(root.width - 580) <= 1, row.width > 0, root.contains(row) else { throw ReadoutError("status_wide_toolbar_geometry_invalid") }
    let header = CGRect(x: root.minX, y: root.minY, width: root.width, height: root.height / 2)
    var stages: [[String: Any]] = []
    defer {
        let evidence: [String: Any] = ["rootWindowID": rootID, "controlWindowID": childID, "root": root.dictionaryRepresentation,
                                      "control": child.dictionaryRepresentation, "pause": pause.dictionaryRepresentation,
                                      "numericRow": row.dictionaryRepresentation, "stages": stages]
        if let data = try? JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys]) {
            try? data.write(to: output.appendingPathComponent("status-readout.json"), options: .atomic)
        }
    }
    func advance(to step: Int) throws {
        if step > 0 {
            try Data().write(to: fixture.appendingPathComponent("translation-layout-step-\(step)"), options: .withoutOverwriting)
        }
        let deadline = ProcessInfo.processInfo.systemUptime + 5
        while !acknowledged(step) {
            if ProcessInfo.processInfo.systemUptime >= deadline { throw ReadoutError("status_step_\(step)_not_acknowledged") }
            Thread.sleep(forTimeInterval: 0.04)
        }
    }
    func observe(_ name: String, step: Int, hover: CGWindowID? = nil) throws {
        let working = step == 1 || step == 3 || step == 4
        let expectedRoot = hover == nil ? Set<String>() : measurements
        let expectedChild: Set<String> = working && hover != nil ? [translating] : []
        let deadline = ProcessInfo.processInfo.systemUptime + 5
        var rootWords = Set<String>(), rowWords = Set<String>(), childWords = Set<String>(), childVisible = false
        func matches() -> Bool {
            rootWords == expectedRoot && rowWords == expectedRoot && childWords == expectedChild && childVisible == (hover != nil)
        }
        repeat {
            guard try compactBounds().equalTo(child), acknowledged(step), let cursor = CGEvent(source: nil)?.location else {
                throw ReadoutError("status_scene_changed")
            }
            let underCursor = child.contains(cursor) ? childID : root.contains(cursor) ? rootID : nil
            guard underCursor == hover else { throw ReadoutError("status_cursor_left_target_\(name)") }
            let image = try captureIsolatedWindow(windowID: rootID, frame: root, output: output, stage: name + "-root")
            let capsule = try captureIsolatedWindow(windowID: childID, frame: child, output: output, stage: name + "-child")
            guard let top = image.cropping(to: try pixelRect(header, root, image)),
                  let besideToolbar = image.cropping(to: try pixelRect(row, root, image)) else { throw ReadoutError("status_crop_missing") }
            rootWords = try readStatus(in: top)
            rowWords = hover == nil ? [] : try readStatus(in: besideToolbar)
            let language = CGRect(x: child.minX + 32, y: child.minY + 4, width: child.width - 52, height: child.height - 8)
            let glyph = try glyphWitness(in: capsule, frame: child, crop: language)
            childVisible = glyph.visible
            // WindowServer can represent an entirely transparent window as a
            // single pixel; glyphWitness verifies its alpha before we skip OCR.
            childWords = capsule.width == 1 && capsule.height == 1 && glyph.maxAlpha == 0 ? [] : try readStatus(in: capsule)
            if matches() { break }
            Thread.sleep(forTimeInterval: 0.08)
        } while ProcessInfo.processInfo.systemUptime < deadline
        stages.append(["stage": name, "fixtureStep": step, "pointerWindow": hover.map(String.init) ?? "outside",
                       "expectedRoot": expectedRoot.sorted(), "expectedChild": expectedChild.sorted(),
                       "rootText": rootWords.sorted(), "toolbarRowText": rowWords.sorted(), "controlText": childWords.sorted(),
                       "visibleLanguage": childVisible, "accepted": matches()])
        let result = "\(name) root=\(rootWords.sorted()) toolbarRow=\(rowWords.sorted()) capsule=\(childWords.sorted()) visible=\(childVisible)"
        guard matches() else { throw ReadoutError("status_readout_mismatch \(result)") }
        print("status_readout \(result)")
    }
    try pointer.move(outside)
    try advance(to: 0)
    try observe("status-quiet", step: 0)
    func hoverCaption(_ key: String) throws {
        guard let line = try caption(key), !line.isNull else { throw ReadoutError("default_\(key)_lane_not_visible") }
        let ink = try captureGlyphWitness(windowID: rootID, frame: root, crop: line, output: output, stage: "status-target-" + key)
        guard ink.visible, let body = ink.point, !(try compactBounds()).contains(body) else { throw ReadoutError("status_body_target_unavailable") }
        try pointer.move(body)
    }
    try hoverCaption("oldTranslation")
    try observe("status-body", step: 0, hover: rootID)
    try advance(to: 1)
    try observe("status-pending-body", step: 1, hover: rootID)
    let capsule = CGPoint(x: child.midX, y: child.midY)
    try pointer.move(capsule)
    try observe("status-pending-capsule", step: 1, hover: childID)
    try pointer.move(outside)
    try observe("status-pending-quiet", step: 1)
    try pointer.move(capsule)
    try observe("status-pending-reentry", step: 1, hover: childID)
    try advance(to: 2)
    try observe("status-settled-capsule", step: 2, hover: childID)
    try advance(to: 3)
    try advance(to: 4) // The shared projection now holds a draft translation with both pending flags off.
    try observe("status-streaming-capsule", step: 4, hover: childID)
    try hoverCaption("translation")
    try observe("status-streaming-body", step: 4, hover: rootID)
    try advance(to: 5)
    try observe("status-confirmed-body", step: 5, hover: rootID)
    try pointer.move(outside)
    try observe("status-restored-quiet", step: 5)
}
