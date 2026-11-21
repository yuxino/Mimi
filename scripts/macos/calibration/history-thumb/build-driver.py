from pathlib import Path
import subprocess
import sys

suite, output = map(Path, sys.argv[1:])
harness = Path(__file__).resolve().parent
def prefix(name):
    before, entry, _ = (suite / name).read_text().partition('@main @MainActor')
    if not entry:
        raise SystemExit(f'Expected native test helper entry in {name}')
    return before
rows = '''
    var expected = [String: String]()
    for i in 1...20 {
        let n = String(format: "%02d", i)
        expected["s" + n] = "Original " + n + " follows the riverside path past the old station and reaches the bridge."
        expected["t" + n] = "译文 " + n + " 沿着河边的小路，经过老车站，走到了桥边。"
    }
'''
prior = '    let expected = inspectHistory ? historyNames : names'
source = prefix('translating-scroll.swift')
assert source.count(prior) == 1
source = source.replace(prior, rows)
probe = output.parent / 'HistoryProbe.swift'
probe.write_text(source + (harness / 'Driver.swift').read_text())
pointer = output.parent / 'NativePointer.swift'
pointer.write_text(prefix('caption-drag.swift'))
subprocess.run(['xcrun','swiftc',str(probe),str(pointer),str(suite/'Capture.swift'),str(harness/'PaintedThumb.swift'),str(harness/'QuickEvents.swift'),'-o', str(output)],check=True)
