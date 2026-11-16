param([string]$ChildDesktop, [string]$Receipt, [string]$MimiExecutable)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or
    $env:RUNNER_OS -ne 'Windows' -or $env:RUNNER_ARCH -ne 'ARM64') {
    throw 'The temporary input desktop probe requires a disposable hosted Windows ARM64 runner.'
}

Add-Type @'
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class MimiIsolatedInputProbe {
    private const uint ReadObjects = 0x0001, CreateWindow = 0x0002, WriteObjects = 0x0080, SwitchAccess = 0x0100;
    private const int ObjectName = 2;
    private const uint Popup = 0x80000000, Visible = 0x10000000, Border = 0x00800000;
    private const uint LeftDown = 0x0201, LeftUp = 0x0202, RemoveMessage = 1;
    private const uint InjectLeftDown = 0x0002, InjectLeftUp = 0x0004;

    [StructLayout(LayoutKind.Sequential)]
    private struct Point { internal int X, Y; }
    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { internal int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    private struct Message {
        internal IntPtr Window;
        internal uint Id;
        internal UIntPtr WParam;
        internal IntPtr LParam;
        internal uint Time;
        internal Point Position;
        internal uint Reserved;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct MouseInput {
        internal int X, Y;
        internal uint Data, Flags, Time;
        internal UIntPtr Extra;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct Input { internal uint Type; internal MouseInput Mouse; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct StartupInfo {
        internal uint Size;
        internal string Reserved, Desktop, Title;
        internal uint X, Y, Width, Height, XChars, YChars, Fill, Flags;
        internal ushort ShowWindow, ReservedBytes;
        internal IntPtr ReservedData, StandardInput, StandardOutput, StandardError;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct ProcessInfo {
        internal IntPtr Process, Thread;
        internal uint ProcessId, ThreadId;
    }

    [DllImport("user32.dll", SetLastError = true)]
    private static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll", ExactSpelling = true, CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateDesktopW(string name, IntPtr device, IntPtr mode, uint flags, uint access, IntPtr security);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool SwitchDesktop(IntPtr desktop);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll", ExactSpelling = true, CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool GetUserObjectInformationW(IntPtr handle, int index, StringBuilder data, uint bytes, out uint required);
    [DllImport("user32.dll", ExactSpelling = true, CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateWindowExW(uint ex, string kind, string name, uint style, int x, int y, int width, int height,
        IntPtr parent, IntPtr menu, IntPtr instance, IntPtr parameter);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool DestroyWindow(IntPtr window);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")]
    private static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")]
    private static extern IntPtr GetThreadDesktop(uint threadId);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool SetThreadDesktop(IntPtr desktop);
    [DllImport("kernel32.dll")]
    private static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", ExactSpelling = true, CharSet = CharSet.Unicode)]
    private static extern int GetClassNameW(IntPtr window, StringBuilder name, int max);
    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("dwmapi.dll")]
    private static extern int DwmGetWindowAttribute(IntPtr window, uint kind, out uint flags, uint size);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint count, Input[] entries, int size);
    [DllImport("user32.dll", ExactSpelling = true)]
    private static extern bool PeekMessageW(out Message message, IntPtr window, uint min, uint max, uint remove);
    [DllImport("kernel32.dll", ExactSpelling = true, CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CreateProcessW(string app, StringBuilder command, IntPtr processSecurity, IntPtr threadSecurity,
        bool inherit, uint flags, IntPtr environment, string directory, ref StartupInfo startup, out ProcessInfo info);
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetExitCodeProcess(IntPtr process, out uint status);
    [DllImport("kernel32.dll")]
    private static extern bool TerminateProcess(IntPtr process, uint status);
    [DllImport("kernel32.dll")]
    private static extern bool CloseHandle(IntPtr handle);

    private static InvalidOperationException Win32Failure(string operation) {
        int code = Marshal.GetLastWin32Error();
        return new InvalidOperationException(operation + " failed (win32=" + code + ").", new Win32Exception(code));
    }

    private static string DesktopName(IntPtr desktop) {
        var name = new StringBuilder(128);
        uint required;
        if (!GetUserObjectInformationW(desktop, ObjectName, name, (uint)(name.Capacity * 2), out required))
            throw Win32Failure("GetUserObjectInformationW");
        return name.ToString();
    }

    private static string ActiveDesktopName() {
        var handle = OpenInputDesktop(0, false, ReadObjects);
        if (handle == IntPtr.Zero) throw Win32Failure("OpenInputDesktop");
        try { return DesktopName(handle); }
        finally { CloseDesktop(handle); }
    }

    private static string DescribeWindow(IntPtr window) {
        if (window == IntPtr.Zero) return "none";
        var kind = new StringBuilder(96);
        GetClassNameW(window, kind, kind.Capacity);
        uint pid, cloak;
        GetWindowThreadProcessId(window, out pid);
        Rect rect;
        var bounds = GetWindowRect(window, out rect) ?
            String.Format("{0},{1}..{2},{3}", rect.Left, rect.Top, rect.Right, rect.Bottom) : "unknown";
        var cloakState = DwmGetWindowAttribute(window, 14, out cloak, sizeof(uint)) == 0 ? cloak.ToString() : "unknown";
        return String.Format("0x{0:X} class={1} pid={2} bounds={3} visible={4} cloak={5}",
            window.ToInt64(), kind, pid, bounds, IsWindowVisible(window), cloakState);
    }

    private static void ObserveInitialMiss(Point point, IntPtr window, IntPtr firstHit) {
        var clock = Stopwatch.StartNew();
        var ownerDesktop = DesktopName(GetThreadDesktop(GetCurrentThreadId()));
        Console.WriteLine("Probe point {0},{1}: thread={2}; input={3}; expected {4}; original hit {5}.",
            point.X, point.Y, ownerDesktop, ActiveDesktopName(), DescribeWindow(window), DescribeWindow(firstHit));
        // Observe only. Preserve the original failed assertion and send no input,
        // even if the compositor starts returning the probe window later.
        foreach (int sample in new[] { 0, 25, 75, 200, 500 }) {
            var remaining = sample - (int)clock.ElapsedMilliseconds;
            if (remaining > 0) Thread.Sleep(remaining);
            var hit = WindowFromPoint(point);
            Console.WriteLine("Probe miss follow-up t={0}ms: input={1}; hit_own={2}; hit {3}; foreground {4}.",
                clock.ElapsedMilliseconds, ActiveDesktopName(), hit == window, DescribeWindow(hit),
                DescribeWindow(GetForegroundWindow()));
        }
    }

    private static void ClickOwnWindow(IntPtr window, string desktopName) {
        if (ActiveDesktopName() != desktopName)
            throw new InvalidOperationException("The probe desktop does not own input.");
        Rect rect;
        if (!GetWindowRect(window, out rect)) throw Win32Failure("GetWindowRect");
        var point = new Point { X = (rect.Left + rect.Right) / 2, Y = (rect.Top + rect.Bottom) / 2 };
        var firstHit = WindowFromPoint(point);
        if (firstHit != window) {
            try { ObserveInitialMiss(point, window, firstHit); }
            catch (Exception error) { Console.WriteLine("Probe miss observation stopped: {0}", error.Message); }
            throw new InvalidOperationException("The probe coordinates do not hit its own window.");
        }
        if (!SetCursorPos(point.X, point.Y)) throw Win32Failure("SetCursorPos");
        Point actual;
        if (!GetCursorPos(out actual)) throw Win32Failure("GetCursorPos");
        if (actual.X != point.X || actual.Y != point.Y || ActiveDesktopName() != desktopName || WindowFromPoint(actual) != window)
            throw new InvalidOperationException("Input ownership changed before the probe click.");
        var entries = new[] {
            new Input { Type = 0, Mouse = new MouseInput { Flags = InjectLeftDown } },
            new Input { Type = 0, Mouse = new MouseInput { Flags = InjectLeftUp } }
        };
        var size = Marshal.SizeOf(typeof(Input));
        var sent = SendInput(2, entries, size);
        if (sent == 1) SendInput(1, new[] { entries[1] }, size);
        if (sent != 2) throw Win32Failure("SendInput accepted " + sent + " events");
        Console.WriteLine("Owned probe hit verified; SendInput accepted exactly two events.");
    }

    private static void RequireClickPair(IntPtr window) {
        uint expected = LeftDown;
        var clock = Stopwatch.StartNew();
        while (clock.ElapsedMilliseconds < 2500) {
            Message message;
            if (PeekMessageW(out message, window, LeftDown, LeftUp, RemoveMessage)) {
                if (message.Window != window || message.Id != expected)
                    throw new InvalidOperationException("The probe got an unexpected mouse message.");
                if (expected == LeftUp) {
                    Console.WriteLine("Probe window received WM_LBUTTONDOWN and WM_LBUTTONUP in order.");
                    return;
                }
                expected = LeftUp;
            } else {
                Thread.Sleep(10);
            }
        }
        throw new InvalidOperationException("The owned probe window did not receive both mouse messages.");
    }

    private static IntPtr OpenProbeWindow() {
        var window = CreateWindowExW(0, "BUTTON", "Mimi isolated input probe", Popup | Visible | Border,
            80, 100, 240, 140, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
        if (window == IntPtr.Zero) throw Win32Failure("CreateWindowExW (probe)");
        return window;
    }

    public static void RunChild(string desktopName) {
        if (DesktopName(GetThreadDesktop(GetCurrentThreadId())) != desktopName || ActiveDesktopName() != desktopName)
            throw new InvalidOperationException("The child launch did not attach to the active temporary desktop.");
        var window = OpenProbeWindow();
        try {
            ClickOwnWindow(window, desktopName);
            RequireClickPair(window);
        } finally {
            if (!DestroyWindow(window)) throw Win32Failure("DestroyWindow (child probe)");
        }
    }

    private static void LaunchProbeChild(string shell, string script, string receipt, string desktopName, string mimi) {
        var runMimi = !String.IsNullOrEmpty(mimi);
        if (shell.Contains("\"") || script.Contains("\"") || receipt.Contains("\"") || (runMimi && mimi.Contains("\"")))
            throw new InvalidOperationException("Unexpected quote in probe launch paths.");
        var command = new StringBuilder("\"" + shell + "\" -NoLogo -NoProfile -NonInteractive -File \"" + script +
            "\" -ChildDesktop \"" + desktopName + "\" -Receipt \"" + receipt + "\"");
        if (runMimi) command.Append(" -MimiExecutable \"").Append(mimi).Append("\"");
        var startup = new StartupInfo { Size = (uint)Marshal.SizeOf(typeof(StartupInfo)), Desktop = "WinSta0\\" + desktopName };
        ProcessInfo child;
        const uint noConsole = 0x08000000;
        if (!CreateProcessW(shell, command, IntPtr.Zero, IntPtr.Zero, false, noConsole, IntPtr.Zero, null, ref startup, out child))
            throw Win32Failure("CreateProcessW (isolated probe)");
        var childExited = false;
        try {
            var wait = WaitForSingleObject(child.Process, runMimi ? 240000u : 20000u);
            if (wait == 0x0102) throw new TimeoutException("The isolated probe process did not finish.");
            if (wait != 0) throw Win32Failure("WaitForSingleObject (isolated probe)");
            childExited = true;
            uint code;
            if (!GetExitCodeProcess(child.Process, out code)) throw Win32Failure("GetExitCodeProcess (isolated probe)");
            var outcome = File.Exists(receipt) ? File.ReadAllText(receipt).Trim() : "No child receipt.";
            Console.WriteLine("Isolated child exit={0}: {1}", code, outcome);
            var expected = runMimi ? "Mimi ARM64 native mouse smoke completed on the isolated desktop."
                : "Child observed both owned-window mouse messages.";
            if (code != 0 || !outcome.EndsWith(expected, StringComparison.Ordinal))
                throw new InvalidOperationException("The isolated child did not prove native mouse delivery.");
        } finally {
            if (!childExited) {
                if (runMimi) {
                    try { using (var process = Process.GetProcessById((int)child.ProcessId)) process.Kill(true); }
                    catch (ArgumentException) { }
                    catch (InvalidOperationException) { }
                    catch (Win32Exception) { }
                }
                TerminateProcess(child.Process, 1);
                WaitForSingleObject(child.Process, 3000);
            }
            CloseHandle(child.Thread);
            CloseHandle(child.Process);
        }
    }

    public static void Run(string shell, string script, string receipt, string mimi) {
        var original = OpenInputDesktop(0, false, ReadObjects | SwitchAccess);
        if (original == IntPtr.Zero) throw Win32Failure("OpenInputDesktop (original)");
        try {
            if (!String.Equals(DesktopName(original), "Default", StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("The runner is not on the expected Default desktop.");
            var desktopName = "MimiQa" + Guid.NewGuid().ToString("N");
            var temporary = IntPtr.Zero;
            var gate = new object();
            var canceled = false;
            var switched = false;
            var separateProcess = !String.IsNullOrEmpty(shell);
            Exception failure = null;
            using (var ready = new ManualResetEventSlim(false))
            using (var permitInput = new ManualResetEventSlim(false)) {
                var worker = new Thread(() => {
                    var window = IntPtr.Zero;
                    try {
                        temporary = CreateDesktopW(desktopName, IntPtr.Zero, IntPtr.Zero, 0,
                            ReadObjects | CreateWindow | WriteObjects | SwitchAccess, IntPtr.Zero);
                        if (temporary == IntPtr.Zero) throw Win32Failure("CreateDesktopW");
                        if (separateProcess) {
                            ready.Set();
                            return;
                        }
                        // The hosted runner kept the worker on Default after CreateDesktopW.
                        // Bind this windowless thread explicitly before creating the probe.
                        var before = DesktopName(GetThreadDesktop(GetCurrentThreadId()));
                        if (!SetThreadDesktop(temporary)) throw Win32Failure("SetThreadDesktop (probe worker)");
                        if (DesktopName(GetThreadDesktop(GetCurrentThreadId())) != desktopName)
                            throw new InvalidOperationException("The probe worker did not enter the temporary desktop.");
                        Console.WriteLine("Probe worker desktop: {0} -> {1}.", before, desktopName);
                        window = OpenProbeWindow();
                        ready.Set();
                        if (!permitInput.Wait(6000)) throw new TimeoutException("The input desktop was never enabled.");
                        lock (gate) {
                            if (canceled) return;
                            ClickOwnWindow(window, desktopName);
                        }
                        RequireClickPair(window);
                    } catch (Exception error) {
                        failure = error;
                        ready.Set();
                    } finally {
                        if (window != IntPtr.Zero && !DestroyWindow(window) && failure == null)
                            failure = Win32Failure("DestroyWindow (probe)");
                    }
                });
                worker.IsBackground = true;
                worker.Start();
                try {
                    if (!ready.Wait(5000)) throw new TimeoutException("Temporary desktop setup timed out.");
                    if (failure != null) throw failure;
                    if (!String.Equals(ActiveDesktopName(), "Default", StringComparison.OrdinalIgnoreCase))
                        throw new InvalidOperationException("The input desktop changed during setup.");
                    if (!SwitchDesktop(temporary)) throw Win32Failure("SwitchDesktop (temporary)");
                    switched = true;
                    if (ActiveDesktopName() != desktopName)
                        throw new InvalidOperationException("The switch did not give the temporary desktop input.");
                    Console.WriteLine("Windows switched to the temporary input desktop.");
                    if (separateProcess) {
                        LaunchProbeChild(shell, script, receipt, desktopName, mimi);
                    } else {
                        permitInput.Set();
                        if (!worker.Join(5500)) throw new TimeoutException("The mouse probe did not settle.");
                    }
                    if (failure != null) throw failure;
                } finally {
                    try {
                        lock (gate) {
                            canceled = true;
                            if (switched) {
                                if (!SwitchDesktop(original)) throw Win32Failure("SwitchDesktop (restore)");
                                if (!String.Equals(ActiveDesktopName(), "Default", StringComparison.OrdinalIgnoreCase))
                                    throw new InvalidOperationException("The original input desktop did not return.");
                                Console.WriteLine("Restored the Default input desktop.");
                            }
                        }
                    } finally {
                        permitInput.Set();
                        if (!worker.Join(6500)) throw new TimeoutException("The probe worker did not exit.");
                        if (temporary != IntPtr.Zero && !CloseDesktop(temporary))
                            throw Win32Failure("CloseDesktop (temporary)");
                    }
                }
            }
        } finally {
            CloseDesktop(original);
        }
        Console.WriteLine("Isolated ARM64 native mouse probe passed.");
    }
}
'@
if ($ChildDesktop) {
    if ($ChildDesktop -cnotmatch '^MimiQa[a-f0-9]{32}$' -or -not $Receipt) {
        throw 'The isolated child needs the controller-generated desktop and receipt.'
    }
    try {
        [MimiIsolatedInputProbe]::RunChild($ChildDesktop)
        Set-Content -LiteralPath $Receipt -Value 'Child observed both owned-window mouse messages.'
        if ($MimiExecutable) {
            $env:MIMI_SMOKE_CAPTURE_POINTER_LOG = '1'
            $smoke = Join-Path $PSScriptRoot 'windows-ci-smoke.ps1'
            & $smoke -ExecutablePath $MimiExecutable -ExpectedArchitecture arm64 -OverlayInteraction pointer |
                Out-File -LiteralPath $Receipt -Append
            Add-Content -LiteralPath $Receipt -Value 'Mimi ARM64 native mouse smoke completed on the isolated desktop.'
        }
        exit 0
    }
    catch {
        Add-Content -LiteralPath $Receipt -Value ($_.Exception.GetBaseException().Message)
        exit 1
    }
}
if ($env:MIMI_SMOKE_DESKTOP_CHILD -eq '1') {
    $childReceipt = Join-Path ([IO.Path]::GetTempPath()) "mimi-desktop-probe-$([guid]::NewGuid().ToString('N')).txt"
    if ($MimiExecutable) { $MimiExecutable = (Resolve-Path -LiteralPath $MimiExecutable).Path }
    try {
        [MimiIsolatedInputProbe]::Run((Join-Path $PSHOME 'pwsh.exe'), $PSCommandPath, $childReceipt, $MimiExecutable)
    }
    finally {
        if (Test-Path -LiteralPath $childReceipt -PathType Leaf) {
            Remove-Item -LiteralPath $childReceipt -Force
        }
    }
} else {
    [MimiIsolatedInputProbe]::Run('', '', '', '')
}
