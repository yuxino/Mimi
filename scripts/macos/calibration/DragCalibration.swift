import AppKit
import CoreGraphics
import Darwin

private let baseline = CGRect(x: 222, y: 394, width: 580, height: 240)
private let glyph = CGPoint(x: 509.5, y: 584.5)
private var uptime: TimeInterval { ProcessInfo.processInfo.systemUptime }
private func log(_ message: String) {
    let timestamp = String(format: "%.6f", uptime)
    FileHandle.standardOutput.write(Data("\(timestamp) \(message)\n".utf8))
}
private struct Failure: Error, CustomStringConvertible {
    let description: String
    init(_ message: String) { description = message }
}
private func require(_ success: Bool, _ message: String) throws {
    if !success { throw Failure(message) }
}
private func near(_ a: CGPoint, _ b: CGPoint) -> Bool {
    abs(a.x - b.x) <= 1 && abs(a.y - b.y) <= 1
}
private func windowBounds(_ id: CGWindowID, pid: pid_t) throws -> CGRect {
    guard let rows = CGWindowListCopyWindowInfo(.optionIncludingWindow, id) as? [[String: Any]],
          let row = rows.first(where: { ($0[kCGWindowNumber as String] as? NSNumber)?.uint32Value == id }),
          (row[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid,
          let bounds = row[kCGWindowBounds as String] as? [String: Any],
          let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { throw Failure("Calibration window unavailable") }
    return rect
}

@MainActor private func appKitPoint(_ point: CGPoint) throws -> CGPoint {
    let display = CGDisplayBounds(CGMainDisplayID())
    guard let zero = NSScreen.screens.first, display.size == zero.frame.size else { throw Failure("Unmatched primary display") }
    return CGPoint(x: zero.frame.minX + point.x - display.minX, y: zero.frame.maxY - (point.y - display.minY))
}
@MainActor private func reportEvent(_ stage: String, _ event: NSEvent, in window: NSWindow) {
    let local = event.locationInWindow
    let root = try? windowBounds(CGWindowID(window.windowNumber), pid: getpid())
    log("app \(stage) type=\(event.type.rawValue) eventNumber=\(event.eventNumber) eventWindow=\(event.windowNumber) rootID=\(window.windowNumber) local=\(local) localToNSScreen=\(window.convertPoint(toScreen: local)) quartzCursor=\(String(describing: CGEvent(source: nil)?.location)) quartzRoot=\(String(describing: root))")
}
private final class TestWindow: NSWindow {
    override func sendEvent(_ event: NSEvent) {
        if [.leftMouseDown, .leftMouseDragged, .leftMouseUp].contains(event.type) { reportEvent("dispatch", event, in: self) }
        super.sendEvent(event)
    }
}
private final class DragSurface: NSView {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func mouseDown(with event: NSEvent) {
        guard let window else { return }
        reportEvent("performDrag-enter", event, in: window)
        window.performDrag(with: event)
        reportEvent("performDrag-return", event, in: window)
    }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        dirtyRect.fill()
        NSColor.labelColor.setFill()
        NSBezierPath(ovalIn: CGRect(x: glyph.x - baseline.minX - 3, y: baseline.maxY - glyph.y - 3, width: 6, height: 6)).fill()
    }
}

@MainActor private func runFixture(ready: URL) throws {
    let app = NSApplication.shared
    if app.activationPolicy() != .regular { try require(app.setActivationPolicy(.regular), "Fixture cannot present a regular window") }
    app.finishLaunching()
    let bottomLeft = try appKitPoint(CGPoint(x: baseline.minX, y: baseline.maxY))
    let frame = CGRect(origin: bottomLeft, size: baseline.size)
    let window = TestWindow(contentRect: frame, styleMask: [.titled, .closable], backing: .buffered, defer: false)
    window.title = "AppKit drag calibration"
    window.contentView = DragSurface()
    window.isMovableByWindowBackground = false
    window.isRestorable = false
    window.setFrame(frame, display: true)
    window.makeKeyAndOrderFront(nil)
    app.activate()
    let id = CGWindowID(window.windowNumber)
    // The driver requests a reset only between fully observed, released gestures.
    signal(SIGUSR1, SIG_IGN)
    let reset = DispatchSource.makeSignalSource(signal: SIGUSR1, queue: .main)
    reset.setEventHandler { window.setFrame(frame, display: true); log("app reset id=\(id)") }
    reset.activate()
    let until = uptime + 3
    while (try? windowBounds(id, pid: getpid())) != baseline, uptime < until {
        RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.025))
    }
    let actual = try windowBounds(id, pid: getpid())
    try require(actual == baseline, "Fixture frame \(actual) differs from \(baseline)")
    log("app ready pid=\(getpid()) id=\(id) quartzRoot=\(actual) content=\(window.contentRect(forFrameRect: window.frame))")
    try String(id).write(to: ready, atomically: true, encoding: .utf8)
    withExtendedLifetime((window, reset)) { app.run() }
}

@MainActor private func runDriver(id: CGWindowID, pid: pid_t) throws {
    let app = NSApplication.shared
    if app.activationPolicy() != .prohibited { try require(app.setActivationPolicy(.prohibited), "Observer cannot disable activation") }
    app.finishLaunching()
    guard !app.isActive, app.windows.isEmpty, CGPreflightPostEventAccess(),
          let source = CGEventSource(stateID: .combinedSessionState), let initialCursor = CGEvent(source: nil)?.location
    else { throw Failure("Observer must stay inactive and have existing PostEvent access") }
    var pointer = initialCursor
    func post(_ type: CGEventType, _ point: CGPoint) throws {
        guard let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point, mouseButton: .left) else {
            throw Failure("Unable to create pointer event")
        }
        if type != .mouseMoved { event.setIntegerValueField(.mouseEventClickState, value: 1) }
        if type == .leftMouseDragged {
            event.setIntegerValueField(.mouseEventDeltaX, value: Int64((point.x - pointer.x).rounded()))
            event.setIntegerValueField(.mouseEventDeltaY, value: Int64((point.y - pointer.y).rounded()))
        }
        event.post(tap: .cghidEventTap)
        pointer = point
    }
    func down() -> Bool { CGEventSource.buttonState(.combinedSessionState, button: .left) }
    func wait(_ message: String, timeout: TimeInterval = 2, _ predicate: () throws -> Bool) throws {
        let until = uptime + timeout
        repeat {
            if try predicate() { return }
            Thread.sleep(forTimeInterval: 0.01)
        } while uptime < until
        throw Failure(message)
    }
    func sample(_ stage: String) throws -> CGRect {
        let frame = try windowBounds(id, pid: pid)
        log("driver \(stage) held=\(down()) quartzCursor=\(String(describing: CGEvent(source: nil)?.location)) quartzRoot=\(frame) delta=(\(frame.minX - baseline.minX),\(frame.minY - baseline.minY))")
        return frame
    }
    try require(!down(), "Mouse button already down")
    defer { if !down() { try? post(.mouseMoved, initialCursor) } }
    try require(CGDisplayBounds(CGMainDisplayID()).contains(baseline.offsetBy(dx: 48, dy: 32)), "Primary display cannot fit the calibration")
    for (index, stage) in ["body-click", "body-drag", "titlebar-click", "titlebar-drag"].enumerated() {
        let drag = stage.hasSuffix("-drag")
        let point = stage.hasPrefix("body-") ? glyph : CGPoint(x: glyph.x, y: baseline.minY + 12)
        if index > 0, try windowBounds(id, pid: pid) != baseline {
            try require(kill(pid, SIGUSR1) == 0, "Fixture no longer running")
            try wait("Failed to restore the calibration window") { try windowBounds(id, pid: pid) == baseline }
        }
        try require(try windowBounds(id, pid: pid) == baseline && !down(), "Unexpected pre-gesture state")
        try post(.mouseMoved, point)
        let expectedNS = try appKitPoint(point)
        try wait("Pointer has not hit the calibration window at \(point)") {
            guard let cursor = CGEvent(source: nil)?.location else { return false }
            return near(point, cursor) && near(expectedNS, NSEvent.mouseLocation) &&
                NSWindow.windowNumber(at: expectedNS, belowWindowWithWindowNumber: 0) == Int(id) &&
                NSWindow.windowNumber(at: NSEvent.mouseLocation, belowWindowWithWindowNumber: 0) == Int(id)
        }
        _ = try sample("\(stage) hit=\(id) at=\(point)")
        try post(.leftMouseDown, point)
        var released = false
        defer { if !released { try? post(.leftMouseUp, pointer) } }
        try wait("Quartz did not register mouse-down") { down() }
        _ = try sample("\(stage) down")
        Thread.sleep(forTimeInterval: drag ? 0.10 : 0.065)
        _ = try sample("\(stage) held-before-motion")
        if drag {
            for step in 1...8 {
                try require(down(), "Unexpected release before step \(step)")
                try post(.leftMouseDragged, CGPoint(x: point.x + CGFloat(step * 6), y: point.y + CGFloat(step * 4)))
                Thread.sleep(forTimeInterval: 0.035)
                _ = try sample("\(stage) step-\(step)")
            }
            Thread.sleep(forTimeInterval: 0.10)
        }
        try require(down(), "Unexpected release before mouse-up")
        let held = try sample("\(stage) held")
        try post(.leftMouseUp, pointer)
        released = true
        try wait("Quartz did not register mouse-up") { !down() }
        _ = try sample("\(stage) released")
        Thread.sleep(forTimeInterval: 0.1)
        _ = try sample("\(stage) released+100ms")
        Thread.sleep(forTimeInterval: 0.3)
        let stable = try sample("\(stage) released+400ms")
        let expected = drag ? baseline.offsetBy(dx: 48, dy: 32) : baseline
        log("result \(stage) heldMatchesReference=\(held == expected) releasedMatchesReference=\(stable == expected) expected=\(expected)")
        Thread.sleep(forTimeInterval: max(0.0, NSEvent.doubleClickInterval - 0.4) + 0.05)
    }
}

@main @MainActor private enum Calibration {
    static func main() {
        do {
            try require(ProcessInfo.processInfo.environment["RUNNER_ENVIRONMENT"] == "github-hosted", "Calibration requires an isolated GitHub-hosted desktop")
            let args = Array(CommandLine.arguments.dropFirst())
            if args.count == 2, args[0] == "window" { try runFixture(ready: URL(fileURLWithPath: args[1])); return }
            guard args.count == 3, args[0] == "driver", let pid = pid_t(args[1]), pid > 0,
                  let id = CGWindowID(args[2]), id != kCGNullWindowID else { throw Failure("Expected window READY or driver PID WINDOW_ID") }
            try runDriver(id: id, pid: pid)
        } catch {
            FileHandle.standardError.write(Data("Calibration failed: \(error)\n".utf8))
            exit(1)
        }
    }
}
