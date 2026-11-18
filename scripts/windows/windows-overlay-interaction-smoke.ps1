param(
    [Parameter(Mandatory = $true)][long]$OverlayWindow,
    [Parameter(Mandatory = $true)][long]$SettingsWindow,
    [Parameter(Mandatory = $true)][string]$SessionStateFile,
    [Parameter(Mandatory = $true)][string]$SettingsActivationFile,
    [Parameter(Mandatory = $true)][ValidateSet('pointer', 'accessibility', 'caption-drag')][string]$Interaction,
    [Parameter(Mandatory = $true)][string]$FixtureDirectory,
    [string]$FrontendReadyDirectory,
    [ValidateSet('True', 'False')][string]$ExpectedFullWindows
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class MimiPointerSmoke {
    [StructLayout(LayoutKind.Sequential)]
    private struct MouseInput {
        internal int X, Y;
        internal uint Data, Flags, Time;
        internal UIntPtr Extra;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Input {
        internal uint Type;
        internal MouseInput Mouse;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X, Y; }

    [StructLayout(LayoutKind.Sequential)]
    public struct Rect {
        public int Left, Top, Right, Bottom;
        public int Width { get { return Right - Left; } }
        public int Height { get { return Bottom - Top; } }
        public override string ToString() { return String.Format("({0},{1},{2},{3})", Left, Top, Width, Height); }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MonitorInfo { internal int Size; internal Rect Monitor, Work; internal uint Flags; }

    [StructLayout(LayoutKind.Sequential)]
    private struct GuiThreadInfo {
        internal uint Size, Flags;
        internal IntPtr Active, Focus, Capture, MenuOwner, MoveSize, Caret;
        internal Rect CaretBounds;
    }

    private static Point lastVerifiedMove;
    private static long lastVerifiedMoveTicks;
    private static IntPtr firstHoverWindow, firstHoverLeaf, lastHoverLeaf;

    [StructLayout(LayoutKind.Sequential)]
    private struct DibInfo {
        internal uint HeaderSize;
        internal int Width, Height;
        internal ushort Planes, Bits;
        internal uint Compression, ImageSize;
        internal int XPixelsPerMeter, YPixelsPerMeter;
        internal uint ColorsUsed, ColorsImportant, Color;
    }

    [DllImport("user32.dll")]
    public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")]
    public static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")]
    private static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")]
    private static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")]
    private static extern IntPtr MonitorFromPoint(Point point, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Auto)]
    private static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo info);
    private delegate bool WindowVisitor(IntPtr window, IntPtr data);
    [DllImport("user32.dll")]
    private static extern bool EnumWindows(WindowVisitor visit, IntPtr data);
    [DllImport("user32.dll")]
    private static extern IntPtr GetWindow(IntPtr window, uint command);
    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr window, StringBuilder className, int capacity);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongW")]
    private static extern int GetWindowExStyle(IntPtr window, int index);
    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")]
    private static extern bool GetGUIThreadInfo(uint thread, ref GuiThreadInfo info);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint count, Input[] inputs, int size);
    [DllImport("user32.dll")]
    private static extern IntPtr GetDC(IntPtr window);
    [DllImport("user32.dll")]
    private static extern int ReleaseDC(IntPtr window, IntPtr dc);
    [DllImport("gdi32.dll")]
    private static extern IntPtr CreateCompatibleDC(IntPtr source);
    [DllImport("gdi32.dll")]
    private static extern bool DeleteDC(IntPtr dc);
    [DllImport("gdi32.dll")]
    private static extern IntPtr CreateDIBSection(IntPtr dc, ref DibInfo info, uint usage, out IntPtr pixels, IntPtr section, uint offset);
    [DllImport("gdi32.dll")]
    private static extern IntPtr SelectObject(IntPtr dc, IntPtr value);
    [DllImport("gdi32.dll")]
    private static extern bool DeleteObject(IntPtr value);
    [DllImport("gdi32.dll")]
    private static extern bool BitBlt(IntPtr target, int x, int y, int width, int height, IntPtr source, int sourceX, int sourceY, uint operation);
    [DllImport("gdi32.dll")]
    private static extern bool GdiFlush();

    public sealed class PixelPatch {
        private readonly int x, y, width, height;

        public PixelPatch(int x, int y, int width, int height) {
            this.x = x;
            this.y = y;
            this.width = width;
            this.height = height;
        }

        public int[] Read() {
            IntPtr screen = GetDC(IntPtr.Zero);
            if (screen == IntPtr.Zero) throw new InvalidOperationException("Could not open the screen DC for hover verification.");
            IntPtr target = IntPtr.Zero, bitmap = IntPtr.Zero, previous = IntPtr.Zero;
            try {
                target = CreateCompatibleDC(screen);
                if (target == IntPtr.Zero) throw new InvalidOperationException("Could not create the hover sample DC.");
                var info = new DibInfo { HeaderSize = 40, Width = width, Height = -height, Planes = 1, Bits = 32 };
                IntPtr pixels;
                bitmap = CreateDIBSection(target, ref info, 0, out pixels, IntPtr.Zero, 0);
                if (bitmap == IntPtr.Zero || pixels == IntPtr.Zero) throw new InvalidOperationException("Could not allocate the hover sample bitmap.");
                previous = SelectObject(target, bitmap);
                if (previous == IntPtr.Zero || previous == new IntPtr(-1)) throw new InvalidOperationException("Could not select the hover sample bitmap.");
                // Read the composited desktop, including layered windows.
                if (!BitBlt(target, 0, 0, width, height, screen, x, y, 0x40CC0020) || !GdiFlush())
                    throw new InvalidOperationException("Could not read composed hover pixels from the test desktop.");
                var result = new int[width * height];
                Marshal.Copy(pixels, result, 0, result.Length);
                return result;
            }
            finally {
                if (previous != IntPtr.Zero && previous != new IntPtr(-1)) SelectObject(target, previous);
                if (target != IntPtr.Zero) DeleteDC(target);
                if (bitmap != IntPtr.Zero) DeleteObject(bitmap);
                ReleaseDC(IntPtr.Zero, screen);
            }
        }
    }

    public sealed class PixelReference {
        private readonly int[] quiet, visible;
        private readonly bool[] foreground;
        private readonly double contrast;
        public int ForegroundCount { get; private set; }

        public PixelReference(int[] quiet, int[] visible) {
            this.quiet = quiet;
            this.visible = visible;
            var brightness = Array.ConvertAll(visible, Brightness);
            Array.Sort(brightness);
            int background = brightness[brightness.Length / 2];
            foreground = new bool[visible.Length];
            for (int i = 0; i < foreground.Length; i++) {
                int difference = Distance(quiet[i], visible[i]);
                // The text/icon must be brighter than its own background.
                // A background fill change alone is not evidence of exposure.
                if (Brightness(visible[i]) - background < 32 || difference < 96) continue;
                foreground[i] = true;
                ForegroundCount++;
                contrast += difference;
            }
        }

        public double Exposure(int[] sample) {
            double difference = 0;
            for (int i = 0; i < foreground.Length; i++) {
                if (foreground[i]) difference += Distance(quiet[i], sample[i]) - Distance(visible[i], sample[i]);
            }
            return contrast == 0 ? 0 : (difference / contrast + 1) / 2;
        }

        private static int Brightness(int value) { return ((value & 255) + ((value >> 8) & 255) + ((value >> 16) & 255)) / 3; }

        private static int Distance(int a, int b) {
            return Math.Abs((a & 255) - (b & 255)) + Math.Abs(((a >> 8) & 255) - ((b >> 8) & 255))
                + Math.Abs(((a >> 16) & 255) - ((b >> 16) & 255));
        }
    }

    private static string DescribeWindow(IntPtr window) {
        if (window == IntPtr.Zero) return "none";
        var name = new StringBuilder(128);
        uint process;
        Rect rect;
        GetClassName(window, name, name.Capacity);
        GetWindowThreadProcessId(window, out process);
        var bounds = GetWindowRect(window, out rect)
            ? String.Format("{0},{1}..{2},{3}", rect.Left, rect.Top, rect.Right, rect.Bottom)
            : "unavailable";
        return String.Format("0x{0:X} class={1} pid={2} bounds={3} dpi={4} ex=0x{5:X}",
            window.ToInt64(), name, process, bounds, GetDpiForWindow(window), GetWindowExStyle(window, -20));
    }

    public static IntPtr FindVisibleControlWindow(IntPtr overlay) {
        IntPtr result = IntPtr.Zero;
        int count = 0;
        // On Windows the detached control WebView is an owned top-level
        // window. Settings can have a similar title, so use native ownership.
        EnumWindows(delegate(IntPtr window, IntPtr data) {
            if (IsWindowVisible(window) && GetWindow(window, 4) == overlay) { // GW_OWNER
                result = window;
                count++;
            }
            return true;
        }, IntPtr.Zero);
        if (count > 1)
            throw new InvalidOperationException("Multiple visible windows are owned by the subtitle overlay.");
        return result;
    }

    private static void AssertHit(IntPtr overlay, Point point, string action) {
        var hit = WindowFromPoint(point);
        var root = GetAncestor(hit, 2);
        if (root != overlay)
            throw new InvalidOperationException(String.Format(
                "{0} point ({1},{2}) did not hit the expected Mimi window. Expected {3}; hit {4}; hit root {5}.",
                action, point.X, point.Y, DescribeWindow(overlay), DescribeWindow(hit), DescribeWindow(root)));
    }

    public static void MoveOverWindow(IntPtr window, int x, int y, string action) {
        AssertHit(window, new Point { X = x, Y = y }, action);
        MoveAndVerify(x, y);
        // One read after movement records the native leaf without adding an
        // input event, logging delay or another wait before the pixel sensor.
        lastHoverLeaf = WindowFromPoint(lastVerifiedMove);
        if (firstHoverWindow == IntPtr.Zero) {
            firstHoverWindow = window;
            firstHoverLeaf = lastHoverLeaf;
        }
    }

    public static string PointerOwnership(IntPtr overlay, IntPtr controls) {
        Point point;
        if (!GetCursorPos(out point)) return "pointer_position=unavailable";
        var hit = GetAncestor(WindowFromPoint(point), 2);
        return String.Format("point=({0},{1}) overlay_hit={2} controls_hit={3}", point.X, point.Y, hit == overlay, hit == controls);
    }

    public static string WindowVisibilityAndFocus(IntPtr overlay, IntPtr controls) {
        var foreground = GetForegroundWindow();
        return String.Format("overlay_visible={0} controls_visible={1} overlay_foreground={2} controls_foreground={3}",
            IsWindowVisible(overlay), IsWindowVisible(controls), foreground == overlay, foreground == controls);
    }

    private static string HitSurface(IntPtr leaf, IntPtr overlay, IntPtr controls) {
        var hit = GetAncestor(leaf, 2);
        return hit == overlay ? "overlay" : hit == controls ? "controls" : hit == IntPtr.Zero ? "none" : "other";
    }

    private static string WindowGeometry(IntPtr window) {
        Rect rect;
        var bounds = GetWindowRect(window, out rect)
            ? String.Format("({0},{1},{2},{3})", rect.Left, rect.Top, rect.Right, rect.Bottom)
            : "unavailable";
        return String.Format("rect={0} dpi={1} ex=0x{2:X}", bounds, GetDpiForWindow(window), GetWindowExStyle(window, -20));
    }

    public static string HoverGeometry(IntPtr overlay, IntPtr controls) {
        Point point;
        IntPtr leaf = IntPtr.Zero;
        var hasPointer = GetCursorPos(out point);
        if (hasPointer) leaf = WindowFromPoint(point);
        var current = hasPointer
            ? String.Format("point=({0},{1}) hit={2} leaf=0x{3:X}", point.X, point.Y, HitSurface(leaf, overlay, controls), leaf.ToInt64())
            : "point=unavailable";
        var move = lastVerifiedMoveTicks == 0 ? "unavailable" : String.Format("({0},{1}) age_ms={2} hit_now={3}",
            lastVerifiedMove.X, lastVerifiedMove.Y,
            (System.Diagnostics.Stopwatch.GetTimestamp() - lastVerifiedMoveTicks) * 1000 / System.Diagnostics.Stopwatch.Frequency,
            HitSurface(WindowFromPoint(lastVerifiedMove), overlay, controls));
        return String.Format("verified_move={0} current_{1} first_hover_window={2} first_hover_leaf=0x{3:X} after_move_leaf=0x{4:X} same_first_leaf={5} overlay_{6} controls_{7} {8}",
            move, current, firstHoverWindow == overlay ? "overlay" : "other", firstHoverLeaf.ToInt64(), lastHoverLeaf.ToInt64(),
            lastHoverLeaf == firstHoverLeaf, WindowGeometry(overlay), WindowGeometry(controls), WindowVisibilityAndFocus(overlay, controls));
    }

    public static Point MoveOutside(IntPtr overlay, IntPtr controls) {
        Rect main, child;
        if (!GetWindowRect(overlay, out main) || !GetWindowRect(controls, out child))
            throw new InvalidOperationException("Could not determine the overlay and language window bounds.");
        int middleX = (main.Left + main.Right) / 2, middleY = (main.Top + main.Bottom) / 2;
        var candidates = new[] {
            new Point { X = middleX, Y = Math.Min(main.Top, child.Top) - 24 },
            new Point { X = Math.Max(main.Right, child.Right) + 24, Y = middleY },
            new Point { X = Math.Min(main.Left, child.Left) - 24, Y = middleY },
            new Point { X = middleX, Y = Math.Max(main.Bottom, child.Bottom) + 24 }
        };
        foreach (var candidate in candidates) {
            if (MonitorFromPoint(candidate, 0) == IntPtr.Zero) continue;
            var hit = GetAncestor(WindowFromPoint(candidate), 2);
            if (hit == overlay || hit == controls) continue;
            MoveAndVerify(candidate.X, candidate.Y);
            return candidate;
        }
        throw new InvalidOperationException("Could not find an on-screen point outside both Mimi windows.");
    }

    public static void MoveAndVerify(int x, int y) {
        Point actual;
        if (!SetCursorPos(x, y) || !GetCursorPos(out actual) || actual.X != x || actual.Y != y)
            throw new InvalidOperationException("Could not verify the physical pointer position on the test desktop.");
        lastVerifiedMove = actual;
        lastVerifiedMoveTicks = System.Diagnostics.Stopwatch.GetTimestamp();
    }

    public static Rect ReadBounds(IntPtr window) {
        Rect rect;
        if (!IsWindowVisible(window) || !GetWindowRect(window, out rect))
            throw new InvalidOperationException("Could not read a visible native drag window.");
        return rect;
    }

    public static Point CaptionDragDelta(IntPtr overlay, IntPtr controls) {
        var main = ReadBounds(overlay);
        var child = ReadBounds(controls);
        var info = new MonitorInfo { Size = Marshal.SizeOf(typeof(MonitorInfo)) };
        var monitor = MonitorFromPoint(new Point { X = (main.Left + main.Right) / 2, Y = (main.Top + main.Bottom) / 2 }, 0);
        if (monitor == IntPtr.Zero || !GetMonitorInfo(monitor, ref info))
            throw new InvalidOperationException("Could not read the native drag work area.");
        foreach (int x in new[] { 48, -48 }) foreach (int y in new[] { 32, -32 }) {
            if (Math.Min(main.Left, child.Left) + x >= info.Work.Left && Math.Max(main.Right, child.Right) + x <= info.Work.Right &&
                Math.Min(main.Top, child.Top) + y >= info.Work.Top && Math.Max(main.Bottom, child.Bottom) + y <= info.Work.Bottom)
                return new Point { X = x, Y = y };
        }
        throw new InvalidOperationException("The native drag fixture has no room for a reversible caption gesture.");
    }

    public static bool IsCaptionPoint(IntPtr overlay, int x, int y) {
        return GetAncestor(WindowFromPoint(new Point { X = x, Y = y }), 2) == overlay;
    }

    public static void AssertCaptionMoveModal(IntPtr overlay) {
        uint process;
        var thread = GetWindowThreadProcessId(overlay, out process);
        var gui = new GuiThreadInfo { Size = (uint)Marshal.SizeOf(typeof(GuiThreadInfo)) };
        if (thread == 0 || !GetGUIThreadInfo(thread, ref gui) || (gui.Flags & 0x0002) == 0 || gui.MoveSize != overlay)
            throw new InvalidOperationException("Windows did not enter a native move modal owned by the subtitle root.");
    }

    public static void ClickOverlay(IntPtr overlay, int x, int y, string action) {
        AssertHit(overlay, new Point { X = x, Y = y }, action);
        if (!SetCursorPos(x, y))
            throw new InvalidOperationException("Could not move the pointer on the test desktop.");
        var clicks = new Input[] {
            new Input { Mouse = new MouseInput { Flags = 0x0002 } }, // left down
            new Input { Mouse = new MouseInput { Flags = 0x0004 } }  // left up
        };
        var size = Marshal.SizeOf(typeof(Input));
        var sent = SendInput(2, clicks, size);
        if (sent == 1) SendInput(1, new Input[] { clicks[1] }, size);
        if (sent != 2)
            throw new InvalidOperationException("Windows did not accept both mouse events.");
    }
}
'@

function Assert-SettingsHidden {
    if ([MimiPointerSmoke]::IsWindowVisible($settings) -or
        (Test-Path -LiteralPath $SettingsActivationFile)) {
        throw 'Settings opened without an explicit Settings click.'
    }
}

function Read-SmokeMarker([string]$Path) {
    try {
        # The application may still have the marker open for writing. Read
        # access must share writes so the two handles can coexist on Windows.
        $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        try {
            $reader = [IO.StreamReader]::new($stream)
            try { return $reader.ReadToEnd().Trim() }
            finally { $reader.Dispose() }
        }
        finally { $stream.Dispose() }
    }
    catch [IO.FileNotFoundException] { return '' }
    catch [IO.IOException] {
        if ($_.Exception.HResult -eq -2147024864) { return '' } # ERROR_SHARING_VIOLATION
        throw
    }
}

function Write-HoverMetadata([ValidateSet('root-rehover', 'panel-exit')][string]$Phase) {
    if ($Interaction -ne 'pointer' -or [string]::IsNullOrEmpty($FrontendReadyDirectory)) { return }
    try {
        $optIn = Join-Path $FrontendReadyDirectory 'windows-hover-sample-enabled'
        if (-not (Test-Path -LiteralPath $optIn -PathType Leaf)) { return }
        $request = Join-Path $FrontendReadyDirectory 'windows-hover-sample-request'
        [IO.File]::Open($request, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write).Dispose()
        $pending = @('overlay', 'overlay-control')
        $deadline = [Diagnostics.Stopwatch]::StartNew()
        do {
            foreach ($label in $pending) {
                $path = Join-Path $FrontendReadyDirectory "windows-hover-sample-$label.json"
                # The app publishes complete, typed JSON with an atomic rename.
                if (Test-Path -LiteralPath $path -PathType Leaf) {
                    if ((Get-Item -LiteralPath $path).Length -gt 2048) { continue }
                    $sample = Read-SmokeMarker $path
                    if (-not $sample) { continue }
                    Write-Output "Native hover metadata phase=$Phase surface=$label $sample"
                    $pending = @($pending | Where-Object { $_ -ne $label })
                }
            }
            if ($pending.Count -gt 0) { Start-Sleep -Milliseconds 20 }
        } until ($pending.Count -eq 0 -or $deadline.ElapsedMilliseconds -ge 1800)
        if ($pending.Count -gt 0) { Write-Output "Native hover metadata unavailable phase=$Phase surfaces=$($pending -join ',')." }
    }
    catch { Write-Output "Native hover metadata diagnostic could not finish phase=$Phase." }
}

function Write-RootRehoverGeometry {
    try {
        Write-Output ('Native hover geometry phase=root-rehover ' + [MimiPointerSmoke]::HoverGeometry($overlay, $controls))
        # Read once after the original pixel deadline; do not wait for UIA or
        # change the input, target, assertion or timeout to obtain diagnostics.
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($overlay)
        $byName = [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::NameProperty, 'Collapse subtitles')
        $byRole = [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
        $condition = [System.Windows.Automation.AndCondition]::new([System.Windows.Automation.Condition[]]@($byName, $byRole))
        $button = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
        if ($null -eq $button) { Write-Output 'Native hover target phase=root-rehover present=false' }
        else {
            $bounds = $button.Current.BoundingRectangle
            Write-Output ('Native hover target phase=root-rehover present=true bounds=({0},{1},{2},{3}) empty={4} enabled={5} offscreen={6}' -f
                $bounds.X, $bounds.Y, $bounds.Width, $bounds.Height, $bounds.IsEmpty, $button.Current.IsEnabled, $button.Current.IsOffscreen)
        }
    }
    catch { Write-Output 'Native hover geometry diagnostic could not finish phase=root-rehover.' }
}

function Use-OverlayControl(
    [string]$Name, [string]$State, [string]$Action,
    [switch]$ReturnPoint, [switch]$WaitOnly, [IntPtr]$Surface = $overlay, [switch]$NameSuffix,
    [switch]$BoundsOnly, [switch]$Expand, [int]$WaitMs = 10000, [System.Windows.Rect]$ExpectedBounds = [System.Windows.Rect]::Empty
) {
    $byName = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    $byRole = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    $condition = [System.Windows.Automation.AndCondition]::new([System.Windows.Automation.Condition[]]@($byName, $byRole))
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    $observedState = ''
    while ([DateTime]::UtcNow -lt $deadline) {
        Assert-SettingsHidden
        $observedState = Read-SmokeMarker $SessionStateFile
        if ($observedState -eq $State) {
            try {
                $root = [System.Windows.Automation.AutomationElement]::FromHandle($Surface)
                if ($NameSuffix) {
                    $button = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $byRole) |
                        Where-Object {
                            $elementName = $_.Current.Name
                            -not [string]::IsNullOrEmpty($elementName) -and $elementName.EndsWith($Name, [StringComparison]::Ordinal)
                        } |
                        Select-Object -First 1
                }
                else {
                    $button = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
                }
                if ($null -ne $button -and $button.Current.IsEnabled -and -not $button.Current.IsOffscreen) {
                    if ($WaitOnly) { return }
                    if ($BoundsOnly) {
                        $bounds = $button.Current.BoundingRectangle
                        if (-not $bounds.IsEmpty -and $bounds.Width -gt 0 -and $bounds.Height -gt 0 -and
                            ($ExpectedBounds.IsEmpty -or $bounds -eq $ExpectedBounds)) { return $bounds }
                    }
                    elseif ($Interaction -eq 'accessibility') {
                        $pattern = $null
                        if ($Expand) {
                            # Chromium exposes aria-haspopup/aria-expanded through
                            # ExpandCollapse rather than Invoke. Assert that contract.
                            if (-not $button.TryGetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern, [ref]$pattern)) {
                                throw "The '$Name' control does not support accessibility expansion."
                            }
                            $expandState = $pattern.Current.ExpandCollapseState
                            Write-Output "Language capsule accessibility state: $expandState."
                            if ($expandState -ne [System.Windows.Automation.ExpandCollapseState]::Collapsed) {
                                throw "The language capsule was not collapsed before accessibility expansion."
                            }
                            $pattern.Expand()
                        }
                        else {
                            if (-not $button.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
                                throw "The '$Name' control does not support accessibility activation."
                            }
                            $pattern.Invoke()
                        }
                        return
                    }
                    else {
                        $bounds = $button.Current.BoundingRectangle
                        if (-not $bounds.IsEmpty -and $bounds.Width -gt 0 -and $bounds.Height -gt 0) {
                            $x = [int]($bounds.X + $bounds.Width / 2)
                            $y = [int]($bounds.Y + $bounds.Height / 2)
                            [MimiPointerSmoke]::ClickOverlay($Surface, $x, $y, $Action)
                            if ($ReturnPoint) { return @{ X = $x; Y = $y } }
                            return
                        }
                    }
                }
            }
            catch [System.Windows.Automation.ElementNotAvailableException] {
                # React can replace a control while WebView2 updates the tree.
            }
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $ExpectedBounds.IsEmpty) { throw "$Action did not return to the original sample bounds within $WaitMs ms." }
    throw "Overlay did not expose an enabled '$Name' in state '$State' (last state: '$observedState')."
}

function Move-OverControlBounds([IntPtr]$Window, [System.Windows.Rect]$Bounds, [string]$Label) {
    [MimiPointerSmoke]::MoveOverWindow($Window, [int]($Bounds.X + $Bounds.Width / 2), [int]($Bounds.Y + $Bounds.Height / 2), $Label)
}

function Read-HoverPatches($Toolbar, $Capsule) {
    return @{ Toolbar = $Toolbar.Read(); Capsule = $Capsule.Read() }
}

function Wait-QuietHover($Toolbar, $Capsule, $ToolbarReference, $CapsuleReference) {
    $leave = [Diagnostics.Stopwatch]::StartNew()
    $quietSamples = 0
    do {
        Start-Sleep -Milliseconds 30
        $sample = Read-HoverPatches $Toolbar $Capsule
        $toolbarExposure = $ToolbarReference.Exposure($sample.Toolbar)
        $capsuleExposure = $CapsuleReference.Exposure($sample.Capsule)
        if ($toolbarExposure -le 0.12 -and $capsuleExposure -le 0.12) { $quietSamples++ } else { $quietSamples = 0 }
    } until ($quietSamples -ge 3 -or $leave.ElapsedMilliseconds -ge 1800)
    return @{ Quiet = $quietSamples -ge 3; ElapsedMs = $leave.ElapsedMilliseconds; Toolbar = $toolbarExposure; Capsule = $capsuleExposure }
}

function Wait-CapsuleRevealed($Capsule, $CapsuleReference, [string]$Action, [switch]$DiagnoseRootRehover) {
    $reveal = [Diagnostics.Stopwatch]::StartNew()
    do {
        Start-Sleep -Milliseconds 25
        $exposure = $CapsuleReference.Exposure($Capsule.Read())
    } until ($exposure -ge 0.80 -or $reveal.ElapsedMilliseconds -ge 1800)
    if ($exposure -lt 0.80) {
        if ($DiagnoseRootRehover) {
            Write-RootRehoverGeometry
            Write-HoverMetadata 'root-rehover'
        }
        throw "The language capsule did not reveal again before $Action."
    }
}

function Assert-HoverHandoff($Toolbar, $Capsule, $ToolbarReference, $CapsuleReference, [IntPtr]$Controls, [System.Windows.Rect]$Island) {
    Move-OverControlBounds $Controls $Island 'Move from the toolbar to the owned language capsule'
    $handoff = [Diagnostics.Stopwatch]::StartNew()
    $sampleCount = 0
    $minimumToolbar = 1.0
    $minimumCapsule = 1.0
    do {
        $sampleStartedMs = $handoff.ElapsedMilliseconds
        $sample = Read-HoverPatches $Toolbar $Capsule
        $sampleCount++
        $minimumToolbar = [Math]::Min($minimumToolbar, $ToolbarReference.Exposure($sample.Toolbar))
        $minimumCapsule = [Math]::Min($minimumCapsule, $CapsuleReference.Exposure($sample.Capsule))
        if ($minimumToolbar -lt 0.80 -or $minimumCapsule -lt 0.80) {
            throw ('A control dimmed during the native handoff at {0}ms (toolbar={1:F2}, capsule={2:F2}).' -f $sampleStartedMs, $minimumToolbar, $minimumCapsule)
        }
        if ($sampleStartedMs -lt 400) { Start-Sleep -Milliseconds 20 }
    } until ($sampleStartedMs -ge 400)
    if ($sampleCount -lt 4) { throw "The test desktop yielded only $sampleCount hover handoff samples." }
    Write-Output ('Native hover handoff: {0} samples ending at {1}ms (minimum exposure toolbar={2:F2}, capsule={3:F2}).' -f $sampleCount, $sampleStartedMs, $minimumToolbar, $minimumCapsule)
}

function Test-OverlayHoverHandoff([IntPtr]$Controls, [string]$MoreSettings) {
    $readBounds = @{ BoundsOnly = $true; WaitMs = 2500 }
    $waitForAction = @{ WaitMs = 2500 }
    $pause = Use-OverlayControl 'Pause translating' 'listening' 'Locate the toolbar glyph' @readBounds
    $collapse = Use-OverlayControl 'Collapse subtitles' 'listening' 'Locate the toolbar hover point' @readBounds
    $island = Use-OverlayControl 'Open subtitle controls' 'listening' 'Locate the language capsule' -Surface $Controls -NameSuffix @readBounds
    # UIA and the desktop DC use physical pixels under the same per-monitor
    # DPI context. Measure the Pause glyph and the language/chevron area away
    # from the animated mark on the left. Keep the cursor off the samples.
    $toolbarScale = $pause.Height / 24.0
    $capsuleScale = $island.Height / 30.0
    if ($island.Width -lt 66 * $capsuleScale) { throw 'The language capsule cannot provide a static text sample.' }
    $toolbar = [MimiPointerSmoke+PixelPatch]::new([int]($pause.X + $pause.Width / 2 - 8 * $toolbarScale),
        [int]($pause.Y + 4 * $toolbarScale), [int](16 * $toolbarScale), [int](16 * $toolbarScale))
    $capsule = [MimiPointerSmoke+PixelPatch]::new([int]($island.Right - 32 * $capsuleScale),
        [int]($island.Y + 7 * $capsuleScale), [int](26 * $capsuleScale), [int](16 * $capsuleScale))

    [void][MimiPointerSmoke]::MoveOutside($overlay, $Controls)
    # Give the previous hover's 160ms cross-window hold and the capsule's
    # 130ms fade room to finish before learning the quiet reference.
    Start-Sleep -Milliseconds 380
    $quiet = Read-HoverPatches $toolbar $capsule
    Start-Sleep -Milliseconds 45
    $quietCheck = Read-HoverPatches $toolbar $capsule
    Move-OverControlBounds $overlay $collapse 'Hover the root toolbar'
    $reveal = [Diagnostics.Stopwatch]::StartNew()
    $previous = Read-HoverPatches $toolbar $capsule
    do {
        Start-Sleep -Milliseconds 35
        $visible = Read-HoverPatches $toolbar $capsule
        $toolbarReference = [MimiPointerSmoke+PixelReference]::new($quiet.Toolbar, $visible.Toolbar)
        $capsuleReference = [MimiPointerSmoke+PixelReference]::new($quiet.Capsule, $visible.Capsule)
        $calibrated = $reveal.ElapsedMilliseconds -ge 180 -and
            $toolbarReference.ForegroundCount -ge 10 -and $capsuleReference.ForegroundCount -ge 10 -and
            $toolbarReference.Exposure($quietCheck.Toolbar) -le 0.12 -and $capsuleReference.Exposure($quietCheck.Capsule) -le 0.12 -and
            $toolbarReference.Exposure($previous.Toolbar) -ge 0.92 -and $capsuleReference.Exposure($previous.Capsule) -ge 0.92
        $previous = $visible
    } until ($calibrated -or $reveal.ElapsedMilliseconds -ge 1800)
    if (-not $calibrated) {
        throw "The composed pixels did not establish distinct quiet and revealed controls with the cursor over the root toolbar (foreground toolbar=$($toolbarReference.ForegroundCount), capsule=$($capsuleReference.ForegroundCount))."
    }
    Write-Output "Native hover references separated: toolbar=$($toolbarReference.ForegroundCount) px, capsule=$($capsuleReference.ForegroundCount) px."

    Assert-HoverHandoff $toolbar $capsule $toolbarReference $capsuleReference $Controls $island
    [void][MimiPointerSmoke]::MoveOutside($overlay, $Controls)
    $directLeave = Wait-QuietHover $toolbar $capsule $toolbarReference $capsuleReference
    if (-not $directLeave.Quiet) {
        throw ('The controls did not return to quiet after a direct capsule exit (toolbar={0:F2}, capsule={1:F2}).' -f $directLeave.Toolbar, $directLeave.Capsule)
    }
    Write-Output "Direct capsule exit returned both controls to quiet in $($directLeave.ElapsedMs)ms."
    Assert-SettingsHidden
    Move-OverControlBounds $overlay $collapse 'Hover the root toolbar before opening the panel'
    Wait-CapsuleRevealed $capsule $capsuleReference 'the panel test' -DiagnoseRootRehover
    Assert-HoverHandoff $toolbar $capsule $toolbarReference $capsuleReference $Controls $island

    Use-OverlayControl 'Open subtitle controls' 'listening' 'Open the hovered capsule' -Surface $Controls -NameSuffix @waitForAction
    $panelAction = Use-OverlayControl $MoreSettings 'listening' 'Locate an interior panel control' -Surface $Controls @readBounds
    Move-OverControlBounds $Controls $panelAction 'Move inside the expanded language panel'
    Start-Sleep -Milliseconds 240
    if ([MimiPointerSmoke]::FindVisibleControlWindow($overlay) -ne $Controls) { throw 'Opening the language panel changed its native window.' }
    Use-OverlayControl $MoreSettings 'listening' 'The expanded panel stayed usable' -Surface $Controls -WaitOnly @waitForAction
    Use-OverlayControl 'Collapse subtitle controls' 'listening' 'Close the hovered panel' -Surface $Controls -NameSuffix @waitForAction
    $null = Use-OverlayControl 'Open subtitle controls' 'listening' 'The compact capsule' -Surface $Controls -NameSuffix -ExpectedBounds $island @readBounds
    Write-Output ('Pointer after exact compact bounds, before moving outside: ' + [MimiPointerSmoke]::PointerOwnership($overlay, $Controls))
    [void][MimiPointerSmoke]::MoveOutside($overlay, $Controls)

    $panelLeave = Wait-QuietHover $toolbar $capsule $toolbarReference $capsuleReference
    if (-not $panelLeave.Quiet) {
        $originalFailure = 'The controls did not return to quiet outside both windows (toolbar={0:F2}, capsule={1:F2}).' -f $panelLeave.Toolbar, $panelLeave.Capsule
        Write-Output ('Windows after failed post-panel exit: ' + [MimiPointerSmoke]::WindowVisibilityAndFocus($overlay, $Controls))
        Write-HoverMetadata 'panel-exit'
        try {
            Move-OverControlBounds $Controls $island 'Re-enter the compact capsule for diagnosis'
            Start-Sleep -Milliseconds 160
            [void][MimiPointerSmoke]::MoveOutside($overlay, $Controls)
            $reentry = Wait-QuietHover $toolbar $capsule $toolbarReference $capsuleReference
            Write-Output ('Compact re-entry diagnostic: quiet={0} elapsed={1}ms (toolbar={2:F2}, capsule={3:F2}).' -f $reentry.Quiet, $reentry.ElapsedMs, $reentry.Toolbar, $reentry.Capsule)
        }
        catch {
            Write-Output "Compact re-entry diagnostic could not finish ($($_.Exception.GetType().Name))."
        }
        throw $originalFailure
    }
    Assert-SettingsHidden
    if ((Read-SmokeMarker $SessionStateFile) -ne 'listening') { throw 'The hover handoff changed the active session.' }
    Write-Output "Native panel opened and stayed under hover, then both controls returned to quiet in $($panelLeave.ElapsedMs)ms."

    # The existing Settings-click path begins with the pointer over the root
    # toolbar, as it did after the three Pause/Resume cycles.
    Move-OverControlBounds $overlay $pause 'Restore the original toolbar hover'
    Wait-CapsuleRevealed $capsule $capsuleReference 'the existing Settings check'
}

function Close-SettingsForOverlayCheck {
    if (-not [MimiPointerSmoke]::PostMessage($settings, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) {
        throw 'Could not close Settings before testing the overlay.'
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([MimiPointerSmoke]::IsWindowVisible($settings) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 50
    }
    Assert-SettingsHidden
}

function Wait-SettingsOpened([string]$Action) {
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ([MimiPointerSmoke]::IsWindowVisible($settings) -and
            (Read-SmokeMarker $SettingsActivationFile) -eq 'focus-requested') {
            return
        }
        Start-Sleep -Milliseconds 100
    }
    throw "$Action via $Interaction did not restore the original Settings window."
}

$overlay = [IntPtr]$OverlayWindow
$settings = [IntPtr]$SettingsWindow
# UI Automation rectangles use physical screen coordinates. Match their DPI
# context for cursor movement and hit testing on scaled Windows desktops.
$previousDpi = [IntPtr]::Zero
$restorePointer = $false
try {
    if ($Interaction -in @('pointer', 'caption-drag')) {
        $previousDpi = [MimiPointerSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
        if ($previousDpi -eq [IntPtr]::Zero) { throw 'Could not enable per-monitor DPI awareness for pointer testing.' }
        $originalPointer = [MimiPointerSmoke+Point]::new()
        $restorePointer = [MimiPointerSmoke]::GetCursorPos([ref]$originalPointer)
    }
    Close-SettingsForOverlayCheck

    if ($Interaction -eq 'caption-drag') {
        Add-Type -Path (Join-Path $PSScriptRoot 'tests/CaptionDragDesktop.cs')
        . (Join-Path $PSScriptRoot 'tests/overlay-caption-drag.ps1')
        $controls = [MimiPointerSmoke]::FindVisibleControlWindow($overlay)
        if ($controls -eq [IntPtr]::Zero) { throw 'The caption drag fixture did not expose its owned language control window.' }
        Test-NativeCaptionDrag $controls
        return
    }

    for ($round = 1; $round -le 3; $round++) {
        Use-OverlayControl 'Pause translating' 'listening' "Pause $round"
        Use-OverlayControl 'Resume translating' 'paused' "Resume $round"
    }

    $controls = [MimiPointerSmoke]::FindVisibleControlWindow($overlay)
    if ($controls -eq [IntPtr]::Zero) {
        throw 'The subtitle overlay did not expose its owned language control window.'
    }
    $moreSettings = 'More Settings' + [char]0x2026
    if ($Interaction -eq 'pointer') { Test-OverlayHoverHandoff $controls $moreSettings }
    Use-OverlayControl 'Open subtitle controls' 'listening' 'Language capsule' -Surface $controls -NameSuffix -Expand
    Use-OverlayControl $moreSettings 'listening' 'More Settings in subtitle controls' -Surface $controls
    Wait-SettingsOpened 'More Settings in subtitle controls'
    Write-Output "Native overlay $Interaction passed: three Pause/Resume cycles kept Settings hidden, then More Settings in the owned control window opened it."

    if ($Interaction -eq 'pointer') {
        Remove-Item -LiteralPath $SettingsActivationFile -Force
        Close-SettingsForOverlayCheck
        $failureTrigger = Join-Path $FixtureDirectory 'fail-next-pause'
        try {
            New-Item -ItemType File -Path $failureTrigger | Out-Null
            $pause = Use-OverlayControl 'Pause translating' 'listening' 'Pause with configuration failure' -ReturnPoint
            Use-OverlayControl 'Open speech settings' 'error' 'Wait for configuration error' -WaitOnly
        }
        finally {
            Remove-Item -LiteralPath $failureTrigger -Force -ErrorAction SilentlyContinue
        }
        [MimiPointerSmoke]::ClickOverlay($overlay, $pause.X, $pause.Y, 'Repeat click at the old Pause point')
        $deadline = [DateTime]::UtcNow.AddMilliseconds(500)
        do {
            Assert-SettingsHidden
            Start-Sleep -Milliseconds 25
        } while ([DateTime]::UtcNow -lt $deadline)
        Assert-SettingsHidden
        Use-OverlayControl 'Open speech settings' 'error' 'Explicit error settings action'
        Wait-SettingsOpened 'Explicit error settings action'
        Write-Output 'Native pause-error pointer passed: the old Pause point kept Settings hidden and the explicit error action opened it.'
    }
}
finally {
    if ($restorePointer) { [MimiPointerSmoke]::SetCursorPos($originalPointer.X, $originalPointer.Y) | Out-Null }
    if ($previousDpi -ne [IntPtr]::Zero) { [MimiPointerSmoke]::SetThreadDpiAwarenessContext($previousDpi) | Out-Null }
}
