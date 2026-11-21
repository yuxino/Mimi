import ApplicationServices
import Carbon.HIToolbox
import Foundation

struct DisplayModeView {
    let visible: [String: CGRect]
    let full: Set<String>
    let backToLive: AXUIElement?
}

private struct ModeError: Error, CustomStringConvertible {
    let description: String
    init(_ message: String) { description = message }
}

private struct ModePreference: Decodable { let subtitle_display_mode: String }

/// Use the public global shortcut and WebKit's real scrolling and button actions.
/// The caller already owns this signed app, its private preferences and synthetic captions.
func verifyDisplayModes(pid: pid_t, window: CGWindowID, bounds: CGRect, fixture: URL, output: URL,
                        observe: () throws -> DisplayModeView) throws {
    func clock() -> TimeInterval { ProcessInfo.processInfo.systemUptime }
    func waitFor(_ stage: String, mode: String, _ accepts: (DisplayModeView) -> Bool) throws -> DisplayModeView {
        let deadline = clock() + 5
        var steady: TimeInterval?
        var actual = "", last: [String] = []
        repeat {
            let data = try Data(contentsOf: fixture.appendingPathComponent("preferences.json"))
            actual = try JSONDecoder().decode(ModePreference.self, from: data).subtitle_display_mode
            let view = try observe()
            last = view.visible.keys.sorted()
            if actual == mode && accepts(view) {
                steady = steady ?? clock()
                if clock() - steady! >= 0.2 {
                    try captureFrame(windowID: window, frame: bounds, crop: bounds, output: output, stage: "display-\(stage)")
                    print("display_mode stage=\(stage) saved=\(actual) full=\(view.full.sorted()) backToLive=\(view.backToLive != nil)")
                    return view
                }
            } else { steady = nil }
            Thread.sleep(forTimeInterval: 0.035)
        } while clock() < deadline
        throw ModeError("\(stage) did not settle: saved=\(actual), visible=\(last)")
    }

    guard let input = CGEventSource(stateID: .combinedSessionState) else { throw ModeError("No input event source") }
    var lastKey: TimeInterval = 0
    func nextMode() throws {
        // The product ignores repeat shortcuts within 500 ms.
        Thread.sleep(forTimeInterval: max(0, lastKey + 0.55 - clock()))
        guard let down = CGEvent(keyboardEventSource: input, virtualKey: CGKeyCode(kVK_ANSI_B), keyDown: true),
              let up = CGEvent(keyboardEventSource: input, virtualKey: CGKeyCode(kVK_ANSI_B), keyDown: false) else {
            throw ModeError("Could not produce the display shortcut")
        }
        down.flags = [.maskCommand, .maskShift]
        up.flags = down.flags
        lastKey = clock()
        down.post(tap: .cghidEventTap)
        Thread.sleep(forTimeInterval: 0.03)
        up.post(tap: .cghidEventTap)
    }

    try nextMode()
    _ = try waitFor("original", mode: "original") { view in
        view.full.contains("source") && ["translation", "oldTranslation", "firstTranslation"].allSatisfy { view.visible[$0] == nil }
    }
    try nextMode()
    _ = try waitFor("translation", mode: "translation") { view in
        view.full.contains("translation") && ["source", "oldSource", "firstSource"].allSatisfy { view.visible[$0] == nil }
    }
    try nextMode()
    let live: (DisplayModeView) -> Bool = { $0.full.isSuperset(of: ["source", "translation"]) && $0.backToLive == nil }
    let bilingual = try waitFor("bilingual", mode: "bilingual", live)

    guard let line = bilingual.visible["translation"] else { throw ModeError("Latest translated line missing") }
    let point = CGPoint(x: line.midX, y: line.midY)
    guard let move = CGEvent(mouseEventSource: input, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left),
          let wheel = CGEvent(scrollWheelEvent2Source: input, units: .pixel, wheelCount: 1,
                              wheel1: Int32(ceil(bounds.height * 4)), wheel2: 0, wheel3: 0) else {
        throw ModeError("Could not produce the history scroll")
    }
    move.post(tap: .cghidEventTap)
    var hit: AXUIElement?
    var hitPid: pid_t = -1
    guard AXUIElementCopyElementAtPosition(AXUIElementCreateSystemWide(), Float(point.x), Float(point.y), &hit) == .success,
          let hit, AXUIElementGetPid(hit, &hitPid) == .success, hitPid == pid else {
        throw ModeError("History scroll did not target the signed app")
    }
    wheel.location = point
    wheel.post(tap: .cghidEventTap)
    let history = try waitFor("history", mode: "bilingual") {
        $0.full.isSuperset(of: ["firstSource", "firstTranslation"]) && $0.visible["source"] == nil && $0.backToLive != nil
    }
    guard let button = history.backToLive, AXUIElementPerformAction(button, kAXPressAction as CFString) == .success else {
        throw ModeError("Could not press Back to live")
    }
    _ = try waitFor("returned", mode: "bilingual") { live($0) && $0.visible["firstSource"] == nil }
}
