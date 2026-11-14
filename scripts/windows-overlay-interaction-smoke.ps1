param(
    [Parameter(Mandatory = $true)][long]$OverlayWindow,
    [Parameter(Mandatory = $true)][long]$SettingsWindow,
    [Parameter(Mandatory = $true)][string]$SessionStateFile,
    [Parameter(Mandatory = $true)][string]$SettingsActivationFile,
    [Parameter(Mandatory = $true)][ValidateSet('pointer', 'accessibility')][string]$Interaction,
    [Parameter(Mandatory = $true)][string]$FixtureDirectory
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
    private struct Rect { internal int Left, Top, Right, Bottom; }

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
    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint count, Input[] inputs, int size);

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

function Use-OverlayControl([string]$Name, [string]$State, [string]$Action, [switch]$ReturnPoint, [switch]$WaitOnly) {
    $byName = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    $byRole = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    $condition = [System.Windows.Automation.AndCondition]::new([System.Windows.Automation.Condition[]]@($byName, $byRole))
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    $observedState = ''
    while ([DateTime]::UtcNow -lt $deadline) {
        Assert-SettingsHidden
        $observedState = Read-SmokeMarker $SessionStateFile
        if ($observedState -eq $State) {
            try {
                $root = [System.Windows.Automation.AutomationElement]::FromHandle($overlay)
                $button = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
                if ($null -ne $button -and $button.Current.IsEnabled -and -not $button.Current.IsOffscreen) {
                    if ($WaitOnly) { return }
                    if ($Interaction -eq 'accessibility') {
                        $pattern = $null
                        if (-not $button.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
                            throw "The '$Name' control does not support accessibility activation."
                        }
                        $pattern.Invoke()
                        return
                    }
                    $bounds = $button.Current.BoundingRectangle
                    if (-not $bounds.IsEmpty -and $bounds.Width -gt 0 -and $bounds.Height -gt 0) {
                        $x = [int]($bounds.X + $bounds.Width / 2)
                        $y = [int]($bounds.Y + $bounds.Height / 2)
                        [MimiPointerSmoke]::ClickOverlay($overlay, $x, $y, $Action)
                        if ($ReturnPoint) { return @{ X = $x; Y = $y } }
                        return
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
    if ($Interaction -eq 'pointer') {
        $previousDpi = [MimiPointerSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
        if ($previousDpi -eq [IntPtr]::Zero) { throw 'Could not enable per-monitor DPI awareness for pointer testing.' }
        $originalPointer = [MimiPointerSmoke+Point]::new()
        $restorePointer = [MimiPointerSmoke]::GetCursorPos([ref]$originalPointer)
    }
    Close-SettingsForOverlayCheck

    for ($round = 1; $round -le 3; $round++) {
        Use-OverlayControl 'Pause translating' 'listening' "Pause $round"
        Use-OverlayControl 'Resume translating' 'paused' "Resume $round"
    }

    Use-OverlayControl 'Open Mimi Settings' 'listening' 'Settings gear'
    Wait-SettingsOpened 'Settings gear'
    Write-Output "Native overlay $Interaction passed: three Pause/Resume cycles kept Settings hidden, then the gear opened it."

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
