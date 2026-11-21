
import ImageIO

// Compiled together with the actual native test's private AX/window samplers.
@main @MainActor
private enum HoverInput {
    static func main() {
        do { try run() } catch {
            FileHandle.standardError.write(Data("Hover input failed: \(error)\n".utf8))
            exit(1)
        }
    }

    static func run() throws {
        let args = CommandLine.arguments
        let bundles = ["app.yuxino.mimi": "mimi.app", "app.yuxino.mimi.dev": "mimi-dev.app"]
        guard args.count == 8, ProcessInfo.processInfo.environment["RUNNER_ENVIRONMENT"] == "github-hosted",
              AXIsProcessTrusted(), CGPreflightScreenCaptureAccess(), CGPreflightPostEventAccess(),
              let pid = pid_t(args[1]), pid > 0, let bundle = bundles[args[2]],
              let appInfo = NSRunningApplication(processIdentifier: pid), appInfo.bundleIdentifier == args[2],
              appInfo.executableURL?.standardizedFileURL.path == "/Applications/\(bundle)/Contents/MacOS/mimi",
              ["layout", "history", "language"].contains(args[5]), ["observe", "require-first-click"].contains(args[7]) else {
            throw LayoutError("expected_signed_fixture_on_isolated_host")
        }
        let fixture = URL(fileURLWithPath: args[3], isDirectory: true)
        let out = URL(fileURLWithPath: args[4], isDirectory: true)
        guard fixture.lastPathComponent.hasPrefix("mimi-ui-test-"),
              fixture.resolvingSymlinksInPath().deletingLastPathComponent() == FileManager.default.temporaryDirectory.resolvingSymlinksInPath() else {
            throw LayoutError("private_scene_required")
        }
        let capsuleOnly = args[5] == "language"
        let chinese = ProcessInfo.processInfo.environment["MIMI_CAPSULE_TEST_LANGUAGE"] == "zh"
        let languageFields = chinese ? ["识别语言","翻译成"] : ["Recognize","Translate To"]
        let collapseName = chinese ? "收起字幕控制" : "Collapse subtitle controls"
        let layout = args[5] != "history"
        let app = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(app, 0.25)
        try closeSettings(pid: pid, app: app)
        let suffix = args[2].hasSuffix(".dev") ? " (dev)" : ""
        let startup = try windows(pid: pid)
        let roots = startup.filter { $0.title == "Mimi Subtitles" + suffix }
        let children = startup.filter { $0.title == "Mimi" + suffix }
        guard roots.count == 1, children.count == 1 else { throw LayoutError("expected_unique_owned_root_and_island") }
        let root = roots[0], island = children[0]
        let axRoots = (attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []).filter { frame(of: $0).map { equal($0, root.bounds) } ?? false }
        guard axRoots.count == 1, let input = CGEventSource(stateID: .combinedSessionState),
              let originalPoint = CGEvent(source: nil)?.location, !CGEventSource.buttonState(.combinedSessionState, button: .left)
        else { throw LayoutError("missing_ax_root_or_idle_pointer") }
        let outside = CGPoint(x: CGDisplayBounds(CGMainDisplayID()).minX + 8, y: CGDisplayBounds(CGMainDisplayID()).minY + 8)
        guard !root.bounds.contains(outside), !island.bounds.contains(outside) else { throw LayoutError("outside_point_inside") }
        func pointer(_ type: CGEventType, _ point: CGPoint) throws {
            guard let e = CGEvent(mouseEventSource: input, mouseType: type, mouseCursorPosition: point, mouseButton: .left) else {
                throw LayoutError("pointer_event_failed")
            }
            if type != .mouseMoved { e.setIntegerValueField(.mouseEventClickState, value: 1) }
            e.post(tap: .cghidEventTap)
        }
        func center(_ box: CGRect) -> CGPoint { CGPoint(x: box.midX, y: box.midY) }
        func move(_ point: CGPoint) throws {
            let start = CGEvent(source: nil)?.location ?? point
            for index in 1...6 {
                let t = CGFloat(index) / 6
                try pointer(.mouseMoved, CGPoint(x: start.x + (point.x - start.x) * t, y: start.y + (point.y - start.y) * t))
                Thread.sleep(forTimeInterval: 0.014)
            }
            guard let actual = CGEvent(source: nil)?.location, hypot(actual.x - point.x, actual.y - point.y) < 2 else {
                throw LayoutError("pointer_did_not_reach_destination")
            }
        }
        try move(outside)
        defer { try? pointer(.mouseMoved, originalPoint) }
        var tracked = layout ? historyNames : [:]
        if !layout {
            for i in 1...20 {
                let n = String(format: "%02d", i)
                tracked["s\(n)"] = "Original \(n) follows the riverside path past the old station and reaches the bridge."
                tracked["t\(n)"] = "译文 \(n) 沿着河边的小路，经过老车站，走到了桥边。"
            }
        }
        tracked["pause"] = "Pause translating"
        tracked["toolbar"] = "Subtitle Controls"
        if layout { tracked["pendingLabel"] = "Translating" }
        func current() throws -> Sample {
            let own = try windows(pid: pid)
            guard own.contains(where: { $0.id == root.id && equal($0.bounds, root.bounds) }),
                  own.contains(where: { $0.id == island.id && equal($0.bounds, island.bounds) }),
                  !own.contains(where: { $0.title == settingsTitle }),
                  (try? String(contentsOf: fixture.appendingPathComponent("session"), encoding: .utf8)) == "listening",
                  !FileManager.default.fileExists(atPath: fixture.appendingPathComponent("settings-activation").path) else {
                throw LayoutError("signed_window_identity_bounds_session_or_settings_changed")
            }
            return try sample(axRoots[0], bounds: root.bounds.insetBy(dx: 10, dy: 10), inspectHistory: true, expectedNames: tracked)
        }
        func wait(_ stage: String, _ check: (Sample) -> Bool) throws -> Sample {
            let deadline = now + 4
            repeat {
                let value = try current()
                if check(value) { return value }
                Thread.sleep(forTimeInterval: 0.025)
            } while now < deadline
            throw LayoutError("unpainted_stage_\(stage)")
        }
        let initialSource = layout ? "source" : "s18"
        _ = try wait("initial") { $0.lanes[initialSource]?.full == true }
        let recorder = Process()
        recorder.executableURL = URL(fileURLWithPath: args[6])
        recorder.arguments = [String(pid), String(root.id), String(island.id), out.path]
        var recEnv = ProcessInfo.processInfo.environment
        recEnv["MIMI_UI_TEST"] = "1"
        recorder.environment = recEnv
        if !capsuleOnly { try recorder.run() }
        let stop = out.appendingPathComponent("stop")
        func finishRecorder() -> Bool {
            if !FileManager.default.fileExists(atPath: stop.path) { try? Data().write(to: stop, options: .withoutOverwriting) }
            let deadline = now + 9
            while recorder.isRunning && now < deadline { Thread.sleep(forTimeInterval: 0.05) }
            if recorder.isRunning { recorder.terminate(); return false }
            return recorder.terminationStatus == 0
        }
        var finalized = capsuleOnly
        defer { if !finalized { _ = finishRecorder() } }
        if !capsuleOnly {
            let deadline = now + 8
            while !FileManager.default.fileExists(atPath: out.appendingPathComponent("ready").path), now < deadline, recorder.isRunning {
                Thread.sleep(forTimeInterval: 0.02)
            }
            guard recorder.isRunning, FileManager.default.fileExists(atPath: out.appendingPathComponent("ready").path) else { throw LayoutError("recorder_never_armed") }
        }
        var phases: [[String: Any]] = [], events: [[String: Any]] = []
        var outcome: [String: Any] = ["scene": args[5], "rootWindowID": root.id, "islandWindowID": island.id,
                                       "root": rect(root.bounds), "island": rect(island.bounds), "clock": "systemUptime"]
        func save() throws {
            for (file, data) in [("phases", ["clock": "systemUptime", "phases": phases] as [String: Any]),
                                 ("events", outcome.merging(["events": events]) { _, b in b })] {
                try JSONSerialization.data(withJSONObject: data, options: [.sortedKeys, .prettyPrinted])
                    .write(to: out.appendingPathComponent("\(file).json"), options: .atomic)
            }
        }
        defer { try? save() }
        func mark(_ name: String, _ s: Sample? = nil, _ more: [String: Any] = [:]) {
            let p = CGEvent(source: nil)?.location ?? .zero
            var m: [String: Any] = ["name": name, "time": now, "wallMs": Date().timeIntervalSince1970 * 1000, "pointer": ["x": p.x, "y": p.y]]
            if let s {
                m["AXSampleStart"] = s.started; m["AXSampleEnd"] = s.finished
                m["lanes"] = s.lanes.mapValues(\.json)
                m["translating"] = s.translating
                m["backToLive"] = s.backToLive.flatMap { frame(of: $0) }.map(rect) ?? [:]
            }
            m.merge(more) { _, new in new }
            events.append(m)
        }
        func phase(_ name: String, _ expectation: String, _ controls: String, pendingPaint: String? = nil, delay: Double = 0.35, _ operation: @MainActor () throws -> Void = {}) throws {
            guard recorder.isRunning else { throw LayoutError("recorder_exited_before_\(name)") }
            let start = now
            mark("begin-" + name)
            try operation()
            Thread.sleep(forTimeInterval: delay)
            let sampled = try current()
            mark("end-" + name, sampled)
            var result: [String: Any] = ["name": name, "start": start, "end": now, "expectation": expectation, "controls": controls]
            if let pendingPaint { result["pendingPaint"] = pendingPaint }
            phases.append(result)
            try save()
        }
        func lanePoint(_ key: String) throws -> CGPoint {
            let sample = try wait("target-" + key) { $0.lanes[key]?.full == true }
            return center(sample.lanes[key]!.clipped)
        }
        func markFile(_ name: String) throws {
            mark("input-" + name)
            try Data().write(to: fixture.appendingPathComponent(name), options: .withoutOverwriting)
        }
        func stateStep(_ i: Int) throws {
            try markFile("translation-layout-step-\(i)")
            let file = fixture.appendingPathComponent("translation-layout-state.json")
            _ = try wait("step-\(i)") { s in
                guard let data = try? Data(contentsOf: file), let ack = try? JSONDecoder().decode(Ack.self, from: data) else { return false }
                return ack.matches(i) && s.lanes["source"]?.full == true &&
                    (i < 4 || s.lanes["translation"]?.full == true)
            }
        }
        func append(_ i: Int) throws {
            try markFile("append-caption-\(i)")
            _ = try wait("append-\(i)") { s in
                (try? String(contentsOf: fixture.appendingPathComponent("caption-history-count"), encoding: .utf8)) == String(i) &&
                    s.lanes["t\(i)"]?.full == true
            }
        }
        func still(_ name: String, _ controls: String) throws { try phase(name, "stationary", controls, delay: 0.50) }
        func islandPicture(_ stage: String, bounds expected: CGRect? = nil) {
            let bounds = expected ?? island.bounds
            let start = now
            let own = (try? windows(pid: pid)) ?? []
            var details: [String: Any] = ["windowID": island.id, "bounds": rect(bounds), "started": start,
                "orderedFrontFirst": own.map(\.id).filter { $0 == root.id || $0 == island.id }]
            // Fully transparent windows can produce a trimmed PNG. Save exactly
            // the single-window capture, without scaling it or treating it as a screen crop.
            do {
                guard own.contains(where: { $0.id == island.id && equal($0.bounds, bounds) }) else { throw LayoutError("child_geometry_changed_before_picture") }
                let file = out.appendingPathComponent(stage + ".png")
                let shot = Process()
                shot.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
                shot.arguments = ["-x", "-o", "-a", "-l\(island.id)", file.path]
                shot.standardOutput = FileHandle.nullDevice
                shot.standardError = FileHandle.nullDevice
                let done = DispatchSemaphore(value: 0)
                shot.terminationHandler = { _ in done.signal() }
                try shot.run()
                if done.wait(timeout: .now() + 3) == .timedOut {
                    shot.terminate()
                    if done.wait(timeout: .now() + 1) == .timedOut { kill(shot.processIdentifier, SIGKILL) }
                    throw LayoutError("child_picture_timed_out")
                }
                guard shot.terminationStatus == 0, let raw = try? Data(contentsOf: file),
                      let source = CGImageSourceCreateWithData(raw as CFData, nil),
                      let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
                      let width = properties[kCGImagePropertyPixelWidth] as? NSNumber,
                      let height = properties[kCGImagePropertyPixelHeight] as? NSNumber else { throw LayoutError("child_png_unavailable") }
                details["rawPNG"] = file.lastPathComponent
                details["pixelBounds"] = ["x": 0, "y": 0, "width": width, "height": height]
                details["pngByteCount"] = raw.count
            } catch {
                details["captureIssue"] = String(describing: error)
                print("Optional child picture \(stage): \(error)")
            }
            var hit: AXUIElement?
            var owner: pid_t = -1
            let probe = center(bounds)
            details["finished"] = now
            if AXUIElementCopyElementAtPosition(AXUIElementCreateSystemWide(), Float(probe.x), Float(probe.y), &hit) == .success,
               let hit, AXUIElementGetPid(hit, &owner) == .success, owner == pid {
                details["hitRole"] = attribute(hit, kAXRoleAttribute) as? String ?? ""
                if let parent = attribute(hit, kAXWindowAttribute), CFGetTypeID(parent) == AXUIElementGetTypeID(),
                   let hitFrame = frame(of: parent as! AXUIElement) { details["hitWindowBounds"] = rect(hitFrame) }
            }
            mark(stage + "-single-window-capture", nil, details)
        }
        // The recording has a frozen two-window crop. Check the real expandable
        // island afterwards, so its enlarged native panel is captured in full.
        var checkedPanel = false
        func inspectLanguagePanel() throws {
            checkedPanel = true
            _ = try current()
            func surface() throws -> Window {
                let own = try windows(pid: pid)
                guard own.contains(where: { $0.id == root.id && equal($0.bounds, root.bounds) }),
                      !own.contains(where: { $0.title == settingsTitle }),
                      !FileManager.default.fileExists(atPath: fixture.appendingPathComponent("settings-activation").path),
                      let child = own.first(where: { $0.id == island.id }) else { throw LayoutError("panel_root_or_settings_changed") }
                return child
            }
            func physical(_ point: CGPoint, expected: CGRect, stage: String) throws {
                try move(point)
                var hit: AXUIElement?
                var owner: pid_t = -1
                guard expected.contains(point),
                      AXUIElementCopyElementAtPosition(AXUIElementCreateSystemWide(), Float(point.x), Float(point.y), &hit) == .success,
                      let hit, AXUIElementGetPid(hit, &owner) == .success, owner == pid else { throw LayoutError("panel_pointer_hit_another_app") }
                var info: [String: Any] = ["role": attribute(hit, kAXRoleAttribute) as? String ?? ""]
                if let window = attribute(hit, kAXWindowAttribute), CFGetTypeID(window) == AXUIElementGetTypeID(),
                   let bounds = frame(of: window as! AXUIElement) { info["actualAXWindow"] = rect(bounds) }
                mark(stage + "-physical-down", nil, info)
                try pointer(.leftMouseDown, point)
                Thread.sleep(forTimeInterval: 0.07)
                try pointer(.leftMouseUp, point)
                mark(stage + "-physical-up")
            }
            try move(center(island.bounds))
            Thread.sleep(forTimeInterval: 0.3)
            islandPicture("island-before-open")
            try physical(center(island.bounds), expected: island.bounds, stage: "open-language-panel")
            var expanded: Window?, collapse: CGRect?
            let deadline = now + 4
            repeat {
                let w = try surface()
                if w.bounds.height > island.bounds.height + 50,
                   let ax = (attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []).first(where: { frame(of: $0).map { equal($0, w.bounds) } == true }) {
                    var nodes = [ax], seen = 0, fields = Set<String>()
                    while let e = nodes.popLast(), seen < 700 {
                        seen += 1
                        let labels = [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute].compactMap { attribute(e, $0) as? String }
                        let role = attribute(e, kAXRoleAttribute) as? String
                        if let b = frame(of: e), w.bounds.intersects(b) {
                            fields.formUnion(labels.filter { languageFields.contains($0) })
                            if role == kAXButtonRole, labels.contains(where: { $0.hasSuffix(collapseName) }) { collapse = b }
                        }
                        nodes.append(contentsOf: attribute(e, kAXChildrenAttribute) as? [AXUIElement] ?? [])
                    }
                    if fields.count == 2 { expanded = w; if collapse != nil { break } }
                }
                Thread.sleep(forTimeInterval: 0.05)
            } while now < deadline
            let postClick = try surface()
            islandPicture("language-panel-after-physical-click", bounds: postClick.bounds)
            outcome["openedLanguagePanel"] = expanded != nil
            outcome["islandPostClickWindowBounds"] = rect(postClick.bounds)
            guard let expanded else { throw LayoutError("physical_island_click_did_not_open_verified_language_controls") }
            mark("language-panel-both-fields-visible", nil, ["windowID": expanded.id, "fields": languageFields, "bounds": rect(expanded.bounds)])
            guard let collapse else { throw LayoutError("opened_language_panel_but_collapse_button_not_located") }
            try physical(center(collapse), expected: expanded.bounds, stage: "collapse-language-panel")
            let closeDeadline = now + 4
            while !equal(try surface().bounds, island.bounds), now < closeDeadline { Thread.sleep(forTimeInterval: 0.05) }
            _ = try current()
            islandPicture("island-restored-after-panel")
            outcome["collapsedLanguagePanel"] = true
            print("language panel opened from the physical island; both language fields verified; same window returned to its compact bounds")
        }
        defer {
            if !checkedPanel {
                if !finalized { _ = finishRecorder(); finalized = true }
                do { try inspectLanguagePanel() }
                catch { outcome["languagePanelDiagnostic"] = String(describing: error); print("Language panel diagnostic: \(error)") }
            }
        }
        if capsuleOnly {
            islandPicture("island-quiet")
            try captureFrame(windowID: root.id, frame: root.bounds, crop: root.bounds, output: out, stage: "canvas-quiet")
            try move(lanePoint("source"))
            Thread.sleep(forTimeInterval: 0.35)
            islandPicture("island-from-caption-hover")
            try captureFrame(windowID: root.id, frame: root.bounds, crop: root.bounds, output: out, stage: "canvas-hover")
            try move(center(island.bounds))
            Thread.sleep(forTimeInterval: 0.35)
            islandPicture("island-direct-hover")
            do { try inspectLanguagePanel() }
            catch { outcome["languagePanelDiagnostic"] = String(describing: error); throw error }
            outcome["finalListening"] = true
            outcome["settingsClosed"] = true
            try save()
            return
        }
        try still("quiet-before-input", "hidden")
        islandPicture("island-quiet")
        if layout {
            // Save the same root-only patch even when WebKit retains a visually
            // hidden status in its accessibility tree. The video is the paint evidence.
            let patch = CGRect(x: root.bounds.minX + 10, y: root.bounds.minY + 3, width: 145, height: 86)
            outcome["pendingPaintCropGlobal"] = rect(patch)
            func pendingCheckpoint(_ stage: String, _ expected: String) throws {
                try phase(stage, "stationary", expected, pendingPaint: expected, delay: 0.50)
                let s = try current()
                let start = now
                try captureFrame(windowID: root.id, frame: root.bounds, crop: patch, output: out, stage: stage)
                mark(stage + "-capture", s, ["started": start, "finished": now, "expectedPaint": expected, "rootCrop": rect(patch)])
            }
            try phase("quiet-final-pending", "stationary", "hidden") { try stateStep(1) }
            try pendingCheckpoint("pending-quiet-before-entry", "hidden")
            try phase("pending-pointer-enters-caption", "free", "ignore") { try move(lanePoint("source")) }
            try pendingCheckpoint("pending-hovered-caption", "visible")
            try phase("pending-pointer-leaves-caption", "free", "ignore") { try move(outside) }
            try pendingCheckpoint("pending-quiet-after-exit", "hidden")
            try phase("quiet-final-cleared", "stationary", "hidden") { try stateStep(2) }
            try captureFrame(windowID: root.id, frame: root.bounds, crop: patch, output: out, stage: "pending-cleared-reference")
            mark("pending-cleared-reference", try current(), ["rootCrop": rect(patch)])
        } else { try phase("quiet-new-sentence-19", "upward", "hidden") { try append(19) } }
        let bodyKey = layout ? "source" : "t19"
        try phase("pointer-enters-caption", "free", "ignore") { try move(lanePoint(bodyKey)) }
        try still("caption-hover", "visible")
        islandPicture("island-from-caption-hover")
        let pausePoint = try lanePoint("pause")
        let toolbarPoint = (try current()).lanes["toolbar"].map { center($0.clipped) } ?? pausePoint
        try phase("pointer-to-toolbar", "free", "ignore") { try move(toolbarPoint) }
        try phase("pointer-to-language-island", "free", "ignore") { try move(center(island.bounds)) }
        try still("language-island-hover", "visible")
        islandPicture("island-direct-hover")
        try phase("pointer-to-pause-button", "free", "ignore") { try move(pausePoint) }
        try still("pause-button-hover", "visible")
        try phase("pointer-back-to-caption", "free", "ignore") { try move(lanePoint(bodyKey)) }
        if layout {
            try phase("hover-final-pending-again", "stationary", "visible") { try stateStep(3) }
            try phase("hover-translation-arrives", "upward", "visible") { try stateStep(4) }
        } else { try phase("hover-new-sentence-20", "upward", "visible") { try append(20) } }
        try phase("pointer-exits-overlay", "free", "ignore") { try move(outside) }
        if layout { try phase("quiet-pair-confirmed", "stationary", "hidden") { try stateStep(5) } }
        try still("quiet-after-hover", "hidden")
        let latestSource = layout ? "source" : "s20"
        let latestTranslation = layout ? "translation" : "t20"
        let earlySource = layout ? "firstSource" : "s01"
        let earlyTranslation = layout ? "firstTranslation" : "t01"
        func reading(_ s: Sample) -> Bool {
            s.lanes[earlySource]?.full == true && s.lanes[earlyTranslation]?.full == true &&
                s.lanes[latestSource] == nil && s.lanes[latestTranslation] == nil && s.backToLive != nil
        }
        func live(_ s: Sample) -> Bool {
            s.lanes[latestSource]?.full == true && s.lanes[latestTranslation]?.full == true &&
                s.lanes[earlySource] == nil && s.backToLive == nil
        }
        func pointerOwner(_ p: CGPoint) throws -> AXUIElement {
            var element: AXUIElement?
            var owner = pid_t(-1)
            guard let position = CGEvent(source: nil)?.location, hypot(position.x - p.x, position.y - p.y) < 2,
                  root.bounds.contains(p), !island.bounds.contains(p),
                  AXUIElementCopyElementAtPosition(AXUIElementCreateSystemWide(), Float(p.x), Float(p.y), &element) == .success,
                  let element, AXUIElementGetPid(element, &owner) == .success, owner == pid else {
                throw LayoutError("physical_input_did_not_target_owned_root")
            }
            return element
        }
        func hitsReturnButton(_ hit: AXUIElement) -> Bool {
            var node: AXUIElement? = hit
            for _ in 0..<12 {
                guard let e = node else { break }
                if attribute(e, kAXRoleAttribute) as? String == kAXButtonRole,
                   [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute].contains(where: { attribute(e, $0) as? String == "Back to live" }) { return true }
                guard let parent = attribute(e, kAXParentAttribute), CFGetTypeID(parent) == AXUIElementGetTypeID() else { break }
                node = (parent as! AXUIElement)
            }
            return false
        }
        func click(_ p: CGPoint, _ stage: String) throws {
            let hit = try pointerOwner(p)
            let returns = hitsReturnButton(hit)
            mark(stage + "-target", nil, ["hitRole": attribute(hit, kAXRoleAttribute) as? String ?? "", "hitReturnButton": returns])
            if stage == "recovery" && !returns { throw LayoutError("recovery_pointer_did_not_hit_return_button") }
            try pointer(.leftMouseDown, p)
            mark(stage + "-left-down")
            Thread.sleep(forTimeInterval: 0.07)
            try pointer(.leftMouseUp, p)
            mark(stage + "-left-up")
        }
        func photo(_ stage: String) throws {
            try captureFrame(windowID: root.id, frame: root.bounds, crop: root.bounds, output: out, stage: stage)
        }
        func middleCaption() throws -> CGPoint {
            let state = try current()
            let candidates = layout ? ["oldSource", "oldTranslation", latestSource, latestTranslation] :
                ["s18", "t18", "s19", "t19", latestSource, latestTranslation]
            let rows = candidates.compactMap { state.lanes[$0] }.filter(\.full)
            guard let row = rows.min(by: { abs($0.bounds.midY - root.bounds.midY) < abs($1.bounds.midY - root.bounds.midY) }) else {
                throw LayoutError("no_visible_middle_caption_for_wheel")
            }
            return center(row.clipped)
        }
        try phase("pointer-enters-to-read", "free", "ignore") { try move(middleCaption()) }
        try phase("wheel-to-history", "free", "visible") {
            let p = try middleCaption()
            try move(p)
            _ = try pointerOwner(p)
            @MainActor func wheel(_ index: Int) throws {
                guard let event = CGEvent(scrollWheelEvent2Source: input, units: .pixel, wheelCount: 1,
                                          wheel1: Int32(ceil(root.bounds.height * 0.65)), wheel2: 0, wheel3: 0) else {
                    throw LayoutError("native_wheel_unavailable")
                }
                event.location = p
                mark("wheel-up-\(index)")
                event.post(tap: .cghidEventTap)
            }
            @MainActor func observeWheel(_ stage: String, _ predicate: (Sample) -> Bool) throws -> Sample {
                do { return try wait(stage, predicate) }
                catch {
                    mark(stage + "-last-state", try current())
                    try photo(stage + "-failed")
                    throw error
                }
            }
            try wheel(1)
            let detached = try observeWheel("first-scroll-left-live") { $0.backToLive != nil }
            mark("first-scroll-left-live", detached)
            if !reading(detached) {
                for index in 2...(layout ? 7 : 17) {
                    try wheel(index)
                    Thread.sleep(forTimeInterval: 0.06)
                    if reading(try current()) { break }
                }
            }
            _ = try observeWheel("scrolled-to-history", reading)
        }
        try still("reading-while-hovered", "visible")
        try phase("pointer-leaves-while-reading", "free", "ignore") { try move(outside) }
        try still("reading-while-quiet", "hidden")
        let quiet = try wait("quiet-return-button", reading)
        guard let button = quiet.backToLive, let quietBounds = frame(of: button) else { throw LayoutError("quiet_button_geometry_missing") }
        let remembered = center(quietBounds)
        outcome["quietButtonBounds"] = rect(quietBounds)
        outcome["rememberedClickPoint"] = ["x": remembered.x, "y": remembered.y]
        mark("quiet-return-target-measured", quiet, ["buttonBounds": rect(quietBounds)])
        try photo("return-quiet")
        try phase("pointer-to-remembered-return-target", "free", "ignore", delay: 0.12) { try move(remembered) }
        let hovered = try current()
        guard hovered.backToLive != nil, let hoveredBounds = hovered.backToLive.flatMap({ frame(of: $0) }) else {
            throw LayoutError("hovered_return_button_disappeared_before_click")
        }
        let deltaY = hoveredBounds.midY - quietBounds.midY
        outcome["hoveredButtonBounds"] = rect(hoveredBounds)
        outcome["hoveredContainsRememberedPoint"] = hoveredBounds.contains(remembered)
        outcome["buttonDeltaY"] = deltaY
        mark("hovered-return-target-measured", hovered, ["buttonBounds": rect(hoveredBounds)])
        try phase("first-click-at-quiet-target", "free", "ignore", delay: 0.2) { try click(remembered, "remembered") }
        var firstResult = try current()
        let firstDeadline = now + 0.8
        while !live(firstResult) && now < firstDeadline {
            Thread.sleep(forTimeInterval: 0.04)
            firstResult = try current()
        }
        let firstReturned = live(firstResult)
        outcome["firstClickReturnedToLive"] = firstReturned
        outcome["recoveryWasRequired"] = !firstReturned
        mark("first-click-result", firstResult, ["returned": firstReturned])
        try save()
        try photo("return-first-result")
        print("scene=\(args[5]) firstPhysicalClickReturned=\(firstReturned) quietY=\(quietBounds.midY) hoveredY=\(hoveredBounds.midY) deltaY=\(deltaY)")
        if !firstReturned {
            guard reading(firstResult) else { throw LayoutError("first_click_neither_live_nor_reading") }
            // The old toolbar target raises a lock tooltip over the old return
            // button. Approach the measured control from the caption body.
            try phase("leave-obscuring-toolbar-tooltip", "free", "ignore") { try move(lanePoint(earlyTranslation)) }
            guard let newBounds = (try current()).backToLive.flatMap({ frame(of: $0) }) else {
                throw LayoutError("recovery_button_not_visible")
            }
            try phase("reacquire-visible-return-button", "free", "ignore") { try move(center(newBounds)) }
            try phase("recover-at-current-return-button", "free", "ignore") {
                let s = try current()
                guard let currentBounds = s.backToLive.flatMap({ frame(of: $0) }),
                      currentBounds.contains(center(newBounds)) else { throw LayoutError("recovery_target_moved_again") }
                try click(center(newBounds), "recovery")
                _ = try wait("recovered-to-live", live)
            }
        }
        try still("hover-returned-to-live", "visible")
        try phase("pointer-leaves-after-return", "free", "ignore") { try move(outside) }
        try still("quiet-final-live", "hidden")
        let result = try wait("final-live", live)
        mark("complete", result)
        outcome["finalListening"] = true
        outcome["settingsClosed"] = true
        try photo("return-final-live")
        try save()
        let recorderOK = finishRecorder()
        finalized = true
        do { try inspectLanguagePanel() }
        catch { outcome["languagePanelDiagnostic"] = String(describing: error); throw error }
        guard recorderOK else { throw LayoutError("recorder_failed_to_finalize") }
        if args[7] == "require-first-click" && !firstReturned { throw LayoutError("first_physical_click_missed_return") }
    }
}
