import AppKit
import ApplicationServices

private struct CheckError: Error, CustomStringConvertible {
    let description: String
    init(_ text: String) { description = text }
}
private let settingsTitle = "mimi UI test settings"
private let anchorText = "History 06 rests beside the quiet river."
private func uptime() -> Double { ProcessInfo.processInfo.systemUptime }
private func ax(_ node: AXUIElement, _ key: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(node, key as CFString, &value) == .success ? value : nil
}
private func rectJSON(_ r: CGRect) -> [String: CGFloat] {
    ["x": r.minX, "y": r.minY, "width": r.width, "height": r.height]
}
private func same(_ a: CGRect, _ b: CGRect) -> Bool {
    abs(a.minX-b.minX) <= 0.5 && abs(a.minY-b.minY) <= 0.5 && abs(a.width-b.width) <= 0.5 && abs(a.height-b.height) <= 0.5
}
private struct Window {
    let id: CGWindowID, title: String, rect: CGRect, visible: Bool
}
private struct Node {
    let element: AXUIElement, role: String, names: [String], rect: CGRect
}

@main @MainActor private enum RecoverControls {
    static func main() {
        do { try run() } catch { FileHandle.standardError.write(Data("Recover controls failed: \(error)\n".utf8)); exit(1) }
    }
    static func run() throws {
        let args = CommandLine.arguments
        guard args.count == 6, let pid = pid_t(args[1]), pid > 0, ["lock", "settings"].contains(args[5]),
              ProcessInfo.processInfo.environment["RUNNER_ENVIRONMENT"] == "github-hosted",
              AXIsProcessTrusted(), CGPreflightPostEventAccess(), CGPreflightScreenCaptureAccess(),
              let running = NSRunningApplication(processIdentifier: pid), running.bundleIdentifier == args[2],
              args[2] == "app.yuxino.mimi.dev", running.executableURL?.path == "/Applications/mimi-dev.app/Contents/MacOS/mimi" else {
            throw CheckError("isolated_signed_development_scene_required")
        }
        let fixture = URL(fileURLWithPath: args[3], isDirectory: true), output = URL(fileURLWithPath: args[4], isDirectory: true)
        guard fixture.lastPathComponent.hasPrefix("mimi-ui-test-"),
              fixture.resolvingSymlinksInPath().deletingLastPathComponent() == FileManager.default.temporaryDirectory.resolvingSymlinksInPath() else {
            throw CheckError("private_fixture_directory_required")
        }
        let scene = args[5], app = AXUIElementCreateApplication(pid), pointer = try OverlayTestPointer()
        AXUIElementSetMessagingTimeout(app, 0.25)
        defer { pointer.restore() }
        var events: [[String: Any]] = [], passed = false
        func record(_ stage: String, _ data: [String: Any] = [:]) {
            var entry = data; entry["stage"] = stage; entry["uptime"] = uptime(); events.append(entry)
        }
        defer {
            let result: [String: Any] = ["scene": scene, "passed": passed, "pid": pid, "events": events,
                "sampling": "Physical mouse input, native window metadata and isolated -a glyph pixels; no provider or audio"]
            if let data = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys]) {
                try? data.write(to: output.appendingPathComponent("result.json"), options: .atomic)
            }
        }
        func windows() throws -> [Window] {
            guard let rows = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
                throw CheckError("native_windows_unavailable")
            }
            return rows.compactMap { r in
                guard (r[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid,
                      let id = r[kCGWindowNumber as String] as? NSNumber,
                      let raw = r[kCGWindowBounds as String] as? NSDictionary, let rect = CGRect(dictionaryRepresentation: raw) else { return nil }
                return Window(id: id.uint32Value, title: r[kCGWindowName as String] as? String ?? "", rect: rect,
                              visible: (r[kCGWindowIsOnscreen as String] as? NSNumber)?.boolValue == true)
            }
        }
        func wait(_ stage: String, _ accepts: () throws -> Bool) throws {
            let end = uptime()+5
            repeat { if try accepts() { record(stage); return }; Thread.sleep(forTimeInterval: 0.04) } while uptime() < end
            throw CheckError(stage+"_timeout")
        }
        func axWindow(_ window: Window) throws -> AXUIElement {
            let candidates = (ax(app, kAXWindowsAttribute) as? [AXUIElement] ?? []).filter {
                overlayTestFrame($0).map { same($0, window.rect) } == true
            }
            guard candidates.count == 1 else { throw CheckError("ax_window_not_unique_\(window.id)") }
            return candidates[0]
        }
        func nodes(_ window: Window) throws -> [Node] {
            var pending = [(try axWindow(window), 0)], visited = 0, result: [Node] = []
            while let (e, depth) = pending.popLast() {
                visited += 1; guard visited <= 2048 else { throw CheckError("ax_tree_limit") }
                if let r = overlayTestFrame(e), window.rect.intersects(r) {
                    result.append(Node(element: e, role: ax(e,kAXRoleAttribute) as? String ?? "",
                        names: [kAXTitleAttribute,kAXDescriptionAttribute,kAXValueAttribute].compactMap { ax(e,$0) as? String }, rect: r))
                }
                if depth < 20 { pending += (ax(e,kAXChildrenAttribute) as? [AXUIElement] ?? []).map { ($0,depth+1) } }
            }
            return result
        }
        func named(_ window: Window, _ name: String, role: String? = nil) throws -> Node {
            let matches = try nodes(window).filter { $0.names.contains(name) && (role == nil || $0.role == role) }
            guard let node = matches.min(by: { $0.rect.width*$0.rect.height < $1.rect.width*$1.rect.height }) else {
                throw CheckError("visible_named_element_missing_\(name)")
            }
            return node
        }
        // Initial fixture Settings is the only setup AX action. Tested controls
        // below always receive physical pointer down/up, never AXPress.
        try wait("initial-settings-closed") {
            guard let w = try windows().first(where: { $0.title == settingsTitle && $0.visible }) else { return true }
            let e = try axWindow(w)
            guard let close = ax(e,kAXCloseButtonAttribute), CFGetTypeID(close) == AXUIElementGetTypeID(),
                  AXUIElementPerformAction(close as! AXUIElement,kAXPressAction as CFString) == .success else { return false }
            return false
        }
        let initial = try windows()
        guard let root = initial.first(where: {$0.title == "Mimi Subtitles (dev)" && $0.visible}),
              let firstChild = initial.first(where: {$0.title == "Mimi (dev)" && $0.visible}) else {
            throw CheckError("native_overlay_pair_missing")
        }
        let childID = firstChild.id
        func surface(settingsAllowed: Bool = false, pausedAllowed: Bool = false) throws -> (Window,Window) {
            let all = try windows()
            guard let r = all.first(where: {$0.id == root.id}), same(r.rect,root.rect), r.visible,
                  let c = all.first(where: {$0.id == childID}) else { throw CheckError("overlay_geometry_or_identity_changed") }
            let state = try String(contentsOf:fixture.appendingPathComponent("session"),encoding:.utf8)
            guard state == "listening" || (pausedAllowed && state == "paused") else { throw CheckError("fixture_not_listening") }
            if !settingsAllowed {
                guard !all.contains(where: {$0.title == settingsTitle && $0.visible}),
                      !FileManager.default.fileExists(atPath:fixture.appendingPathComponent("settings-activation").path) else {
                    throw CheckError("unexpected_settings_activation")
                }
            }
            return (r,c)
        }
        func preferencesLocked() throws -> Bool {
            let p = try JSONSerialization.jsonObject(with:Data(contentsOf:fixture.appendingPathComponent("preferences.json"))) as? [String:Any]
            guard let value = p?["overlay_locked"] as? Bool else { throw CheckError("lock_preference_missing") }
            return value
        }
        func glyph(_ w: Window, _ crop: CGRect, _ stage: String) throws -> GlyphWitness {
            try captureGlyphWitness(windowID:w.id,frame:w.rect,crop:crop,output:output,stage:stage)
        }
        let pauseRect = try named(root,"Pause translating",role:kAXButtonRole).rect
        func pair(_ stage: String, toolbar: Bool, capsule: Bool, settingsAllowed: Bool = false) throws {
            let (r,c) = try surface(settingsAllowed:settingsAllowed)
            let right = try glyph(r,pauseRect.insetBy(dx:4,dy:4),stage+"-root")
            var childPaint: [String:Any] = ["nativeVisible":c.visible]
            var painted = false
            if c.visible {
                guard c.rect.height < 80 else { throw CheckError(stage+"_panel_did_not_collapse") }
                let g = try glyph(c,CGRect(x:c.rect.minX+32,y:c.rect.minY+4,width:c.rect.width-52,height:c.rect.height-8),stage+"-child")
                painted = g.visible; childPaint["glyph"] = g.json
            }
            record(stage,["toolbar":right.json,"capsule":childPaint])
            guard right.visible == toolbar, painted == capsule else { throw CheckError(stage+"_paint_mismatch") }
        }
        let display = CGDisplayBounds(CGMainDisplayID()), outside = CGPoint(x:display.minX+8,y:display.minY+8)
        guard !root.rect.contains(outside), !firstChild.rect.contains(outside) else { throw CheckError("outside_point_invalid") }
        func caption(_ stage: String) throws -> CGPoint {
            let (r,_) = try surface()
            let n = try named(r,anchorText), g = try glyph(r,n.rect.intersection(r.rect),stage)
            record(stage,["glyph":g.json,"bounds":rectJSON(n.rect)])
            guard g.visible, let point = g.point else { throw CheckError("confirmed_caption_not_painted") }
            return point
        }
        func warmBody(_ stage: String) throws {
            try pointer.move(try caption(stage+"-anchor")); Thread.sleep(forTimeInterval:0.5)
            try pair(stage,toolbar:true,capsule:true)
        }
        func clickControl(_ w: Window, _ name: String, _ stage: String) throws {
            let n = try named(w,name,role:kAXButtonRole), p = CGPoint(x:n.rect.midX,y:n.rect.midY)
            try pointer.move(p)
            let g = try glyph(w,n.rect.insetBy(dx:4,dy:4),stage+"-target")
            record(stage+"-target",g.json)
            guard g.visible else { throw CheckError(stage+"_control_not_painted") }
            try pointer.click(p,pid:pid,windowID:w.id,window:w.rect,stage:stage,output:output,buttonName:name)
        }
        try pointer.move(outside); Thread.sleep(forTimeInterval:0.6)
        try pair("initial-quiet",toolbar:false,capsule:false)
        guard try !preferencesLocked() else { throw CheckError("fixture_started_locked") }
        try warmBody("initial-body-hover")

        if scene == "lock" {
            try clickControl(root,"Lock Subtitle Position","lock-first-click")
            try wait("locked-persisted") { try preferencesLocked() }
            record("locked-preference",["overlayLocked":try preferencesLocked()])
            try pointer.move(outside); Thread.sleep(forTimeInterval:0.6)
            try pair("locked-quiet",toolbar:false,capsule:false)
            _ = try caption("locked-caption-retained")
            let (_,c) = try surface(), p = CGPoint(x:c.rect.midX,y:c.rect.midY)
            try pointer.move(p,avoidingBeforeArrival:[root.rect,c.rect]); Thread.sleep(forTimeInterval:0.35)
            try pair("locked-direct-entry",toolbar:false,capsule:true)
            _ = try named(c,"Unlock Subtitle Position",role:kAXButtonRole)
            try pointer.click(p,pid:pid,windowID:c.id,window:c.rect,stage:"unlock-first-click",output:output,buttonName:"Unlock Subtitle Position")
            try wait("unlocked-persisted") { try !preferencesLocked() }
            record("unlocked-preference",["overlayLocked":try preferencesLocked()])
            let (_,u) = try surface()
            guard u.rect.height < 80 else { throw CheckError("unlock_opened_panel") }
            try warmBody("unlocked-body-hover")
            try clickControl(root,"Pause translating","unlocked-pause")
            try wait("root-pause-delivered") { try String(contentsOf:fixture.appendingPathComponent("session"),encoding:.utf8) == "paused" }
            let (paused,_) = try surface(pausedAllowed:true)
            try clickControl(paused,"Resume translating","unlocked-resume")
            try wait("root-resume-delivered") { try String(contentsOf:fixture.appendingPathComponent("session"),encoding:.utf8) == "listening" }
            _ = try caption("unlocked-caption-retained")
            try pointer.move(outside); Thread.sleep(forTimeInterval:0.6)
            try pair("unlocked-final-quiet",toolbar:false,capsule:false)
        } else {
            let (_,c) = try surface(), p = CGPoint(x:c.rect.midX,y:c.rect.midY)
            try pointer.move(p); Thread.sleep(forTimeInterval:0.3)
            try pointer.click(p,pid:pid,windowID:c.id,window:c.rect,stage:"panel-first-open",output:output)
            try wait("panel-expanded") { try surface().1.rect.height > 100 }
            let (_,panel) = try surface()
            for (name,stage) in [("Recognize","panel-source"),("Translate To","panel-target")] {
                let n = try named(panel,name,role:kAXComboBoxRole)
                let g = try glyph(panel,CGRect(x:n.rect.minX+4,y:n.rect.minY+4,width:n.rect.width-28,height:n.rect.height-8),stage)
                record(stage,g.json); guard g.visible else { throw CheckError(stage+"_not_painted") }
            }
            let title = try named(panel,"Subtitle Controls"), blank = CGPoint(x:title.rect.maxX+12,y:title.rect.midY)
            let controls = try nodes(panel).filter { [kAXButtonRole,kAXComboBoxRole,kAXPopUpButtonRole,kAXCheckBoxRole,kAXSliderRole,kAXTextFieldRole].contains($0.role) }
            guard panel.rect.contains(blank), !controls.contains(where:{$0.rect.contains(blank)}) else { throw CheckError("no_verified_blank_header_point") }
            try pointer.move(blank)
            var hit:AXUIElement?, hitPID:pid_t = -1
            let hitResult = AXUIElementCopyElementAtPosition(AXUIElementCreateSystemWide(),Float(blank.x),Float(blank.y),&hit)
            let role = hit.flatMap {ax($0,kAXRoleAttribute) as? String} ?? ""
            let owner = hit.map {AXUIElementGetPid($0,&hitPID)}
            let top = try windows().first(where:{$0.visible && $0.rect.contains(blank)})
            record("blank-hit",["role":role,"pid":hitPID,"point":["x":blank.x,"y":blank.y],"topOwnedWindowID":top?.id ?? 0])
            guard top?.id == panel.id, hitResult == .success, owner == .success, hitPID == pid,
                  [kAXGroupRole,"AXWebArea"].contains(role),
                  let input = CGEventSource(stateID:.combinedSessionState),
                  let down = CGEvent(mouseEventSource:input,mouseType:.leftMouseDown,mouseCursorPosition:blank,mouseButton:.left),
                  let up = CGEvent(mouseEventSource:input,mouseType:.leftMouseUp,mouseCursorPosition:blank,mouseButton:.left) else {
                throw CheckError("blank_point_not_noninteractive_owned_chrome")
            }
            down.post(tap:.cghidEventTap); Thread.sleep(forTimeInterval:0.06); up.post(tap:.cghidEventTap)
            Thread.sleep(forTimeInterval:0.35)
            guard same(try surface().1.rect,panel.rect) else { throw CheckError("blank_click_changed_panel_geometry") }
            record("blank-click-no-settings")
            try clickControl(panel,"More Settings…","more-settings-first-click")
            try wait("settings-opened-and-focused") {
                let value = try? String(contentsOf:fixture.appendingPathComponent("settings-activation"),encoding:.utf8)
                guard value == "focus-requested", let w = try windows().first(where:{$0.title == settingsTitle && $0.visible}),
                      let focused = ax(app,kAXFocusedWindowAttribute), CFGetTypeID(focused) == AXUIElementGetTypeID(),
                      let r = overlayTestFrame(focused as! AXUIElement) else { return false }
                return same(r,w.rect)
            }
            if let w = try windows().first(where:{$0.title == settingsTitle && $0.visible}) {
                record("actual-settings-window",["title":w.title,"windowID":w.id,"bounds":rectJSON(w.rect),
                    "activationMarker":try String(contentsOf:fixture.appendingPathComponent("settings-activation"),encoding:.utf8)])
                try captureFrame(windowID:w.id,frame:w.rect,crop:w.rect,output:output,stage:"actual-settings-window")
            }
            try pointer.move(outside); Thread.sleep(forTimeInterval:0.6)
            try pair("settings-open-outside-quiet",toolbar:false,capsule:false,settingsAllowed:true)
            guard let settings = try windows().first(where:{$0.title == settingsTitle && $0.visible}),
                  let close = ax(try axWindow(settings),kAXCloseButtonAttribute), CFGetTypeID(close) == AXUIElementGetTypeID(),
                  let closeRect = overlayTestFrame(close as! AXUIElement) else { throw CheckError("settings_close_button_missing") }
            let closePoint = CGPoint(x:closeRect.midX,y:closeRect.midY)
            try pointer.move(closePoint)
            try pointer.click(closePoint,pid:pid,windowID:settings.id,window:settings.rect,stage:"settings-close",output:output)
            try wait("settings-closed") { try !windows().contains(where:{$0.title == settingsTitle && $0.visible}) }
            try pointer.move(outside); Thread.sleep(forTimeInterval:0.6)
            try pair("settings-closed-quiet",toolbar:false,capsule:false,settingsAllowed:true)
            // The marker deliberately remains: only this explicit More Settings
            // click was allowed to create it; no file is removed to hide evidence.
            let (r,_) = try surface(settingsAllowed:true)
            let anchor = try named(r,anchorText), g = try glyph(r,anchor.rect.intersection(r.rect),"settings-return-anchor")
            guard g.visible, let point = g.point else { throw CheckError("settings_return_caption_missing") }
            try pointer.move(point); Thread.sleep(forTimeInterval:0.5)
            try pair("settings-return-body-hover",toolbar:true,capsule:true,settingsAllowed:true)
            guard try !windows().contains(where:{$0.title == settingsTitle && $0.visible}) else {
                throw CheckError("settings_reopened_during_caption_hover")
            }
        }
        passed = true; record("complete")
        print("recover_controls scene=\(scene) passed=true")
    }
}
