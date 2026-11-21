import AppKit
import ApplicationServices
import Foundation

private struct HoverError: Error, CustomStringConvertible {
    let description: String
    init(_ label: String) { description = label }
}

private func hoverAttribute(_ node: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(node, name as CFString, &value) == .success ? value : nil
}

func overlayTestFrame(_ node: AXUIElement) -> CGRect? {
    guard let p = hoverAttribute(node, kAXPositionAttribute), CFGetTypeID(p) == AXValueGetTypeID(),
          let s = hoverAttribute(node, kAXSizeAttribute), CFGetTypeID(s) == AXValueGetTypeID() else { return nil }
    var point = CGPoint.zero, size = CGSize.zero
    guard AXValueGetValue(p as! AXValue, .cgPoint, &point), AXValueGetValue(s as! AXValue, .cgSize, &size),
          size.width > 0, size.height > 0 else { return nil }
    return CGRect(origin: point, size: size)
}

struct OverlayTestPointer {
    private let source: CGEventSource
    private let saved: CGPoint

    init() throws {
        guard let source = CGEventSource(stateID: .combinedSessionState), let saved = CGEvent(source: nil)?.location,
              !CGEventSource.buttonState(.combinedSessionState, button: .left) else {
            throw HoverError("hover_idle_pointer_unavailable")
        }
        self.source = source
        self.saved = saved
    }

    func move(_ point: CGPoint, avoidingBeforeArrival windows: [CGRect] = []) throws {
        guard let start = CGEvent(source: nil)?.location else { throw HoverError("hover_cursor_unavailable") }
        let path = (1...6).map { step in
            let fraction = CGFloat(step) / 6
            return CGPoint(x: start.x + (point.x - start.x) * fraction, y: start.y + (point.y - start.y) * fraction)
        }
        guard path.dropLast().allSatisfy({ p in windows.allSatisfy { !$0.contains(p) } }) else {
            throw HoverError("fast_entry_crossed_overlay_before_child")
        }
        for point in path {
            guard let event = CGEvent(mouseEventSource: source, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left) else {
                throw HoverError("hover_pointer_move_unavailable")
            }
            event.post(tap: .cghidEventTap)
            Thread.sleep(forTimeInterval: 0.014)
        }
        guard let actual = CGEvent(source: nil)?.location, hypot(actual.x - point.x, actual.y - point.y) < 2 else {
            throw HoverError("hover_cursor_did_not_reach_target")
        }
    }

    /// Click where the pointer already is. In particular, never repair a failed
    /// fast entry by sending another mouseMoved event before its first click.
    func click(_ point: CGPoint, pid: pid_t, windowID: CGWindowID, window: CGRect,
               stage: String, output: URL, buttonName: String? = nil) throws {
        let cursor = CGEvent(source: nil)?.location
        // Nonactivating WebKit controls can omit AXWindow. Use WindowServer's
        // front-to-back list at the real Quartz pointer, plus the actual AX hit.
        // The command-line AppKit lookup has returned 0 for a painted, hittable
        // child; retain that independent reading without using it as the gate.
        let appKitPoint = NSEvent.mouseLocation
        let appKitNumber = NSWindow.windowNumber(at: appKitPoint, belowWindowWithWindowNumber: 0)
        let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]]
        let atPoint: [[String: Any]] = (rows ?? []).compactMap { row in
            guard let id = row[kCGWindowNumber as String] as? NSNumber,
                  let pid = row[kCGWindowOwnerPID as String] as? NSNumber,
                  let raw = row[kCGWindowBounds as String] as? NSDictionary,
                  let rect = CGRect(dictionaryRepresentation: raw), rect.contains(point),
                  let alpha = row[kCGWindowAlpha as String] as? NSNumber, alpha.doubleValue > 0 else { return nil }
            return ["windowID": id, "pid": pid, "alpha": alpha,
                    "bounds": ["x": rect.minX, "y": rect.minY, "width": rect.width, "height": rect.height]]
        }
        let top = (atPoint.first?["windowID"] as? NSNumber)?.uint32Value ?? kCGNullWindowID
        let topPID = (atPoint.first?["pid"] as? NSNumber)?.int32Value ?? -1
        var hit: AXUIElement?, owner: pid_t = -1
        let hitResult = AXUIElementCopyElementAtPosition(AXUIElementCreateSystemWide(), Float(point.x), Float(point.y), &hit)
        let ownerResult = hit.map { AXUIElementGetPid($0, &owner) }
        let role = hit.flatMap { hoverAttribute($0, kAXRoleAttribute) as? String } ?? ""
        var matches = role == kAXPopUpButtonRole || role == kAXButtonRole
        if let buttonName {
            // WebKit may hit the text inside Back to live; require its actual
            // button ancestor rather than accepting an unrelated caption node.
            matches = false
            var node = hit
            for _ in 0..<12 {
                guard let current = node else { break }
                if hoverAttribute(current, kAXRoleAttribute) as? String == kAXButtonRole,
                   [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute].contains(where: {
                       hoverAttribute(current, $0) as? String == buttonName
                   }) { matches = true; break }
                guard let parent = hoverAttribute(current, kAXParentAttribute), CFGetTypeID(parent) == AXUIElementGetTypeID() else { break }
                node = (parent as! AXUIElement)
            }
        }
        let accepted = window.contains(point) && cursor.map { hypot($0.x - point.x, $0.y - point.y) < 2 } == true &&
            top == windowID && topPID == pid && hitResult == .success && ownerResult == .success && owner == pid && matches &&
            !CGEventSource.buttonState(.combinedSessionState, button: .left)
        var evidence: [String: Any] = ["accepted": accepted, "expectedWindowID": windowID, "topWindowID": top,
            "expectedPID": pid, "hitPID": owner, "hitRole": role, "hitResult": hitResult.rawValue,
            "buttonMatched": matches, "target": ["x": point.x, "y": point.y],
            "windowsAtPointFrontToBack": Array(atPoint.prefix(8)), "windowListAvailable": rows != nil,
            "appKitWindowNumber": appKitNumber, "appKitCursor": ["x": appKitPoint.x, "y": appKitPoint.y],
            "cursor": cursor.map { ["x": $0.x, "y": $0.y] } ?? [:]]
        if let hit, let value = hoverAttribute(hit, kAXWindowAttribute), CFGetTypeID(value) == AXUIElementGetTypeID() {
            evidence["axWindowRole"] = hoverAttribute(value as! AXUIElement, kAXRoleAttribute) as? String ?? ""
        }
        func save() throws {
            try JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys])
                .write(to: output.appendingPathComponent(stage + "-click.json"), options: .atomic)
        }
        try save() // Keep the real top window / AX role even when a guard fails.
        guard accepted else { throw HoverError("\(stage)_physical_click_target_rejected") }
        guard let down = CGEvent(mouseEventSource: source, mouseType: .leftMouseDown, mouseCursorPosition: point, mouseButton: .left),
              let up = CGEvent(mouseEventSource: source, mouseType: .leftMouseUp, mouseCursorPosition: point, mouseButton: .left) else {
            throw HoverError("hover_pointer_click_unavailable")
        }
        evidence["downUptime"] = ProcessInfo.processInfo.systemUptime
        down.post(tap: .cghidEventTap)
        Thread.sleep(forTimeInterval: 0.06)
        up.post(tap: .cghidEventTap)
        evidence["upUptime"] = ProcessInfo.processInfo.systemUptime
        try save()
    }

    func restore() { try? move(saved) }
}

enum OverlayHoverEntry: String {
    case captionHandoff = "hover"
    case fastAfterReturn = "hover-fast-return"
}

/// The runner owns this isolated signed app and its synthetic listening session.
/// Keep real pointer input and each window's framebuffer as separate witnesses:
/// WebKit can expose a hidden button to AX while its pixels remain transparent.
func verifyOverlayHoverControls(pid: pid_t, rootID: CGWindowID, root: CGRect,
                                childID: CGWindowID, compact: CGRect, caption: CGRect,
                                fixture: URL, output: URL, entry: OverlayHoverEntry = .captionHandoff) throws {
    func clock() -> TimeInterval { ProcessInfo.processInfo.systemUptime }
    func same(_ a: CGRect, _ b: CGRect) -> Bool {
        abs(a.minX - b.minX) <= 0.5 && abs(a.minY - b.minY) <= 0.5 &&
            abs(a.width - b.width) <= 0.5 && abs(a.height - b.height) <= 0.5
    }
    func json(_ rect: CGRect) -> [String: CGFloat] {
        ["x": rect.minX, "y": rect.minY, "width": rect.width, "height": rect.height]
    }
    let app = AXUIElementCreateApplication(pid)
    AXUIElementSetMessagingTimeout(app, 0.25)
    func surface() throws -> CGRect {
        guard let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
            throw HoverError("hover_window_list_unavailable")
        }
        var own: [CGWindowID: CGRect] = [:]
        for row in rows where (row[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid {
            guard row[kCGWindowName as String] as? String != "mimi UI test settings",
                  let id = row[kCGWindowNumber as String] as? NSNumber,
                  let raw = row[kCGWindowBounds as String] as? NSDictionary,
                  let rect = CGRect(dictionaryRepresentation: raw) else { throw HoverError("hover_settings_or_window_geometry_changed") }
            own[id.uint32Value] = rect
        }
        guard own[rootID].map({ same($0, root) }) == true, let child = own[childID],
              !FileManager.default.fileExists(atPath: fixture.appendingPathComponent("settings-activation").path),
              (try? String(contentsOf: fixture.appendingPathComponent("session"), encoding: .utf8)) == "listening" else {
            throw HoverError("hover_root_settings_or_listening_changed")
        }
        return child
    }
    func controls(_ window: CGRect) throws -> [(name: String, role: String, rect: CGRect)] {
        let matches = (hoverAttribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []).filter {
            overlayTestFrame($0).map { same($0, window) } == true
        }
        guard matches.count <= 1 else { throw HoverError("hover_ax_window_not_unique") }
        // WindowServer can report an intermediate panel size before AX sees it.
        // The caller polls up to the original deadline for both painted fields.
        guard let matchingWindow = matches.first else { return [] }
        var pending = [(matchingWindow, 0)], result: [(String, String, CGRect)] = [], visited = 0
        while let (node, depth) = pending.popLast() {
            visited += 1
            guard visited <= 2048 else { throw HoverError("hover_ax_tree_limit") }
            let role = hoverAttribute(node, kAXRoleAttribute) as? String ?? ""
            if role == kAXButtonRole || role == kAXComboBoxRole, let rect = overlayTestFrame(node), window.contains(rect) {
                for name in Set([kAXTitleAttribute, kAXDescriptionAttribute].compactMap({ hoverAttribute(node, $0) as? String })) {
                    result.append((name, role, rect))
                }
            }
            if depth < 20 { pending.append(contentsOf: (hoverAttribute(node, kAXChildrenAttribute) as? [AXUIElement] ?? []).map { ($0, depth + 1) }) }
        }
        return result
    }
    let pause = try controls(root).filter { $0.role == kAXButtonRole && $0.name == "Pause translating" }
    guard pause.count == 1, caption.height >= 8, root.contains(caption), same(try surface(), compact) else {
        throw HoverError("hover_compact_caption_unavailable")
    }
    let pointer = try OverlayTestPointer()
    defer { pointer.restore() }
    let prefix = entry.rawValue
    var evidence: [[String: Any]] = []
    var passed = false
    defer {
        let result: [String: Any] = ["passed": passed, "entry": prefix, "rootWindowID": rootID, "controlWindowID": childID,
                                     "root": json(root), "compact": json(compact), "stages": evidence,
                                     "sampling": "Isolated -a native window pixels and physical pointer clicks; not continuous video"]
        if let data = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys]) {
            let name = entry == .captionHandoff ? "overlay-hover-controls.json" : "overlay-hover-fast-return.json"
            try? data.write(to: output.appendingPathComponent(name), options: .atomic)
        }
    }
    let screen = CGDisplayBounds(CGMainDisplayID())
    let outside = CGPoint(x: screen.minX + 8, y: screen.minY + 8)
    guard !root.contains(outside), !compact.contains(outside) else { throw HoverError("hover_outside_point_not_outside") }
    // Exclude button borders, the brand mark and the capsule's trailing chevron.
    let pauseCrop = pause[0].rect.insetBy(dx: 4, dy: 4)
    let capsuleCrop = CGRect(x: compact.minX + 32, y: compact.minY + 4,
                             width: compact.width - 52, height: compact.height - 8)
    func pair(_ stage: String, visible: Bool) throws {
        guard same(try surface(), compact) else { throw HoverError("hover_compact_geometry_changed") }
        let toolbar = try captureGlyphWitness(windowID: rootID, frame: root, crop: pauseCrop, output: output, stage: stage + "-root")
        let capsule = try captureGlyphWitness(windowID: childID, frame: compact, crop: capsuleCrop, output: output, stage: stage + "-child")
        evidence.append(["stage": stage, "uptime": clock(), "toolbar": toolbar.json, "capsule": capsule.json])
        guard toolbar.visible == visible, capsule.visible == visible else { throw HoverError("\(stage)_toolbar_and_child_not_\(visible ? "painted" : "hidden")") }
    }

    try pointer.move(outside)
    Thread.sleep(forTimeInterval: 0.6)
    try pair(prefix + "-quiet", visible: false)
    let target = CGPoint(x: compact.midX, y: compact.midY)
    switch entry {
    case .captionHandoff:
        let glyph = try captureGlyphWitness(windowID: rootID, frame: root, crop: caption, output: output, stage: "hover-caption-target")
        guard glyph.visible, let point = glyph.point else { throw HoverError("hover_caption_target_has_no_painted_glyph") }
        try pointer.move(point)
        Thread.sleep(forTimeInterval: 0.5)
        // Do not enter the child first: that would hide a broken peer-hover path.
        try pair("hover-caption", visible: true)
        try pointer.move(target)
        Thread.sleep(forTimeInterval: 0.3)
        try pair("hover-handoff", visible: true)
    case .fastAfterReturn:
        let moveStarted = clock()
        try pointer.move(target, avoidingBeforeArrival: [root, compact])
        evidence.append(["stage": prefix + "-move", "started": moveStarted, "arrived": clock(),
                         "from": ["x": outside.x, "y": outside.y], "to": ["x": target.x, "y": target.y],
                         "intermediatePointsOutsideBothWindows": true])
        Thread.sleep(forTimeInterval: 0.35)
        try pair(prefix + "-arrived", visible: true)
    }
    try pointer.click(target, pid: pid, windowID: childID, window: compact, stage: prefix + "-open", output: output)
    let deadline = clock() + 4
    var expanded: CGRect?, fields: [(name: String, role: String, rect: CGRect)] = [], collapse: CGRect?
    repeat {
        let current = try surface()
        if current.height > compact.height + 50 {
            let actual = try controls(current)
            fields = actual.filter { $0.role == kAXComboBoxRole && ["Recognize", "Translate To"].contains($0.name) }
            collapse = actual.first { $0.role == kAXButtonRole && $0.name.hasSuffix("Collapse subtitle controls") }?.rect
            if Set(fields.map(\.name)).count == 2, collapse != nil { expanded = current; break }
        }
        Thread.sleep(forTimeInterval: 0.05)
    } while clock() < deadline
    guard let expanded, let collapse else { throw HoverError("first_physical_click_did_not_open_both_language_fields") }
    Thread.sleep(forTimeInterval: 0.2)
    var languages: [[String: Any]] = []
    for field in fields {
        // Only the value area can pass; the dropdown arrow is outside this crop.
        let crop = CGRect(x: field.rect.minX + 4, y: field.rect.minY + 4,
                          width: field.rect.width - 28, height: field.rect.height - 8)
        let painted = try captureGlyphWitness(windowID: childID, frame: expanded, crop: crop, output: output,
                                              stage: prefix + (field.name == "Recognize" ? "-panel-source" : "-panel-target"))
        languages.append(["name": field.name, "bounds": json(field.rect), "pixels": painted.json])
        guard painted.visible else { throw HoverError("opened_language_field_has_no_painted_value_\(field.name)") }
    }
    evidence.append(["stage": "first-physical-open", "uptime": clock(), "bounds": json(expanded), "languages": languages])
    let collapsePoint = CGPoint(x: collapse.midX, y: collapse.midY)
    try pointer.move(collapsePoint)
    try pointer.click(collapsePoint, pid: pid, windowID: childID, window: expanded, stage: prefix + "-collapse", output: output)
    let collapseDeadline = clock() + 4
    while !same(try surface(), compact), clock() < collapseDeadline { Thread.sleep(forTimeInterval: 0.05) }
    guard same(try surface(), compact) else { throw HoverError("hover_panel_did_not_restore_exact_compact_bounds") }
    try pointer.move(outside)
    Thread.sleep(forTimeInterval: 0.6)
    try pair(prefix + "-restored-quiet", visible: false)
    passed = true
    print("overlay_hover_controls entry=\(prefix) quiet=true bothPainted=true firstPhysicalOpen=true languageFieldsPainted=2 compactRestored=true settingsClosed=true")
}
