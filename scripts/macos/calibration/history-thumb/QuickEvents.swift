import CoreGraphics
import Foundation

/// Observe only tagged events addressed to the isolated test process. The
/// callback runs independently of the observer's AX reads and screenshot work.
final class QuickEvents: @unchecked Sendable {
    struct Entry {
        let type: CGEventType
        let point: CGPoint
        let timestamp: UInt64
        let receipt: TimeInterval
        var json: [String: Any] {
            ["type": type == .leftMouseDown ? "down" : type == .leftMouseUp ? "up" : "drag",
             "x": point.x, "y": point.y, "quartzNs": timestamp, "receivedUptime": receipt]
        }
    }

    static let tag: Int64 = 0x4d_49_4d_49_46_41
    private let lock = NSLock()
    private var entries = [Entry]()
    private var loop: CFRunLoop?
    private var tap: CFMachPort?
    private let done = DispatchSemaphore(value: 0)
    var installed: Bool { tap != nil }

    init(pid: pid_t) {
        let mask = [CGEventType.leftMouseDown, .leftMouseDragged, .leftMouseUp].reduce(CGEventMask(0)) {
            $0 | (CGEventMask(1) << $1.rawValue)
        }
        guard let tap = CGEvent.tapCreateForPid(pid: pid, place: .tailAppendEventTap, options: .listenOnly,
                  eventsOfInterest: mask, callback: { _, type, event, ref in
                      if let ref, event.getIntegerValueField(.eventSourceUserData) == QuickEvents.tag,
                         type == .leftMouseDown || type == .leftMouseDragged || type == .leftMouseUp {
                          let owner = Unmanaged<QuickEvents>.fromOpaque(ref).takeUnretainedValue()
                          let entry = Entry(type: type, point: event.location, timestamp: event.timestamp,
                                            receipt: ProcessInfo.processInfo.systemUptime)
                          owner.lock.lock(); owner.entries.append(entry); owner.lock.unlock()
                      }
                      return Unmanaged.passUnretained(event)
                  }, userInfo: Unmanaged.passUnretained(self).toOpaque()) else { return }
        self.tap = tap
        let ready = DispatchSemaphore(value: 0)
        Thread.detachNewThread { [self] in
            let current = CFRunLoopGetCurrent()!
            let source = CFMachPortCreateRunLoopSource(nil, tap, 0)!
            CFRunLoopAddSource(current, source, .defaultMode)
            lock.lock(); loop = current; lock.unlock()
            CGEvent.tapEnable(tap: tap, enable: true)
            ready.signal()
            CFRunLoopRun()
            CFRunLoopRemoveSource(current, source, .defaultMode)
            done.signal()
        }
        ready.wait()
    }

    func snapshot() -> [Entry] {
        lock.lock(); defer { lock.unlock() }
        return entries
    }

    func stop() {
        guard let tap else { return }
        CGEvent.tapEnable(tap: tap, enable: false)
        lock.lock(); let current = loop; lock.unlock()
        if let current { CFRunLoopStop(current) }
        _ = done.wait(timeout: .now() + 1)
        CFMachPortInvalidate(tap)
    }
}
