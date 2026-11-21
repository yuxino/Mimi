import ImageIO

@main @MainActor
private enum HistoryThumbCheck {
    static func main() {
        do { try run() } catch {
            FileHandle.standardError.write(Data("History thumb: \(error)\n".utf8))
            exit(1)
        }
    }

    static func run() throws {
        let argv = CommandLine.arguments
        guard ProcessInfo.processInfo.environment["RUNNER_ENVIRONMENT"] == "github-hosted",
              AXIsProcessTrusted(), CGPreflightPostEventAccess(), CGPreflightScreenCaptureAccess(),
              argv.count == 4, let pid = pid_t(argv[1]), let app = NSRunningApplication(processIdentifier: pid),
              app.bundleIdentifier == "app.yuxino.mimi",
              app.executableURL?.standardizedFileURL.path == "/Applications/mimi.app/Contents/MacOS/mimi" else {
            throw LayoutError("dedicated_host_or_signed_release_missing")
        }
        let fixture = URL(fileURLWithPath: argv[2], isDirectory: true)
        let output = URL(fileURLWithPath: argv[3], isDirectory: true)
        guard fixture.lastPathComponent.hasPrefix("mimi-ui-test-"),
              fixture.resolvingSymlinksInPath().deletingLastPathComponent() == FileManager.default.temporaryDirectory.resolvingSymlinksInPath() else {
            throw LayoutError("unexpected_fixture_directory")
        }
        let application = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(application, 0.25)
        try closeSettings(pid: pid, app: application)
        let owned = try windows(pid: pid)
        let roots = owned.filter { $0.title == "Mimi Subtitles" }
        let islands = owned.filter { $0.title == "Mimi" }
        guard roots.count == 1, islands.count == 1 else { throw LayoutError("subtitle_window_pair_missing") }
        let root = roots[0], island = islands[0], box = root.bounds
        guard box.size == CGSize(width: 580, height: 240) else { throw LayoutError("wrong_fixture_size_\(box.size)") }
        let axWindows = attribute(application, kAXWindowsAttribute) as? [AXUIElement] ?? []
        let axRoots = axWindows.filter { frame(of: $0).map { equal($0, box) } ?? false }
        guard axRoots.count == 1 else { throw LayoutError("subtitle_ax_root_missing") }
        let pointer = try Pointer(pid: pid, controlWindowID: island.id)
        let original = pointer.position
        defer { try? pointer.post(.mouseMoved, at: original) }

        var stages = [[String: Any]]()
        var currentStage = "initial"
        func count() -> String? {
            try? String(contentsOf: fixture.appendingPathComponent("caption-history-count"), encoding: .utf8)
        }
        func checkIdentity() throws {
            let visible = try windows(pid: pid)
            guard visible.contains(where: { $0.id == root.id && equal($0.bounds, box) }),
                  !visible.contains(where: { $0.title == settingsTitle }),
                  (try? String(contentsOf: fixture.appendingPathComponent("session"), encoding: .utf8)) == "listening",
                  !FileManager.default.fileExists(atPath: fixture.appendingPathComponent("settings-activation").path) else {
                throw LayoutError("subtitle_window_moved_or_session_changed")
            }
        }
        func read() throws -> Sample {
            try checkIdentity()
            return try sample(axRoots[0], bounds: box.insetBy(dx: 10, dy: 10), inspectHistory: true)
        }
        func note(_ name: String, _ s: Sample?, _ extra: [String: Any] = [:]) {
            currentStage = name
            var d: [String: Any] = ["stage": name, "at": now, "leftDown": Pointer.pressed,
                                    "count": count() ?? "missing", "root": rect(box)]
            if let s {
                d["rows"] = s.lanes.mapValues(\.json)
                d["footer"] = s.backToLive.flatMap { frame(of: $0) }.map(rect) ?? [:]
            }
            d.merge(extra) { _, new in new }
            stages.append(d)
            print("history_thumb stage=\(name) held=\(Pointer.pressed) count=\(count() ?? "missing") \(extra)")
        }
        func image(_ stage: String) throws -> CGImage {
            try captureFrame(windowID: root.id, frame: box, crop: box, isolateAttachedWindows: true, output: output, stage: stage)
            guard let data = CGImageSourceCreateWithURL(output.appendingPathComponent(stage + ".png") as CFURL, nil),
                  let im = CGImageSourceCreateImageAtIndex(data, 0, nil) else { throw LayoutError("missing_window_pixels") }
            return im
        }
        var finished = false
        defer {
            if !finished { _ = try? image("failed"); note("failure-after-" + currentStage, try? read()) }
            if let bytes = try? JSONSerialization.data(withJSONObject: ["passed": finished, "stages": stages], options: [.prettyPrinted, .sortedKeys]) {
                try? bytes.write(to: output.appendingPathComponent("checkpoints.json"))
            }
        }
        func wait(_ stage: String, _ allowed: (Sample) -> Bool) throws -> Sample {
            currentStage = stage
            let deadline = now + 5
            var last = try read()
            while !allowed(last) && now < deadline {
                Thread.sleep(forTimeInterval: 0.04)
                last = try read()
            }
            guard allowed(last) else { note(stage + "-last", last); throw LayoutError("missing_stage_\(stage)") }
            return last
        }
        func marker(_ index: Int) throws {
            try Data().write(to: fixture.appendingPathComponent("append-caption-\(index)"), options: .withoutOverwriting)
        }
        func ink(_ sample: Sample, _ key: String, _ im: CGImage) throws -> Bool {
            guard let lane = sample.lanes[key], lane.full, box.contains(lane.bounds) else { return false }
            return try glyphWitness(in: im, frame: box, crop: lane.bounds).visible
        }
        func trackSearch(_ footer: CGRect) -> CGRect {
            CGRect(x: box.maxX - 24, y: box.minY + 45, width: 18, height: footer.minY - box.minY - 45)
        }
        let middle = CGPoint(x: box.midX, y: box.midY + 12)
        try pointer.post(.mouseMoved, at: middle)
        let initial = try wait("18-live") { count() == "18" && $0.backToLive == nil && $0.lanes["s18"]?.full == true && $0.lanes["t18"]?.full == true }
        let initialPic = try image("live18")
        guard try ink(initial, "s18", initialPic), try ink(initial, "t18", initialPic) else { throw LayoutError("initial_pair_unpainted") }
        note("18-live", initial)

        guard let wheel = CGEvent(scrollWheelEvent2Source: pointer.source, units: .pixel, wheelCount: 1, wheel1: 270, wheel2: 0, wheel3: 0) else {
            throw LayoutError("wheel_unavailable")
        }
        wheel.location = middle
        wheel.post(tap: .cghidEventTap)
        _ = try wait("enter-history") { $0.backToLive != nil && $0.lanes["t18"]?.full != true }
        Thread.sleep(forTimeInterval: 0.35)
        let reading = try read()
        guard let footer = reading.backToLive.flatMap({ frame(of: $0) }) else { throw LayoutError("reading_footer_missing") }
        let firstPic = try image("read18")
        let rows = reading.lanes.filter { key, lane in key.hasPrefix("t") && key != "t18" && lane.full && lane.bounds.maxY < footer.minY &&
            lane.bounds.minY >= box.minY + 50 }
        guard let anchor = rows.min(by: { abs($0.value.bounds.midY - middle.y) < abs($1.value.bounds.midY - middle.y) }),
              try ink(reading, anchor.key, firstPic) else { throw LayoutError("reading_anchor_unpainted") }
        note("18-reading", reading, ["anchor": anchor.key, "anchorBox": rect(anchor.value.bounds)])
        try marker(19)
        let appended = try wait("19-in-store") { count() == "19" && $0.backToLive != nil }
        Thread.sleep(forTimeInterval: 0.24)
        let settled = try read()
        let with19 = try image("read19")
        guard let same = settled.lanes[anchor.key], equal(same.bounds, anchor.value.bounds), try ink(settled, anchor.key, with19),
              settled.backToLive.flatMap({ frame(of: $0) }).map({equal($0, footer)}) == true else {
            note("19-changed-anchor", appended); throw LayoutError("append_changed_reading_anchor")
        }
        let start = try paintedThumb(in: with19, window: box, search: trackSearch(footer))
        let from = CGPoint(x: start.midX, y: start.midY), end = CGPoint(x: start.midX, y: footer.minY - 2)
        guard end.y - from.y > 12 else { throw LayoutError("insufficient_drag_distance") }
        note("19-reading", settled, ["anchor": anchor.key, "anchorBox": rect(same.bounds), "thumb": rect(start),
                                      "releasePoint": ["x": end.x, "y": end.y]])
        try pointer.post(.mouseMoved, at: from)
        let ownedRoot = OwnedWindow(id: root.id, title: root.title, frame: box)
        let observed = QuickEvents(pid: pid)
        defer {
            observed.stop()
            let trace: [String: Any] = ["targetTapInstalled": observed.installed, "rootWindow": root.id,
                                       "events": observed.snapshot().map(\.json)]
            if let data = try? JSONSerialization.data(withJSONObject: trace, options: .prettyPrinted) {
                try? data.write(to: output.appendingPathComponent("target-mouse.json"))
            }
        }
        pointer.source.userData = QuickEvents.tag
        var finalDragStart = 0.0, finalDragPosted = 0.0
        try pointer.hold(at: from, in: ownedRoot) {
            note("down", nil)
            for i in 1...4 {
                let y = from.y + (end.y - from.y) * CGFloat(i) / 8
                try pointer.post(.leftMouseDragged, at: CGPoint(x: from.x, y: y))
                Thread.sleep(forTimeInterval: 0.035)
            }
            let half = try read(), pic = try image("held-half")
            let moved = try paintedThumb(in: pic, window: box, search: trackSearch(footer))
            let sameFooter = half.backToLive.flatMap { frame(of: $0) }.map { equal($0, footer) } ?? false
            note("drag-middle", half, ["thumb": rect(moved), "footerUnmoved": sameFooter])
            guard Pointer.pressed, moved.midY > start.midY + 2, moved.midY < end.y - 2,
                  sameFooter else { throw LayoutError("scrollbar_or_footer_changed_before_final_segment") }
            // hold() posts mouse-up at exactly this point on return. No
            // screenshot, AX read, wait, or second drag occurs at the bottom.
            finalDragStart = now
            try pointer.post(.leftMouseDragged, at: end)
            finalDragPosted = now
        }
        let releaseAcknowledged = now
        note("up", nil, ["finalDragStartedUptime": finalDragStart, "finalDragPostedUptime": finalDragPosted,
                         "releaseAcknowledgedUptime": releaseAcknowledged,
                         "postAndReleaseUpperBoundMs": (releaseAcknowledged - finalDragStart) * 1000,
                         "nativeWindow": root.id, "targetTapInstalled": observed.installed])
        let early = try read()
        _ = try image("first-after-up")
        note("first-sample-after-up", early, ["sampleStartedUptime": early.started, "sampleFinishedUptime": early.finished])
        Thread.sleep(forTimeInterval: 0.35)
        let released = try read(), rp = try image("released")
        let follows19 = try released.backToLive == nil && ink(released, "s19", rp) && ink(released, "t19", rp)
        note("release-settled", released, ["pair19Following": follows19])
        try marker(20)
        _ = try wait("20-in-store") { _ in count() == "20" }
        Thread.sleep(forTimeInterval: 0.4)
        let next = try read(), nextPic = try image("next20")
        let nextSource = try ink(next, "s20", nextPic), nextTranslation = try ink(next, "t20", nextPic)
        note("20-after-release", next, ["source20Painted": nextSource, "translation20Painted": nextTranslation])
        let events = observed.snapshot()
        var quick = false
        var timing: [String:Any] = ["events": events.map(\.json), "targetTapInstalled": observed.installed]
        if let drag = events.dropLast().last, let up = events.last, drag.type == .leftMouseDragged, up.type == .leftMouseUp,
           up.timestamp >= drag.timestamp {
            let quartzGap = Double(up.timestamp - drag.timestamp) / 1_000_000
            let receiptGap = (up.receipt - drag.receipt) * 1000
            func atPoint(_ point: CGPoint, _ expected: CGPoint) -> Bool {
                abs(point.x - expected.x) <= 1 && abs(point.y - expected.y) <= 1
            }
            let downs = events.filter { $0.type == .leftMouseDown }
            let ups = events.filter { $0.type == .leftMouseUp }
            quick = downs.count == 1 && ups.count == 1 && atPoint(downs[0].point, from) &&
                atPoint(drag.point, end) && atPoint(up.point, end) && quartzGap <= 16.7 && (0...16.7).contains(receiptGap)
            timing["quartzGapMs"] = quartzGap
            timing["receiverGapMs"] = receiptGap
        }
        timing["verifiedFastPairAtTarget"] = quick
        note("native-event-sequence", nil, timing)
        guard quick else { throw LayoutError("fast_release_at_target_unverified") }
        guard follows19, nextSource, nextTranslation, next.backToLive == nil else { throw LayoutError("release_did_not_resume_painted_following") }
        finished = true
    }
}
