import CoreGraphics
import Foundation

private struct ThumbPaintError: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

/// Locate the painted WebKit thumb from a single Mimi window. Both rectangles
/// use global top-left logical points; the search must contain the whole thumb.
func paintedThumb(in image: CGImage, window: CGRect, search: CGRect) throws -> CGRect {
    guard !window.isEmpty, !search.isEmpty, window.contains(search) else { throw ThumbPaintError("invalid thumb search") }
    let scale = CGFloat(image.width) / window.width
    let backing = Int(scale.rounded())
    guard (1...2).contains(backing), abs(scale - CGFloat(backing)) < 0.001,
          abs(CGFloat(image.height) / window.height - scale) < 0.001 else {
        throw ThumbPaintError("thumb image does not match its 1x/2x window")
    }
    let x0 = ceil((search.minX - window.minX) * scale), x1 = floor((search.maxX - window.minX) * scale)
    let y0 = ceil((search.minY - window.minY) * scale), y1 = floor((search.maxY - window.minY) * scale)
    let searchPixels = CGRect(x: x0, y: y0, width: x1 - x0, height: y1 - y0)
    let thumbWidth = 6 * backing
    guard searchPixels.width >= CGFloat(thumbWidth), searchPixels.height >= CGFloat(thumbWidth) else {
        throw ThumbPaintError("thumb search too small")
    }
    // The one-pixel rim is evidence of the background on both sides and
    // immediately above/below the caps, even when a cap touches the viewport.
    let patch = searchPixels.insetBy(dx: -1, dy: -1)
        .intersection(CGRect(x: 0, y: 0, width: image.width, height: image.height))
    guard let clipped = image.cropping(to: patch) else { throw ThumbPaintError("thumb pixels unavailable") }
    let width = clipped.width, height = clipped.height
    var rgba = [UInt8](repeating: 0, count: width * height * 4)
    try rgba.withUnsafeMutableBytes { bytes in
        guard let context = CGContext(data: bytes.baseAddress, width: width, height: height, bitsPerComponent: 8,
                                      bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue)
        else { throw ThumbPaintError("thumb bitmap unavailable") }
        context.draw(clipped, in: CGRect(x: 0, y: 0, width: width, height: height))
    }
    func pixel(_ x: Int, _ y: Int) -> (level: Int, neutral: Bool) {
        let index = (y * width + x) * 4
        let r = Int(rgba[index]), g = Int(rgba[index + 1]), b = Int(rgba[index + 2])
        return ((r + g + b) / 3, max(r, g, b) - min(r, g, b) <= 8)
    }
    enum Row { case cap, shoulder, body }
    func shape(_ left: Int, _ y: Int) -> Row? {
        let background = max(pixel(left - 1, y).level, pixel(left + thumbWidth, y).level)
        var first = thumbWidth, last = -1, count = 0, darkest = 255, lightest = 0
        for dx in 0..<thumbWidth {
            let sample = pixel(left + dx, y)
            guard sample.neutral, sample.level >= background + 12 else { continue }
            first = min(first, dx)
            last = dx
            count += 1
            if dx > 0 && dx < thumbWidth - 1 {
                darkest = min(darkest, sample.level)
                lightest = max(lightest, sample.level)
            }
        }
        guard count > 0, count == last - first + 1,
              abs(first - (thumbWidth - 1 - last)) <= backing else { return nil }
        // Both outer pixels must clear the track. Edge antialiasing can make
        // them softer than the straight, uniform fill between them.
        if count == thumbWidth, lightest - darkest <= 6 { return .body }
        return first > 0 && last < thumbWidth - 1 ? .cap : .shoulder
    }
    let leftMin = max(1, Int(x0 - patch.minX))
    let leftMax = min(width - thumbWidth - 1, Int(x1 - patch.minX) - thumbWidth)
    let top = Int(y0 - patch.minY), bottom = Int(y1 - patch.minY)
    guard leftMin <= leftMax else { throw ThumbPaintError("thumb side edges outside the window") }
    var matches: [CGRect] = []
    for left in leftMin...leftMax {
        let rows = (0..<height).map { shape(left, $0) }
        var y = top
        while y < bottom {
            guard rows[y] != nil else { y += 1; continue }
            let start = y
            while y < bottom && rows[y] != nil { y += 1 }
            guard start > 0, y < height, rows[start - 1] == nil, rows[y] == nil,
                  rows[start] == .cap, rows[y - 1] == .cap, y - start > thumbWidth,
                  let firstFlat = rows[start..<y].firstIndex(of: .body),
                  let lastFlat = rows[start..<y].lastIndex(of: .body),
                  firstFlat - start <= 3 * backing + 1, y - 1 - lastFlat <= 3 * backing + 1,
                  rows[firstFlat...lastFlat].allSatisfy({ $0 == .body }) else { continue }
            matches.append(CGRect(x: window.minX + (patch.minX + CGFloat(left)) / scale,
                                  y: window.minY + (patch.minY + CGFloat(start)) / scale,
                                  width: CGFloat(thumbWidth) / scale, height: CGFloat(y - start) / scale))
        }
    }
    guard matches.count == 1 else { throw ThumbPaintError("expected one fully painted rounded thumb, found \(matches.count)") }
    return matches[0]
}
