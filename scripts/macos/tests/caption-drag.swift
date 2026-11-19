import AppKit
import ApplicationServices

private struct DragCheckError: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

private let settingsTitle = "mimi UI test settings"
private let tolerance: CGFloat = 0.5 // Window Server reports logical points.
private var now: TimeInterval { ProcessInfo.processInfo.systemUptime }

private struct OwnedWindow {
    let id: CGWindowID
    let title: String
    let frame: CGRect

    var description: String { "id=\(id) title=\(String(reflecting: title)) frame=\(frame)" }
}

private func visibleWindows(of pid: pid_t) throws -> [OwnedWindow] {
    guard let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
        throw DragCheckError("Could not read Window Server geometry.")
    }
    return rows.compactMap { row in
        guard (row[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid,
              let id = row[kCGWindowNumber as String] as? NSNumber,
              let raw = row[kCGWindowBounds as String] as? NSDictionary,
              let frame = CGRect(dictionaryRepresentation: raw) else { return nil }
        return OwnedWindow(id: id.uint32Value, title: row[kCGWindowName as String] as? String ?? "", frame: frame)
    }
}

private func axAttribute(_ element: AXUIElement, _ attribute: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success ? value : nil
}

private struct AXHit {
    let status: AXError
    var elementPID: pid_t?
    var windowPID: pid_t?
    var windowFrame: CGRect?

    init(at point: CGPoint, expectedPID: pid_t) {
        var element: AXUIElement?
        status = AXUIElementCopyElementAtPosition(AXUIElementCreateSystemWide(), Float(point.x), Float(point.y), &element)
        var owner: pid_t = 0
        guard status == .success, let element, AXUIElementGetPid(element, &owner) == .success else { return }
        elementPID = owner
        guard owner == expectedPID else { return } // Do not inspect another app's accessibility tree.
        let window: AXUIElement
        if let value = axAttribute(element, kAXWindowAttribute), CFGetTypeID(value) == AXUIElementGetTypeID() {
            window = value as! AXUIElement
        } else if axAttribute(element, kAXRoleAttribute) as? String == kAXWindowRole {
            window = element
        } else { return }
        guard AXUIElementGetPid(window, &owner) == .success else { return }
        windowPID = owner
        guard owner == expectedPID,
              let position = axAttribute(window, kAXPositionAttribute), CFGetTypeID(position) == AXValueGetTypeID(),
              let size = axAttribute(window, kAXSizeAttribute), CFGetTypeID(size) == AXValueGetTypeID() else { return }
        var origin = CGPoint.zero
        var extent = CGSize.zero
        if AXValueGetValue(position as! AXValue, .cgPoint, &origin), AXValueGetValue(size as! AXValue, .cgSize, &extent) {
            windowFrame = CGRect(origin: origin, size: extent)
        }
    }

    func matches(pid: pid_t, frame: CGRect) -> Bool {
        guard elementPID == pid, windowPID == pid, let windowFrame else { return false }
        return abs(windowFrame.minX - frame.minX) <= tolerance && abs(windowFrame.minY - frame.minY) <= tolerance &&
            abs(windowFrame.width - frame.width) <= tolerance && abs(windowFrame.height - frame.height) <= tolerance
    }

    var description: String {
        "status=\(status.rawValue),pid=\(String(describing: elementPID)),windowPid=\(String(describing: windowPID)),frame=\(String(describing: windowFrame))"
    }
}

private func closeSettings(of pid: pid_t) throws -> CGWindowID {
    let app = AXUIElementCreateApplication(pid)
    let deadline = now + 8
    var lastObservation = ""
    repeat {
        let visible = try visibleWindows(of: pid)
        let settings = visible.filter { $0.title == settingsTitle }
        let windows = axAttribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []
        let matching = windows.filter { axAttribute($0, kAXTitleAttribute) as? String == settingsTitle }
        lastObservation = "AX windows=\(windows.count), AX Settings=\(matching.count), visible=\(visible.map(\.description))"
        if settings.count == 1, matching.count == 1,
           let button = axAttribute(matching[0], kAXCloseButtonAttribute), CFGetTypeID(button) == AXUIElementGetTypeID() {
            guard AXUIElementPerformAction(button as! AXUIElement, kAXPressAction as CFString) == .success else {
                throw DragCheckError("The fixture Settings close button did not accept AXPress.")
            }
            let id = settings[0].id
            let closeDeadline = now + 5
            repeat {
                if try !visibleWindows(of: pid).contains(where: { $0.id == id || $0.title == settingsTitle }) { return id }
                Thread.sleep(forTimeInterval: 0.025)
            } while now < closeDeadline
            throw DragCheckError("The fixture Settings window remained visible after AXPress.")
        }
        Thread.sleep(forTimeInterval: 0.05)
    } while now < deadline
    throw DragCheckError("Could not identify the fixture's visible Settings window and its AX close button: \(lastObservation)")
}

private struct WindowPair {
    let root: OwnedWindow
    let island: OwnedWindow

    static func read(pid: pid_t, settings: CGWindowID, titleSuffix: String) throws -> WindowPair {
        let windows = try visibleWindows(of: pid)
        guard !windows.contains(where: { $0.id == settings || $0.title == settingsTitle }) else {
            throw DragCheckError("Settings became visible during the caption gesture.")
        }
        let roots = windows.filter { $0.title == "Mimi Subtitles" + titleSuffix }
        let islands = windows.filter { $0.title == "Mimi" + titleSuffix }
        guard roots.count == 1, islands.count == 1, roots[0].id != islands[0].id else {
            throw DragCheckError("Could not identify the subtitle windows (roots=\(roots.count), islands=\(islands.count)); this PID's visible windows: \(windows.map(\.description)).")
        }
        return WindowPair(root: roots[0], island: islands[0])
    }

    func sameWindowsAndSizes(as reference: WindowPair) throws {
        guard root.id == reference.root.id, island.id == reference.island.id,
              root.frame.size == reference.root.frame.size, island.frame.size == reference.island.frame.size else {
            throw DragCheckError("The subtitle window or island changed identity or size: \(self)")
        }
    }

    func reached(_ delta: CGPoint, from reference: WindowPair) -> Bool {
        func matches(_ frame: CGRect, _ start: CGRect) -> Bool {
            abs(frame.minX - start.minX - delta.x) <= tolerance && abs(frame.minY - start.minY - delta.y) <= tolerance
        }
        return matches(root.frame, reference.root.frame) && matches(island.frame, reference.island.frame)
    }
}

@MainActor private final class Pointer {
    let source: CGEventSource
    let pid: pid_t
    var position: CGPoint
    static var pressed: Bool { CGEventSource.buttonState(.combinedSessionState, button: .left) }

    init(pid: pid_t) throws {
        // A command-line observer needs an AppKit connection for windowNumber(at:).
        let app = NSApplication.shared
        if app.activationPolicy() != .prohibited, !app.setActivationPolicy(.prohibited) {
            throw DragCheckError("Could not disable activation for the pointer observer.")
        }
        app.finishLaunching()
        guard !app.isActive, !app.windows.contains(where: \.isVisible) else {
            throw DragCheckError("The pointer observer activated or displayed a window.")
        }
        guard let source = CGEventSource(stateID: .combinedSessionState), let cursor = CGEvent(source: nil), !Self.pressed else {
            throw DragCheckError("Could not acquire an idle mouse event source.")
        }
        self.source = source
        self.pid = pid
        position = cursor.location
    }

    func post(_ type: CGEventType, at point: CGPoint) throws {
        guard let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point, mouseButton: .left) else {
            throw DragCheckError("Could not create a mouse event.")
        }
        if type != .mouseMoved { event.setIntegerValueField(.mouseEventClickState, value: 1) }
        if type == .leftMouseDragged {
            event.setIntegerValueField(.mouseEventDeltaX, value: Int64((point.x - position.x).rounded()))
            event.setIntegerValueField(.mouseEventDeltaY, value: Int64((point.y - position.y).rounded()))
        }
        event.post(tap: .cghidEventTap)
        position = point
    }

    private func verifyHit(at point: CGPoint, in root: OwnedWindow) throws {
        let deadline = now + 1
        var quartzCursor = CGPoint.zero
        while true {
            if let actual = CGEvent(source: nil)?.location, abs(actual.x - point.x) <= 1, abs(actual.y - point.y) <= 1 {
                quartzCursor = actual
                break
            }
            if now >= deadline { throw DragCheckError("The cursor did not reach the verified caption glyph at \(point).") }
            Thread.sleep(forTimeInterval: 0.01)
        }
        // Quartz and AX use a top-left origin. AppKit's first screen defines
        // the bottom-left origin, regardless of which screen has the key window.
        let display = CGDisplayBounds(CGMainDisplayID())
        guard let screen = NSScreen.screens.first, screen.frame.size == display.size else {
            throw DragCheckError("AppKit and Quartz do not agree on the primary display.")
        }
        let location = CGPoint(x: screen.frame.minX + quartzCursor.x - display.minX,
                               y: screen.frame.maxY - (quartzCursor.y - display.minY))
        let hit = NSWindow.windowNumber(at: location, belowWindowWithWindowNumber: 0)
        let ax = AXHit(at: quartzCursor, expectedPID: pid)
        guard hit == Int(root.id), ax.matches(pid: pid, frame: root.frame) else {
            throw DragCheckError("Caption hit at \(quartzCursor) expected root \(root.id) \(root.frame), PID \(pid). AppKit hit \(hit), AX \(ax.description).")
        }
        print("Caption hit at \(quartzCursor) verified root \(root.id), PID \(pid), frame \(root.frame).")
    }

    func hold(at point: CGPoint, in root: OwnedWindow, _ whileDown: () throws -> Void) throws {
        try verifyHit(at: point, in: root)
        guard !Self.pressed else { throw DragCheckError("The mouse was not idle before the caption press.") }
        try post(.leftMouseDown, at: point)
        var released = false
        defer { if !released { try? post(.leftMouseUp, at: position) } }
        try waitButton(true)
        try whileDown()
        guard Self.pressed else { throw DragCheckError("Quartz released the left button before the requested mouse-up.") }
        try post(.leftMouseUp, at: position)
        released = true
        try waitButton(false)
    }

    private func waitButton(_ expected: Bool) throws {
        let deadline = now + 1
        while Self.pressed != expected {
            if now >= deadline { throw DragCheckError("Quartz left-button state did not become \(expected).") }
            Thread.sleep(forTimeInterval: 0.01)
        }
    }
}

@main @MainActor
private enum CaptionDrag {
    static func main() {
        do { try run() } catch {
            FileHandle.standardError.write(Data("Caption drag failed: \(error)\n".utf8))
            exit(1)
        }
    }

    static func run() throws {
        guard ProcessInfo.processInfo.environment["RUNNER_ENVIRONMENT"] == "github-hosted",
              AXIsProcessTrusted(), CGPreflightPostEventAccess(), CGPreflightScreenCaptureAccess() else {
            throw DragCheckError("This check requires an isolated hosted desktop with Accessibility, PostEvent and screen capture already allowed.")
        }
        let args = CommandLine.arguments
        let bundles = ["app.yuxino.mimi": "mimi.app", "app.yuxino.mimi.dev": "mimi-dev.app"]
        guard args.count == 4, let pid = pid_t(args[1]), pid > 0, let bundle = bundles[args[2]],
              let process = NSRunningApplication(processIdentifier: pid), process.bundleIdentifier == args[2],
              process.executableURL?.standardizedFileURL.path == "/Applications/\(bundle)/Contents/MacOS/mimi" else {
            throw DragCheckError("Expected the PID and identifier of the verified Mimi installation, then an evidence directory.")
        }
        let titleSuffix = args[2] == "app.yuxino.mimi.dev" ? " (dev)" : ""
        let output = URL(fileURLWithPath: args[3], isDirectory: true)
        let settings = try closeSettings(of: pid)
        let readPair = { try WindowPair.read(pid: pid, settings: settings, titleSuffix: titleSuffix) }
        let initial = try readPair()
        guard initial.root.frame.contains(initial.island.frame), initial.island.frame.height < initial.root.frame.height / 2 else {
            throw DragCheckError("The language island is not in its compact position: \(initial).")
        }
        let sample = { () throws -> WindowPair in
            let pair = try readPair()
            try pair.sameWindowsAndSizes(as: initial)
            return pair
        }
        let pointer = try Pointer(pid: pid)
        let savedPointer = pointer.position
        defer { if !Pointer.pressed { try? pointer.post(.mouseMoved, at: savedPointer) } }
        print("Caption drag baseline pid=\(pid) root=\(initial.root.id) \(initial.root.frame), island=\(initial.island.id) \(initial.island.frame).")

        // Sample continuously through a dwell. A transient visit to the goal
        // cannot certify either a held drag or a released, stable position.
        func settled(_ delta: CGPoint, held: Bool, duration: TimeInterval, timeout: TimeInterval) throws -> Bool {
            let deadline = now + timeout
            var since: TimeInterval?
            repeat {
                let current = try sample()
                guard Pointer.pressed == held else { throw DragCheckError("Unexpected button state while observing native geometry.") }
                if current.reached(delta, from: initial) {
                    since = since ?? now
                    if now - since! >= duration { return true }
                } else { since = nil }
                Thread.sleep(forTimeInterval: 0.025)
            } while now < deadline
            return false
        }

        let viewport = CGDisplayBounds(CGMainDisplayID())
        guard viewport.contains(initial.root.frame.offsetBy(dx: 48, dy: 32)) else {
            throw DragCheckError("The fixture lacks room for the full 48 × 32 point movement.")
        }
        try pointer.post(.mouseMoved, at: CGPoint(x: initial.root.frame.midX, y: initial.root.frame.midY))
        guard try settled(.zero, held: false, duration: 0.25, timeout: 2) else { throw DragCheckError("The initial window geometry did not settle.") }

        func caption(_ pair: WindowPair, stage: String) throws -> (point: CGPoint, crop: CGRect) {
            let hit = try locateCaption(pid: pid, windowID: pair.root.id, frame: pair.root.frame, output: output, stage: stage)
            guard pair.root.frame.insetBy(dx: 12, dy: 12).contains(hit.point), hit.point.y > pair.island.frame.maxY,
                  !pair.island.frame.contains(hit.point) else { throw DragCheckError("The verified glyph is not clear of the window edges and controls.") }
            try pointer.post(.mouseMoved, at: hit.point)
            return hit
        }
        let click = try caption(initial, stage: "before-click")
        try captureFrame(windowID: initial.root.id, frame: initial.root.frame, crop: initial.root.frame, output: output, stage: "before-click-window")
        var clickStayed = true
        func observeClick(held: Bool, duration: TimeInterval) throws {
            let deadline = now + duration
            var first = true
            while true {
                let pair = try sample()
                guard held == Pointer.pressed else { throw DragCheckError("Unexpected button state during the caption click.") }
                let stationary = pair.reached(.zero, from: initial)
                let remaining = deadline - now
                if first || remaining <= 0 || (clickStayed && !stationary) {
                    print("Caption click held=\(held) cursor=\(String(describing: CGEvent(source: nil)?.location)) root=\(pair.root.frame) island=\(pair.island.frame).")
                }
                if !stationary { clickStayed = false }
                if remaining <= 0 { return }
                first = false
                Thread.sleep(forTimeInterval: min(0.025, remaining))
            }
        }
        try pointer.hold(at: click.point, in: initial.root) {
            try observeClick(held: true, duration: 0.065)
        }
        try observeClick(held: false, duration: max(0.4, NSEvent.doubleClickInterval))
        guard clickStayed else {
            let pair = try sample()
            try captureFrame(windowID: pair.root.id, frame: pair.root.frame, crop: pair.root.frame, output: output, stage: "after-click-window")
            throw DragCheckError("The subtitle window and island did not remain at their initial positions for the required click dwell.")
        }

        var failures: [String] = []
        for (stage, direction) in [("forward", 1), ("return", -1)] {
            let start = try sample()
            let glyph = try caption(start, stage: stage + "-start")
            let goal = direction == 1 ? CGPoint(x: 48, y: 32) : .zero
            func record(_ phase: String) throws {
                let pair = try sample()
                let dx = pair.root.frame.minX - start.root.frame.minX
                let dy = pair.root.frame.minY - start.root.frame.minY
                let cursor = CGEvent(source: nil)?.location
                print("Caption drag \(stage)-\(phase) held=\(Pointer.pressed) cursor=\(String(describing: cursor)) root=\(pair.root.frame) island=\(pair.island.frame).")
                if stage == "return", phase == "released" {
                    _ = try locateCaption(pid: pid, windowID: pair.root.id, frame: pair.root.frame, output: output, stage: "return-released")
                } else {
                    try captureFrame(windowID: pair.root.id, frame: pair.root.frame, crop: glyph.crop.offsetBy(dx: dx, dy: dy), output: output, stage: stage + "-" + phase)
                }
                if stage == "forward", phase == "released" {
                    try captureFrame(windowID: pair.root.id, frame: pair.root.frame, crop: pair.root.frame, output: output, stage: "forward-released-window")
                }
            }
            try pointer.hold(at: glyph.point, in: start.root) {
                // Allow the WebView to forward the actual mouse-down to AppKit
                // before delivering the native dragged events for this press.
                Thread.sleep(forTimeInterval: 0.10)
                for step in 1...8 {
                    guard Pointer.pressed else { throw DragCheckError("Left button lifted during the drag.") }
                    let point = CGPoint(x: glyph.point.x + CGFloat(direction * step * 6), y: glyph.point.y + CGFloat(direction * step * 4))
                    try pointer.post(.leftMouseDragged, at: point)
                    Thread.sleep(forTimeInterval: 0.035)
                    let current = try sample()
                    print("Caption drag \(stage) step=\(step) root=\(current.root.frame.origin) island=\(current.island.frame.origin).")
                }
                if try !settled(goal, held: true, duration: 0.1, timeout: 2) { failures.append(stage + " held position") }
                try record("held")
            }
            let stable = try settled(goal, held: false, duration: 0.4, timeout: 2)
            try record("released")
            guard stable else { throw DragCheckError("\(stage) did not reach and retain its expected geometry after release.") }
        }
        guard failures.isEmpty else { throw DragCheckError("Unverified native movement: \(failures.joined(separator: ", ")).") }
        print("Verified caption click, held movement, 400 ms stability and return to the original root and island frames.")
    }
}
