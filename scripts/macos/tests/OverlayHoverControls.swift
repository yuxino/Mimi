import ApplicationServices
import Foundation

private struct HoverError: Error, CustomStringConvertible {
    let description: String
    init(_ label: String) { description = label }
}

/// The runner owns this isolated signed app and its synthetic listening session.
/// Keep real pointer input and each window's framebuffer as separate witnesses:
/// WebKit can expose a hidden button to AX while its pixels remain transparent.
func verifyOverlayHoverControls(pid: pid_t, rootID: CGWindowID, root: CGRect,
                                childID: CGWindowID, compact: CGRect, caption: CGRect,
                                fixture: URL, output: URL) throws {
    func clock() -> TimeInterval { ProcessInfo.processInfo.systemUptime }
    func attr(_ node: AXUIElement, _ name: String) -> CFTypeRef? {
        var value: CFTypeRef?
        return AXUIElementCopyAttributeValue(node, name as CFString, &value) == .success ? value : nil
    }
    func bounds(_ node: AXUIElement) -> CGRect? {
        guard let p = attr(node, kAXPositionAttribute), CFGetTypeID(p) == AXValueGetTypeID(),
              let s = attr(node, kAXSizeAttribute), CFGetTypeID(s) == AXValueGetTypeID() else { return nil }
        var point = CGPoint.zero, size = CGSize.zero
        guard AXValueGetValue(p as! AXValue, .cgPoint, &point), AXValueGetValue(s as! AXValue, .cgSize, &size),
              size.width > 0, size.height > 0 else { return nil }
        return CGRect(origin: point, size: size)
    }
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
    func windowAX(_ expected: CGRect) throws -> AXUIElement {
        let matches = (attr(app, kAXWindowsAttribute) as? [AXUIElement] ?? []).filter {
            bounds($0).map { same($0, expected) } == true
        }
        guard matches.count == 1 else { throw HoverError("hover_ax_window_not_unique") }
        return matches[0]
    }
    func controls(_ window: CGRect) throws -> [(name: String, role: String, rect: CGRect)] {
        var pending = [(try windowAX(window), 0)], result: [(String, String, CGRect)] = [], visited = 0
        while let (node, depth) = pending.popLast() {
            visited += 1
            guard visited <= 2048 else { throw HoverError("hover_ax_tree_limit") }
            let role = attr(node, kAXRoleAttribute) as? String ?? ""
            if role == kAXButtonRole || role == kAXComboBoxRole, let rect = bounds(node), window.contains(rect) {
                for name in Set([kAXTitleAttribute, kAXDescriptionAttribute].compactMap({ attr(node, $0) as? String })) {
                    result.append((name, role, rect))
                }
            }
            if depth < 20 { pending.append(contentsOf: (attr(node, kAXChildrenAttribute) as? [AXUIElement] ?? []).map { ($0, depth + 1) }) }
        }
        return result
    }
    let pause = try controls(root).filter { $0.role == kAXButtonRole && $0.name == "Pause translating" }
    guard pause.count == 1, caption.height >= 8, root.contains(caption), same(try surface(), compact),
          let savedCursor = CGEvent(source: nil)?.location,
          let source = CGEventSource(stateID: .combinedSessionState),
          !CGEventSource.buttonState(.combinedSessionState, button: .left) else {
        throw HoverError("hover_compact_caption_or_idle_pointer_unavailable")
    }
    var holding = false
    func post(_ type: CGEventType, _ point: CGPoint) throws {
        guard let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point, mouseButton: .left) else {
            throw HoverError("hover_pointer_event_unavailable")
        }
        event.post(tap: .cghidEventTap)
    }
    func move(_ point: CGPoint) throws {
        guard let start = CGEvent(source: nil)?.location else { throw HoverError("hover_cursor_unavailable") }
        for step in 1...6 {
            let fraction = CGFloat(step) / 6
            try post(.mouseMoved, CGPoint(x: start.x + (point.x - start.x) * fraction, y: start.y + (point.y - start.y) * fraction))
            Thread.sleep(forTimeInterval: 0.014)
        }
        guard let actual = CGEvent(source: nil)?.location, abs(actual.x - point.x) < 2, abs(actual.y - point.y) < 2 else {
            throw HoverError("hover_cursor_did_not_reach_target")
        }
    }
    func click(_ point: CGPoint, window: CGRect) throws {
        try move(point)
        var hit: AXUIElement?, owner: pid_t = -1
        guard window.contains(point),
              AXUIElementCopyElementAtPosition(AXUIElementCreateSystemWide(), Float(point.x), Float(point.y), &hit) == .success,
              let hit, AXUIElementGetPid(hit, &owner) == .success, owner == pid,
              let raw = attr(hit, kAXWindowAttribute), CFGetTypeID(raw) == AXUIElementGetTypeID(),
              bounds(raw as! AXUIElement).map({ same($0, window) }) == true else {
            throw HoverError("hover_physical_click_not_in_expected_child")
        }
        try post(.leftMouseDown, point)
        holding = true
        Thread.sleep(forTimeInterval: 0.06)
        try post(.leftMouseUp, point)
        holding = false
    }
    defer {
        if holding, let point = CGEvent(source: nil)?.location { try? post(.leftMouseUp, point) }
        try? move(savedCursor)
    }
    var evidence: [[String: Any]] = []
    var passed = false
    defer {
        let result: [String: Any] = ["passed": passed, "rootWindowID": rootID, "controlWindowID": childID,
                                     "root": json(root), "compact": json(compact), "stages": evidence,
                                     "sampling": "Isolated -a native window pixels and physical pointer clicks; not continuous video"]
        if let data = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys]) {
            try? data.write(to: output.appendingPathComponent("overlay-hover-controls.json"), options: .atomic)
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

    try move(outside)
    Thread.sleep(forTimeInterval: 0.6)
    try pair("hover-quiet", visible: false)
    let glyph = try captureGlyphWitness(windowID: rootID, frame: root, crop: caption, output: output, stage: "hover-caption-target")
    guard glyph.visible, let point = glyph.point else { throw HoverError("hover_caption_target_has_no_painted_glyph") }
    try move(point)
    Thread.sleep(forTimeInterval: 0.5)
    // Do not enter the child first: that would hide a broken peer-hover path.
    try pair("hover-caption", visible: true)
    let target = CGPoint(x: compact.midX, y: compact.midY)
    try move(target)
    Thread.sleep(forTimeInterval: 0.3)
    try pair("hover-handoff", visible: true)
    try click(target, window: compact) // Exactly one physical click, with no AX press or retry.
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
                                              stage: field.name == "Recognize" ? "hover-panel-source" : "hover-panel-target")
        languages.append(["name": field.name, "bounds": json(field.rect), "pixels": painted.json])
        guard painted.visible else { throw HoverError("opened_language_field_has_no_painted_value_\(field.name)") }
    }
    evidence.append(["stage": "first-physical-open", "uptime": clock(), "bounds": json(expanded), "languages": languages])
    try click(CGPoint(x: collapse.midX, y: collapse.midY), window: expanded)
    let collapseDeadline = clock() + 4
    while !same(try surface(), compact), clock() < collapseDeadline { Thread.sleep(forTimeInterval: 0.05) }
    guard same(try surface(), compact) else { throw HoverError("hover_panel_did_not_restore_exact_compact_bounds") }
    try move(outside)
    Thread.sleep(forTimeInterval: 0.6)
    try pair("hover-restored-quiet", visible: false)
    passed = true
    print("overlay_hover_controls quiet=true peerPainted=true handoffPainted=true firstPhysicalOpen=true languageFieldsPainted=2 compactRestored=true settingsClosed=true")
}
