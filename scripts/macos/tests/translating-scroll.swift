import AppKit
import ApplicationServices

private struct LayoutError: Error, CustomStringConvertible {
    let description: String
    init(_ label: String) { description = label }
}

private var now: TimeInterval { ProcessInfo.processInfo.systemUptime }
private let tolerance: CGFloat = 0.5
private let settingsTitle = "mimi UI test settings"
private let names = [
    "oldSource": "History 06 rests beside the quiet river.",
    "oldTranslation": "旧句 06 静静地停在河边。",
    "source": "The next boat leaves at noon.",
    "translation": "下一班船在中午出发。",
]
private let historyNames = names.merging(["firstSource": "History 01 rests beside the quiet river.",
                                         "firstTranslation": "旧句 01 静静地停在河边。"]) { first, _ in first }

private func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}

private func frame(of element: AXUIElement) -> CGRect? {
    guard let p = attribute(element, kAXPositionAttribute), CFGetTypeID(p) == AXValueGetTypeID(),
          let s = attribute(element, kAXSizeAttribute), CFGetTypeID(s) == AXValueGetTypeID() else { return nil }
    var position = CGPoint.zero
    var size = CGSize.zero
    guard AXValueGetValue(p as! AXValue, .cgPoint, &position), AXValueGetValue(s as! AXValue, .cgSize, &size),
          size.width > 0, size.height > 0 else { return nil }
    return CGRect(origin: position, size: size)
}

private func equal(_ a: CGRect, _ b: CGRect) -> Bool {
    abs(a.minX - b.minX) <= tolerance && abs(a.minY - b.minY) <= tolerance &&
        abs(a.width - b.width) <= tolerance && abs(a.height - b.height) <= tolerance
}

private func rect(_ value: CGRect) -> [String: CGFloat] {
    ["x": value.minX, "y": value.minY, "width": value.width, "height": value.height]
}

private struct Window {
    let id: CGWindowID
    let title: String
    let bounds: CGRect
}

private func windows(pid: pid_t) throws -> [Window] {
    guard let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
        throw LayoutError("window_geometry_unavailable")
    }
    return rows.compactMap { row in
        guard (row[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid,
              let id = row[kCGWindowNumber as String] as? NSNumber,
              let raw = row[kCGWindowBounds as String] as? NSDictionary,
              let bounds = CGRect(dictionaryRepresentation: raw) else { return nil }
        return Window(id: id.uint32Value, title: row[kCGWindowName as String] as? String ?? "", bounds: bounds)
    }
}

private func closeSettings(pid: pid_t, app: AXUIElement) throws {
    let deadline = now + 8
    repeat {
        let nodes = attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []
        let matching = nodes.filter { attribute($0, kAXTitleAttribute) as? String == settingsTitle }
        if matching.count == 1,
           let close = attribute(matching[0], kAXCloseButtonAttribute), CFGetTypeID(close) == AXUIElementGetTypeID() {
            guard AXUIElementPerformAction(close as! AXUIElement, kAXPressAction as CFString) == .success else {
                throw LayoutError("settings_close_rejected")
            }
            let hiddenDeadline = now + 5
            repeat {
                if try !windows(pid: pid).contains(where: { $0.title == settingsTitle }) { return }
                Thread.sleep(forTimeInterval: 0.025)
            } while now < hiddenDeadline
            throw LayoutError("settings_remained_visible")
        }
        Thread.sleep(forTimeInterval: 0.05)
    } while now < deadline
    throw LayoutError("fixture_settings_not_found")
}

private struct Ack: Decodable {
    let step: Int
    let pending: Bool
    let phase: String
    let historyCount: Int
    let hasTranslation: Bool
    let confirmed: Bool

    func matches(_ expected: Int) -> Bool {
        step == expected && phase == "listening" && pending == (expected == 1 || expected == 3) &&
            historyCount == (expected == 5 ? 7 : 6) && hasTranslation == (expected >= 4) && confirmed == (expected == 5)
    }
}

private struct Lane {
    let bounds: CGRect
    let viewport: CGRect
    let scrollAreaFound: Bool
    var clipped: CGRect { bounds.intersection(viewport) }
    var full: Bool { !clipped.isNull && equal(clipped, bounds) }
    var json: [String: Any] {
        ["bounds": rect(bounds), "viewport": rect(viewport), "visible": clipped.isNull ? [:] : rect(clipped),
         "full": full, "scrollAreaFound": scrollAreaFound]
    }
}

private struct Sample {
    let started: TimeInterval
    let finished: TimeInterval
    let lanes: [String: Lane]
    let translating: Bool
    let backToLive: AXUIElement?

    var json: [String: Any] {
        ["started": started, "finished": finished, "translatingVisible": translating,
         "lanes": lanes.mapValues(\.json)]
    }
}

private func sample(_ root: AXUIElement, bounds: CGRect, inspectHistory: Bool = false) throws -> Sample {
    let started = now
    var pending: [(AXUIElement, Int, CGRect, Bool)] = [(root, 0, bounds, false)]
    var lanes: [String: Lane] = [:]
    var translating = false
    var backToLive: AXUIElement?
    let expected = inspectHistory ? historyNames : names
    var visited = 0
    while let (element, depth, inheritedClip, inheritedScroll) = pending.popLast() {
        visited += 1
        guard visited <= 2048 else { throw LayoutError("caption_ax_tree_limit") }
        let elementFrame = frame(of: element)
        let role = attribute(element, kAXRoleAttribute) as? String
        let scrolling = role == kAXScrollAreaRole
        let clip = scrolling ? inheritedClip.intersection(elementFrame ?? inheritedClip) : inheritedClip
        let scrollFound = inheritedScroll || scrolling
        let labels = [kAXValueAttribute, kAXTitleAttribute, kAXDescriptionAttribute].compactMap {
            attribute(element, $0) as? String
        }
        if labels.contains("Translating"), let elementFrame, !elementFrame.intersection(clip).isNull { translating = true }
        if inspectHistory, role == kAXButtonRole, labels.contains("Back to live"), let elementFrame, clip.intersects(elementFrame) {
            backToLive = element
        }
        for (key, text) in expected where labels.contains(text) {
            if let elementFrame, !elementFrame.intersection(clip).isNull {
                let candidate = Lane(bounds: elementFrame, viewport: clip, scrollAreaFound: scrollFound)
                if lanes[key].map({ elementFrame.width * elementFrame.height < $0.bounds.width * $0.bounds.height }) ?? true {
                    lanes[key] = candidate
                }
            }
        }
        if depth < 20, let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement] {
            pending.append(contentsOf: children.map { ($0, depth + 1, clip, scrollFound) })
        }
    }
    return Sample(started: started, finished: now, lanes: lanes, translating: translating, backToLive: backToLive)
}

@main @MainActor
private enum TranslatingScroll {
    static func main() {
        do { try run() } catch {
            FileHandle.standardError.write(Data("Translating scroll failed: \(error)\n".utf8))
            exit(1)
        }
    }

    static func run() throws {
        guard ProcessInfo.processInfo.environment["RUNNER_ENVIRONMENT"] == "github-hosted",
              AXIsProcessTrusted(), CGPreflightPostEventAccess(), CGPreflightScreenCaptureAccess() else {
            throw LayoutError("isolated_host_and_preapproved_permissions_required")
        }
        let args = CommandLine.arguments
        let bundles = ["app.yuxino.mimi": "mimi.app", "app.yuxino.mimi.dev": "mimi-dev.app"]
        let checkModes = args.count == 6 && args.last == "--display-modes"
        guard args.count == 5 || checkModes, let pid = pid_t(args[1]), pid > 0, let bundle = bundles[args[2]],
              let running = NSRunningApplication(processIdentifier: pid), running.bundleIdentifier == args[2],
              running.executableURL?.standardizedFileURL.path == "/Applications/\(bundle)/Contents/MacOS/mimi" else {
            throw LayoutError("expected_verified_app_pid_identifier_fixture_evidence")
        }
        let fixture = URL(fileURLWithPath: args[3], isDirectory: true)
        let output = URL(fileURLWithPath: args[4], isDirectory: true)
        guard fixture.lastPathComponent.hasPrefix("mimi-ui-test-"),
              fixture.resolvingSymlinksInPath().deletingLastPathComponent() == FileManager.default.temporaryDirectory.resolvingSymlinksInPath() else {
            throw LayoutError("fixture_directory_not_owned_temporary_scene")
        }
        let app = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(app, 0.25)
        try closeSettings(pid: pid, app: app)
        let suffix = args[2].hasSuffix(".dev") ? " (dev)" : ""
        let visible = try windows(pid: pid)
        let roots = visible.filter { $0.title == "Mimi Subtitles" + suffix }
        let islands = visible.filter { $0.title == "Mimi" + suffix }
        guard roots.count == 1, islands.count == 1 else { throw LayoutError("subtitle_window_pair_not_unique") }
        let root = roots[0], island = islands[0]
        let axWindows = attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []
        let axRoots = axWindows.filter { frame(of: $0).map { equal($0, root.bounds) } ?? false }
        guard axRoots.count == 1 else { throw LayoutError("caption_ax_root_not_unique") }
        guard let savedCursor = CGEvent(source: nil)?.location,
              !CGEventSource.buttonState(.combinedSessionState, button: .left) else { throw LayoutError("idle_pointer_unavailable") }
        func move(_ point: CGPoint) throws {
            guard let event = CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left) else {
                throw LayoutError("pointer_move_unavailable")
            }
            event.post(tap: .cghidEventTap)
        }
        defer { try? move(savedCursor) }
        let display = CGDisplayBounds(CGMainDisplayID())
        let outside = CGPoint(x: display.minX + 8, y: display.minY + 8)
        guard !root.bounds.contains(outside), !island.bounds.contains(outside) else { throw LayoutError("quiet_pointer_location_not_outside") }
        try move(outside)

        var trace: [[String: Any]] = []
        var checkpoints: [[String: Any]] = []
        var failures = Set<String>()
        var completed = false
        let origin = now
        func writeEvidence() throws {
            let json: [String: Any] = ["rootWindowID": root.id, "root": rect(root.bounds), "controlWindowID": island.id,
                "control": rect(island.bounds), "startedUptime": origin, "samples": trace, "checkpoints": checkpoints,
                "failures": failures.sorted(), "sampling": "AX geometry, not a continuous framebuffer recording"]
            try JSONSerialization.data(withJSONObject: json, options: [.prettyPrinted, .sortedKeys])
                .write(to: output.appendingPathComponent("translating-scroll.json"), options: .atomic)
        }
        defer {
            try? writeEvidence()
            if !completed, let current = try? windows(pid: pid).first(where: { $0.id == root.id }) {
                try? captureFrame(windowID: current.id, frame: current.bounds, crop: current.bounds, output: output, stage: "failed-window")
            }
        }
        func observe(step: Int) throws -> Sample {
            let current = try windows(pid: pid)
            guard !current.contains(where: { $0.title == settingsTitle }),
                  let r = current.first(where: { $0.id == root.id }), let c = current.first(where: { $0.id == island.id }),
                  equal(r.bounds, root.bounds), equal(c.bounds, island.bounds),
                  let cursor = CGEvent(source: nil)?.location, abs(cursor.x - outside.x) <= 1, abs(cursor.y - outside.y) <= 1,
                  !CGEventSource.buttonState(.combinedSessionState, button: .left) else {
                throw LayoutError("native_window_or_quiet_pointer_changed")
            }
            let value = try sample(axRoots[0], bounds: root.bounds.insetBy(dx: 12, dy: 12))
            var entry = value.json
            entry["requestedStep"] = step
            trace.append(entry)
            return value
        }
        func ack(_ step: Int) -> Bool {
            let path = fixture.appendingPathComponent("translation-layout-state.json")
            guard let data = try? Data(contentsOf: path), data.count <= 1024,
                  let state = try? JSONDecoder().decode(Ack.self, from: data) else { return false }
            return state.matches(step)
        }
        func painted(_ value: Sample, step: Int) -> Bool {
            value.translating == (step == 1 || step == 3) && value.lanes["source"]?.full == true &&
                (step < 4 ? value.lanes["translation"] == nil : value.lanes["translation"]?.full == true)
        }
        var baseline: Sample?
        var pairBeforeConfirmation: Sample?
        for step in 0...5 {
            let requestedAt = now
            if step > 0 {
                try Data().write(to: fixture.appendingPathComponent("translation-layout-step-\(step)"), options: .withoutOverwriting)
            }
            let deadline = now + 5
            var visibleSince: TimeInterval?
            var final: Sample?
            var count = 0
            var lastStarted: TimeInterval?
            var maximumGap: TimeInterval = 0
            var maximumCost: TimeInterval = 0
            repeat {
                let value = try observe(step: step)
                count += 1
                if let lastStarted { maximumGap = max(maximumGap, value.started - lastStarted) }
                lastStarted = value.started
                maximumCost = max(maximumCost, value.finished - value.started)
                if let baseline, step <= 3 {
                    for key in ["oldSource", "oldTranslation", "source"] {
                        if let before = baseline.lanes[key], let after = value.lanes[key] {
                            if !after.full || abs(after.bounds.minY - before.bounds.minY) > tolerance || abs(after.bounds.maxY - before.bounds.maxY) > tolerance {
                                failures.insert("pending_only_\(key)_moved_or_clipped_step_\(step)")
                            }
                        } else { failures.insert("pending_only_\(key)_missing_step_\(step)") }
                    }
                }
                if let baseline, step >= 4 {
                    for key in ["oldSource", "oldTranslation", "source"] {
                        if let before = baseline.lanes[key], let after = value.lanes[key], after.bounds.minY > before.bounds.minY + tolerance {
                            failures.insert("new_translation_\(key)_moved_down_step_\(step)")
                        }
                    }
                }
                if step == 5, let before = pairBeforeConfirmation {
                    for key in ["source", "translation"] {
                        if let a = before.lanes[key], let b = value.lanes[key], !equal(a.bounds, b.bounds) {
                            failures.insert("confirmation_\(key)_geometry_changed")
                        }
                    }
                }
                if ack(step), painted(value, step: step) {
                    visibleSince = visibleSince ?? now
                    if now - visibleSince! >= 0.4 { final = value; break }
                } else { visibleSince = nil }
                Thread.sleep(forTimeInterval: 0.025)
            } while now < deadline
            guard let final else {
                throw LayoutError("step_\(step)_not_accepted_and_visibly_stable_in_5s")
            }
            if step == 0 {
                guard ["oldSource", "oldTranslation", "source"].allSatisfy({ final.lanes[$0]?.full == true }) else {
                    throw LayoutError("baseline_old_anchor_and_draft_not_fully_visible")
                }
                baseline = final
            }
            if step == 4 { pairBeforeConfirmation = final }
            var checkpoint = final.json
            checkpoint["step"] = step
            checkpoint["requestedAt"] = requestedAt
            checkpoint["sampleCount"] = count
            checkpoint["maximumSampleGapMs"] = maximumGap * 1000
            checkpoint["maximumSampleCostMs"] = maximumCost * 1000
            checkpoints.append(checkpoint)
            try captureFrame(windowID: root.id, frame: root.bounds, crop: root.bounds, output: output, stage: "step-\(step)")
            print("translation_layout step=\(step) samples=\(trace.count) accepted=true pending=\(final.translating) elapsedMs=\(Int((now - requestedAt) * 1000)) failures=\(failures.count)")
        }
        try writeEvidence()
        guard failures.isEmpty else { throw LayoutError("caption_geometry_regression: " + failures.sorted().joined(separator: ",")) }
        guard let caption = pairBeforeConfirmation?.lanes["source"], caption.full else {
            throw LayoutError("hover_regression_caption_target_missing")
        }
        try verifyOverlayHoverControls(pid: pid, rootID: root.id, root: root.bounds,
                                       childID: island.id, compact: island.bounds, caption: caption.bounds,
                                       fixture: fixture, output: output)
        try verifyDisplayModes(pid: pid, window: root.id, bounds: root.bounds, fixture: fixture, output: output,
                               cycleModes: checkModes, observe: {
            let value = try sample(axRoots[0], bounds: root.bounds.insetBy(dx: 12, dy: 12), inspectHistory: true)
            return DisplayModeView(visible: value.lanes.mapValues(\.clipped), full: Set(value.lanes.filter { $0.value.full }.keys),
                                   backToLive: value.backToLive)
        }, afterPhysicalReturn: {
            try verifyOverlayHoverControls(pid: pid, rootID: root.id, root: root.bounds,
                                           childID: island.id, compact: island.bounds, caption: caption.bounds,
                                           fixture: fixture, output: output, entry: .fastAfterReturn)
        })
        completed = true
    }
}
