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
    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint count, Input[] inputs, int size);

    public static void ClickOverlay(IntPtr overlay, int x, int y) {
        var point = new Point { X = x, Y = y };
        if (GetAncestor(WindowFromPoint(point), 2) != overlay)
            throw new InvalidOperationException("The control point did not hit the subtitle overlay.");
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
try {
    if (-not [MimiPointerSmoke]::PostMessage($settings, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) {
        throw 'Could not close Settings before testing the overlay pointer.'
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([MimiPointerSmoke]::IsWindowVisible($settings) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 50
    }
    Assert-SettingsHidden

    for ($round = 1; $round -le 3; $round++) {
        $pause = Wait-OverlayControl 'Pause translating' 'listening'
        [MimiPointerSmoke]::ClickOverlay($overlay, $pause.X, $pause.Y)
        $resume = Wait-OverlayControl 'Resume translating' 'paused'
        [MimiPointerSmoke]::ClickOverlay($overlay, $resume.X, $resume.Y)
    }

    $gear = Wait-OverlayControl 'Open Mimi Settings' 'listening'
    [MimiPointerSmoke]::ClickOverlay($overlay, $gear.X, $gear.Y)
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
    if ($restorePointer) { [MimiPointerSmoke]::SetCursorPos($originalPointer.X, $originalPointer.Y) | Out-Null }
    [MimiPointerSmoke]::SetThreadDpiAwarenessContext($previousDpi) | Out-Null
}
