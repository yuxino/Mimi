import AppKit
import ApplicationServices

private struct PanelButton {
    let element: AXUIElement
    let frame: CGRect
    let name: String

    var pressed: Bool? {
        for attribute in [kAXValueAttribute, kAXSelectedAttribute] {
            if let value = axAttribute(element, attribute) as? NSNumber { return value.boolValue }
        }
        return nil
    }
    var enabled: Bool { (axAttribute(element, kAXEnabledAttribute) as? NSNumber)?.boolValue == true }
}

// The island's accessible name ends with its action. Display buttons use
// visible short names; WebKit can expose a toggle button as a checkbox.
private func panelButtons(pid: pid_t, frame: CGRect, named: (String) -> Bool) throws -> [PanelButton] {
    let roles: Set<String> = [kAXButtonRole, kAXCheckBoxRole, kAXRadioButtonRole, kAXPopUpButtonRole]
    func name(_ element: AXUIElement) -> String? {
        for attribute in [kAXTitleAttribute, kAXDescriptionAttribute] {
            if let value = axAttribute(element, attribute) as? String, named(value) { return value }
        }
        return nil
    }
    let matches = try visibleAXMatches(pid: pid, frame: frame) { element in
        guard let role = axAttribute(element, kAXRoleAttribute) as? String, roles.contains(role) else { return false }
        return name(element) != nil
    }
    return matches.compactMap { match in
        name(match.element).map { PanelButton(element: match.element, frame: match.crop, name: $0) }
    }
}

func audioHeadingHelpFrame(pid: pid_t, panel: CGRect) throws -> CGRect {
    let buttons = try panelButtons(pid: pid, frame: panel) { $0 == "\(DragScene.panel.text) · Show help" }
    guard buttons.count == 1 else { throw DragCheckError("Could not isolate the Audio input heading's adjacent help button.") }
    return buttons[0].frame
}

@MainActor private func click(_ control: PanelButton, in window: OwnedWindow, pointer: Pointer) throws {
    let point = CGPoint(x: control.frame.midX, y: control.frame.midY)
    try pointer.post(.mouseMoved, at: point)
    try pointer.hold(at: point, in: window) { Thread.sleep(forTimeInterval: 0.065) }
}

@MainActor func openControlPanel(compact: WindowPair, pointer: Pointer, readPair: () throws -> WindowPair,
                                 output: URL) throws -> WindowPair {
    let island = compact.island
    try pointer.post(.mouseMoved, at: CGPoint(x: island.frame.midX, y: island.frame.midY))
    try captureFrame(windowID: island.id, frame: island.frame, crop: island.frame, output: output, stage: "compact-island")
    let end = ProcessInfo.processInfo.systemUptime + 4
    var opener: PanelButton?
    repeat {
        let pair = try readPair()
        try pair.sameWindowsAndSizes(as: compact)
        guard pair.reached(.zero, from: compact) else { throw DragCheckError("The compact windows moved before the click.") }
        let buttons = try panelButtons(pid: pointer.pid, frame: island.frame) { $0.hasSuffix("Open subtitle controls") }
        if buttons.count == 1, buttons[0].enabled { opener = buttons[0]; break }
        Thread.sleep(forTimeInterval: 0.04)
    } while ProcessInfo.processInfo.systemUptime < end
    guard let opener else { throw DragCheckError("Could not find the island's enabled Open subtitle controls button.") }
    try click(opener, in: island, pointer: pointer)

    let deadline = ProcessInfo.processInfo.systemUptime + 5
    var previous = island.frame, since = ProcessInfo.processInfo.systemUptime
    repeat {
        let pair = try readPair()
        guard pair.root.id == compact.root.id, pair.root.frame == compact.root.frame, pair.island.id == island.id else {
            throw DragCheckError("Opening the panel moved the subtitle window or replaced a window.")
        }
        let frame = pair.island.frame
        let now = ProcessInfo.processInfo.systemUptime
        if frame != previous { previous = frame; since = now }
        if frame.height > island.frame.height * 2, now - since >= 0.25 {
            try captureFrame(windowID: island.id, frame: frame, crop: frame, output: output, stage: "before-click-window")
            return pair
        }
        Thread.sleep(forTimeInterval: 0.025)
    } while ProcessInfo.processInfo.systemUptime < deadline
    throw DragCheckError("The native island did not expand and settle. Last frame \(previous).")
}

@MainActor func checkPanelDisplayButtons(pointer: Pointer, initial: WindowPair, sample: () throws -> WindowPair,
                                         output: URL) throws {
    // Check both states on each transition. A dropped click or an inaccessible
    // toggle state cannot pass by keeping the old selection.
    func waitForSelection(_ selected: String) throws -> PanelButton {
        let other = selected == "Both" ? "Original" : "Both"
        let deadline = ProcessInfo.processInfo.systemUptime + 4
        var last = ""
        repeat {
            let pair = try sample()
            guard pair.reached(.zero, from: initial) else { throw DragCheckError("A display button moved the subtitle windows.") }
            let buttons = try panelButtons(pid: pointer.pid, frame: pair.island.frame) { $0 == selected || $0 == other }
            last = buttons.map { "\($0.name)=\(String(describing: $0.pressed)) enabled=\($0.enabled)" }.joined(separator: ", ")
            if buttons.count == 2, let on = buttons.first(where: { $0.name == selected }),
               let off = buttons.first(where: { $0.name == other }), on.pressed == true, off.pressed == false,
               on.enabled, off.enabled {
                print("Panel display selection: \(last).")
                return off
            }
            Thread.sleep(forTimeInterval: 0.04)
        } while ProcessInfo.processInfo.systemUptime < deadline
        throw DragCheckError("Did not observe \(selected) as the selected display button. Observed: \(last).")
    }
    let panel = initial.island
    let original = try waitForSelection("Both")
    try click(original, in: panel, pointer: pointer)
    let both = try waitForSelection("Original")
    try captureFrame(windowID: panel.id, frame: panel.frame, crop: panel.frame, output: output, stage: "display-original")
    try click(both, in: panel, pointer: pointer)
    _ = try waitForSelection("Both")
    try captureFrame(windowID: panel.id, frame: panel.frame, crop: panel.frame, output: output, stage: "display-both-restored")
}
