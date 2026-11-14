param(
    [Parameter(Mandatory = $true)][long]$OverlayWindow,
    [Parameter(Mandatory = $true)][long]$SettingsWindow,
    [Parameter(Mandatory = $true)][string]$SessionStateFile,
    [Parameter(Mandatory = $true)][string]$SettingsActivationFile
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

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
    private struct Rect { internal int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    private struct Message {
        internal IntPtr Window;
        internal uint Id;
        internal UIntPtr WParam;
        internal IntPtr LParam;
        internal uint Time;
        internal Point Point;
        internal uint Private;
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
    private static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr window, StringBuilder className, int capacity);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongW")]
    private static extern int GetWindowExStyle(IntPtr window, int index);
    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr CreateWindowEx(int exStyle, string className, string title, int style,
        int x, int y, int width, int height, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr data);
    [DllImport("user32.dll")]
    private static extern bool DestroyWindow(IntPtr window);
    [DllImport("user32.dll")]
    private static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")]
    private static extern int GetMessage(out Message message, IntPtr window, uint min, uint max);
    [DllImport("user32.dll")]
    private static extern IntPtr DispatchMessage(ref Message message);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint count, Input[] inputs, int size);

    // A regular, non-topmost desktop window keeps the runner's full-screen
    // shell from becoming the foreground after we hide the app's Settings.
    public sealed class DesktopBackdrop : IDisposable {
        private readonly ManualResetEvent ready = new ManualResetEvent(false);
        private readonly Thread worker;
        private IntPtr window;

        public DesktopBackdrop(IntPtr overlay) {
            Rect bounds;
            if (!GetWindowRect(overlay, out bounds))
                throw new InvalidOperationException("Could not locate the overlay for the desktop fixture.");
            worker = new Thread(delegate() {
                SetThreadDpiAwarenessContext(new IntPtr(-4));
                window = CreateWindowEx(0, "STATIC", "Mimi CI pointer backdrop", 0x00CF0000,
                    bounds.Left, bounds.Top, bounds.Right - bounds.Left, bounds.Bottom - bounds.Top,
                    IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
                if (window != IntPtr.Zero) ShowWindow(window, 5); // SW_SHOW
                ready.Set();
                if (window == IntPtr.Zero) return;
                Message message;
                while (GetMessage(out message, IntPtr.Zero, 0, 0) > 0 && message.Id != 0x0010)
                    DispatchMessage(ref message);
                DestroyWindow(window);
            });
            worker.IsBackground = true;
            worker.Start();
            if (!ready.WaitOne(5000) || window == IntPtr.Zero)
                throw new InvalidOperationException("Could not start the desktop backdrop.");
        }

        public void Activate() {
            for (var attempt = 0; attempt < 20; attempt++) {
                SetForegroundWindow(window);
                if (GetForegroundWindow() == window) return;
                Thread.Sleep(50);
            }
            throw new InvalidOperationException("Could not activate the desktop backdrop. Foreground: "
                + DescribeWindow(GetForegroundWindow()));
        }

        public void Dispose() {
            PostMessage(window, 0x0010, IntPtr.Zero, IntPtr.Zero); // WM_CLOSE
            if (worker.Join(2000)) ready.Dispose();
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

    public static void ClickOverlay(IntPtr overlay, int x, int y, string action) {
        var point = new Point { X = x, Y = y };
        var hit = WindowFromPoint(point);
        var root = GetAncestor(hit, 2);
        if (root != overlay)
            throw new InvalidOperationException(String.Format(
                "{0} point ({1},{2}) did not hit the overlay. Expected {3}; hit {4}; hit root {5}.",
                action, x, y, DescribeWindow(overlay), DescribeWindow(hit), DescribeWindow(root)));
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

function Wait-OverlayControl([string]$Name, [string]$State) {
    $byName = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    $byRole = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    $condition = [System.Windows.Automation.AndCondition]::new([System.Windows.Automation.Condition[]]@($byName, $byRole))
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    $observedState = ''
    while ([DateTime]::UtcNow -lt $deadline) {
        Assert-SettingsHidden
        if (Test-Path -LiteralPath $SessionStateFile -PathType Leaf) {
            $observedState = [IO.File]::ReadAllText($SessionStateFile).Trim()
        }
        if ($observedState -eq $State) {
            try {
                $root = [System.Windows.Automation.AutomationElement]::FromHandle($overlay)
                $button = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
                if ($null -ne $button -and $button.Current.IsEnabled -and -not $button.Current.IsOffscreen) {
                    $bounds = $button.Current.BoundingRectangle
                    if (-not $bounds.IsEmpty -and $bounds.Width -gt 0 -and $bounds.Height -gt 0) {
                        $point = [MimiPointerSmoke+Point]::new()
                        $point.X = [int]($bounds.X + $bounds.Width / 2)
                        $point.Y = [int]($bounds.Y + $bounds.Height / 2)
                        return $point
                    }
                }
            }
            catch [System.Windows.Automation.ElementNotAvailableException] {
                # React can replace a control while WebView2 updates the tree.
            }
        }
        Start-Sleep -Milliseconds 100
    }
    throw "Overlay did not expose an enabled '$Name' in state '$State' (last state: '$observedState')."
}

$overlay = [IntPtr]$OverlayWindow
$settings = [IntPtr]$SettingsWindow
# UI Automation rectangles use physical screen coordinates. Match their DPI
# context for cursor movement and hit testing on scaled Windows desktops.
$previousDpi = [MimiPointerSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
if ($previousDpi -eq [IntPtr]::Zero) { throw 'Could not enable per-monitor DPI awareness for pointer testing.' }
$originalPointer = [MimiPointerSmoke+Point]::new()
$restorePointer = [MimiPointerSmoke]::GetCursorPos([ref]$originalPointer)
$backdrop = $null
try {
    $backdrop = [MimiPointerSmoke+DesktopBackdrop]::new($overlay)
    $backdrop.Activate()
    if (-not [MimiPointerSmoke]::PostMessage($settings, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) {
        throw 'Could not close Settings before testing the overlay pointer.'
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([MimiPointerSmoke]::IsWindowVisible($settings) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 50
    }
    Assert-SettingsHidden
    $backdrop.Activate()

    for ($round = 1; $round -le 3; $round++) {
        $pause = Wait-OverlayControl 'Pause translating' 'listening'
        [MimiPointerSmoke]::ClickOverlay($overlay, $pause.X, $pause.Y, "Pause $round")
        $resume = Wait-OverlayControl 'Resume translating' 'paused'
        [MimiPointerSmoke]::ClickOverlay($overlay, $resume.X, $resume.Y, "Resume $round")
    }

    $gear = Wait-OverlayControl 'Open Mimi Settings' 'listening'
    [MimiPointerSmoke]::ClickOverlay($overlay, $gear.X, $gear.Y, 'Settings gear')
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ([MimiPointerSmoke]::IsWindowVisible($settings) -and
            (Test-Path -LiteralPath $SettingsActivationFile -PathType Leaf) -and
            [IO.File]::ReadAllText($SettingsActivationFile).Trim() -eq 'focus-requested') {
            Write-Output 'Native overlay pointer passed: three Pause/Resume cycles kept Settings hidden, then the gear opened it.'
            return
        }
        Start-Sleep -Milliseconds 100
    }
    throw 'Clicking the overlay gear did not restore the original Settings window.'
}
finally {
    if ($null -ne $backdrop) { $backdrop.Dispose() }
    if ($restorePointer) { [MimiPointerSmoke]::SetCursorPos($originalPointer.X, $originalPointer.Y) | Out-Null }
    [MimiPointerSmoke]::SetThreadDpiAwarenessContext($previousDpi) | Out-Null
}
