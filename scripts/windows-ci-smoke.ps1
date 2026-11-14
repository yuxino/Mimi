param(
    [Parameter(Mandatory = $true)]
    [string]$ExecutablePath,

    [Parameter(Mandatory = $true)]
    [ValidateSet('x64', 'arm64')]
    [string]$ExpectedArchitecture,

    [ValidateSet('none', 'pointer', 'accessibility')]
    [string]$OverlayInteraction = 'none',

    [ValidateRange(1, 30)]
    [int]$StartupSeconds = 5,

    # Cold Windows runners can spend several seconds loading WebView2 or the
    # cross-architecture emulation layer before a secondary reaches handoff.
    [ValidateRange(1, 60)]
    [int]$HandoffExitSeconds = 15,

    # Cross-architecture native acceptance can need longer to initialize the
    # emulation layer. CI keeps the ten-second fail-closed bound by default.
    [ValidateRange(1, 60)]
    [int]$FailureExitSeconds = 10
)

$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class MimiWindowSmoke {
    private delegate bool EnumWindowsCallback(IntPtr window, IntPtr parameter);
    private delegate IntPtr WindowProcedure(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);

    [StructLayout(LayoutKind.Sequential)]
    private struct WindowClassEx {
        internal uint Size;
        internal uint Style;
        internal IntPtr WindowProcedure;
        internal int ClassExtra;
        internal int WindowExtra;
        internal IntPtr Instance;
        internal IntPtr Icon;
        internal IntPtr Cursor;
        internal IntPtr Background;
        internal IntPtr MenuName;
        internal IntPtr ClassName;
        internal IntPtr SmallIcon;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct SecurityAttributes {
        internal int Length;
        internal IntPtr SecurityDescriptor;
        [MarshalAs(UnmanagedType.Bool)]
        internal bool InheritHandle;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct NativePoint {
        internal int X;
        internal int Y;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct NativeMessage {
        internal IntPtr Window;
        internal uint Message;
        internal UIntPtr WParam;
        internal IntPtr LParam;
        internal uint Time;
        internal NativePoint Point;
        internal uint Private;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr CreateMutex(IntPtr attributes, bool initialOwner, string name);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr CreateEvent(
        IntPtr attributes,
        bool manualReset,
        bool initialState,
        string name
    );

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true, EntryPoint = "CreateMutexW")]
    private static extern IntPtr CreateMutexWithSecurity(
        ref SecurityAttributes attributes,
        bool initialOwner,
        string name
    );

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool ConvertStringSecurityDescriptorToSecurityDescriptor(
        string descriptor,
        uint revision,
        out IntPtr securityDescriptor,
        out uint securityDescriptorSize
    );

    [DllImport("kernel32.dll")]
    private static extern IntPtr LocalFree(IntPtr memory);

    [DllImport("kernel32.dll")]
    public static extern bool ReleaseMutex(IntPtr mutex);

    [DllImport("kernel32.dll")]
    public static extern bool CloseHandle(IntPtr handle);

    [DllImport("user32.dll")]
    public static extern bool IsIconic(IntPtr window);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);

    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr window, int command);

    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(
        IntPtr window,
        uint message,
        IntPtr wparam,
        IntPtr lparam
    );

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsCallback callback, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr window, StringBuilder text, int maximumCount);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr GetModuleHandle(string moduleName);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern ushort RegisterClassEx(ref WindowClassEx windowClass);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateWindowEx(
        uint extendedStyle,
        string className,
        string windowName,
        uint style,
        int x,
        int y,
        int width,
        int height,
        IntPtr parent,
        IntPtr menu,
        IntPtr instance,
        IntPtr parameter
    );

    [DllImport("user32.dll")]
    private static extern bool DestroyWindow(IntPtr window);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern bool UnregisterClass(string className, IntPtr instance);

    [DllImport("user32.dll")]
    private static extern IntPtr DefWindowProc(
        IntPtr window,
        uint message,
        IntPtr wparam,
        IntPtr lparam
    );

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern uint RegisterWindowMessage(string messageName);

    [DllImport("user32.dll")]
    private static extern bool PeekMessage(
        out NativeMessage message,
        IntPtr window,
        uint filterMinimum,
        uint filterMaximum,
        uint removeMessage
    );

    [DllImport("user32.dll")]
    private static extern bool TranslateMessage(ref NativeMessage message);

    [DllImport("user32.dll")]
    private static extern IntPtr DispatchMessage(ref NativeMessage message);

    private static readonly WindowProcedure FakeWindowProcedure = FakeListenerWindowProcedure;
    private static Thread fakeWindowThread;
    private static ManualResetEvent fakeWindowReady;
    private static ManualResetEvent fakeWindowStop;
    private static Exception fakeWindowError;
    private static IntPtr fakeWindow;
    private static uint fakeActivationMessage;
    private static int fakeActivationCount;

    public static int FakeActivationCount {
        get { return Interlocked.CompareExchange(ref fakeActivationCount, 0, 0); }
    }

    public static bool StartFakeListenerWindow(
        string className,
        string windowName,
        string activationMessageName
    ) {
        if (fakeWindowThread != null) {
            return false;
        }
        fakeWindowReady = new ManualResetEvent(false);
        fakeWindowStop = new ManualResetEvent(false);
        fakeWindowError = null;
        fakeWindow = IntPtr.Zero;
        fakeActivationMessage = 0;
        Interlocked.Exchange(ref fakeActivationCount, 0);
        fakeWindowThread = new Thread(delegate() {
            IntPtr classNamePointer = IntPtr.Zero;
            IntPtr instance = IntPtr.Zero;
            bool registered = false;
            try {
                instance = GetModuleHandle(null);
                if (instance == IntPtr.Zero) {
                    throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                }
                fakeActivationMessage = RegisterWindowMessage(activationMessageName);
                if (fakeActivationMessage == 0) {
                    throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                }
                classNamePointer = Marshal.StringToHGlobalUni(className);
                WindowClassEx windowClass = new WindowClassEx {
                    Size = (uint)Marshal.SizeOf<WindowClassEx>(),
                    WindowProcedure = Marshal.GetFunctionPointerForDelegate(FakeWindowProcedure),
                    Instance = instance,
                    ClassName = classNamePointer
                };
                if (RegisterClassEx(ref windowClass) == 0) {
                    throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                }
                registered = true;
                fakeWindow = CreateWindowEx(
                    0x08000080,
                    className,
                    windowName,
                    0,
                    0,
                    0,
                    0,
                    0,
                    IntPtr.Zero,
                    IntPtr.Zero,
                    instance,
                    IntPtr.Zero
                );
                if (fakeWindow == IntPtr.Zero) {
                    throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                }
                fakeWindowReady.Set();
                NativeMessage message;
                while (!fakeWindowStop.WaitOne(10)) {
                    while (PeekMessage(out message, IntPtr.Zero, 0, 0, 1)) {
                        TranslateMessage(ref message);
                        DispatchMessage(ref message);
                    }
                }
            }
            catch (Exception error) {
                fakeWindowError = error;
                fakeWindowReady.Set();
            }
            finally {
                if (fakeWindow != IntPtr.Zero) {
                    DestroyWindow(fakeWindow);
                    fakeWindow = IntPtr.Zero;
                }
                if (registered) {
                    UnregisterClass(className, instance);
                }
                if (classNamePointer != IntPtr.Zero) {
                    Marshal.FreeHGlobal(classNamePointer);
                }
            }
        });
        fakeWindowThread.IsBackground = true;
        fakeWindowThread.Start();
        return fakeWindowReady.WaitOne(5000) && fakeWindowError == null && fakeWindow != IntPtr.Zero;
    }

    public static bool StopFakeListenerWindow() {
        if (fakeWindowThread == null) {
            return true;
        }
        fakeWindowStop.Set();
        bool stopped = fakeWindowThread.Join(5000);
        fakeWindowThread = null;
        fakeWindowReady.Dispose();
        fakeWindowStop.Dispose();
        fakeWindowReady = null;
        fakeWindowStop = null;
        return stopped;
    }

    private static IntPtr FakeListenerWindowProcedure(
        IntPtr window,
        uint message,
        IntPtr wparam,
        IntPtr lparam
    ) {
        if (fakeActivationMessage != 0 && message == fakeActivationMessage) {
            Interlocked.Increment(ref fakeActivationCount);
            return new IntPtr(1);
        }
        return DefWindowProc(window, message, wparam, lparam);
    }

    public static IntPtr CreateSynchronizeOnlyMutex(string name) {
        IntPtr descriptor;
        uint descriptorSize;
        if (!ConvertStringSecurityDescriptorToSecurityDescriptor(
            "D:(A;;0x00100000;;;WD)",
            1,
            out descriptor,
            out descriptorSize
        )) {
            return IntPtr.Zero;
        }
        try {
            SecurityAttributes attributes = new SecurityAttributes {
                Length = Marshal.SizeOf<SecurityAttributes>(),
                SecurityDescriptor = descriptor,
                InheritHandle = false
            };
            return CreateMutexWithSecurity(ref attributes, true, name);
        }
        finally {
            LocalFree(descriptor);
        }
    }

    public static IntPtr FindUniqueWindow(uint processId, string title) {
        IntPtr match = IntPtr.Zero;
        int matchCount = 0;
        EnumWindowsCallback callback = delegate(IntPtr window, IntPtr parameter) {
            uint ownerProcessId;
            GetWindowThreadProcessId(window, out ownerProcessId);
            if (ownerProcessId != processId) {
                return true;
            }
            StringBuilder windowTitle = new StringBuilder(256);
            GetWindowText(window, windowTitle, windowTitle.Capacity);
            if (String.Equals(windowTitle.ToString(), title, StringComparison.Ordinal)) {
                match = window;
                matchCount++;
            }
            return true;
        };
        EnumWindows(callback, IntPtr.Zero);
        return matchCount == 1 ? match : IntPtr.Zero;
    }
}
'@

function Get-PeArchitecture([string]$Path) {
    $stream = [System.IO.File]::OpenRead($Path)
    $reader = [System.IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5A4D) {
            throw "Not a PE executable: $Path"
        }
        $stream.Position = 0x3C
        $peOffset = $reader.ReadUInt32()
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "Missing PE signature: $Path"
        }
        switch ($reader.ReadUInt16()) {
            0x8664 { return 'x64' }
            0xAA64 { return 'arm64' }
            default { throw "Unsupported PE machine type in $Path" }
        }
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

function Get-MatchingMimiProcesses([string]$Path) {
    @(
        Get-Process -Name 'mimi' -ErrorAction SilentlyContinue |
            Where-Object {
                try {
                    [string]::Equals($_.Path, $Path, [StringComparison]::OrdinalIgnoreCase)
                }
                catch {
                    $false
                }
            }
    )
}

function Stop-SmokeProcess($TargetProcess) {
    if ($null -eq $TargetProcess) {
        return
    }
    try {
        if (-not $TargetProcess.HasExited) {
            Stop-Process -Id $TargetProcess.Id -Force -ErrorAction SilentlyContinue
        }
    }
    catch {
        # The process may have exited between the state check and termination.
    }
    try {
        $TargetProcess.WaitForExit(5000) | Out-Null
    }
    catch {
        # Cleanup is best-effort; the exact-path residual check catches leaks.
    }
}

function Reset-SmokeFrontendMarkers([string]$Directory) {
    foreach ($label in @('settings', 'overlay')) {
        $marker = Join-Path $Directory $label
        Remove-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $marker) {
            throw "Could not reset the $label frontend readiness marker."
        }
    }
}

function Wait-SmokeFrontendReady($TargetProcess, [string]$Directory) {
    $settingsMarker = Join-Path $Directory 'settings'
    $overlayMarker = Join-Path $Directory 'overlay'
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($TargetProcess.HasExited) {
            throw "mimi exited before both frontends mounted with code $($TargetProcess.ExitCode)."
        }
        if ((Test-Path -LiteralPath $settingsMarker -PathType Leaf) -and
            (Test-Path -LiteralPath $overlayMarker -PathType Leaf)) {
            return
        }
        Start-Sleep -Milliseconds 100
    }
    $settingsReady = Test-Path -LiteralPath $settingsMarker -PathType Leaf
    $overlayReady = Test-Path -LiteralPath $overlayMarker -PathType Leaf
    throw "mimi frontends did not mount within 30 seconds (settings=$settingsReady overlay=$overlayReady)."
}

$resolvedExecutable = (Resolve-Path -LiteralPath $ExecutablePath).Path
$actualArchitecture = Get-PeArchitecture $resolvedExecutable
if ($actualArchitecture -ne $ExpectedArchitecture) {
    throw "Expected a $ExpectedArchitecture executable, found $actualArchitecture."
}

$previousUiTest = $env:MIMI_UI_TEST
$previousStandardOverlay = $env:MIMI_UI_TEST_STANDARD_OVERLAY
$previousAutoStart = $env:MIMI_AUTO_START
$previousStartupGateDelay = $env:MIMI_UI_TEST_STARTUP_GATE_DELAY_MS
$previousStartupGateReadyFile = $env:MIMI_UI_TEST_STARTUP_GATE_READY_FILE
$previousSessionStateFile = $env:MIMI_UI_TEST_SESSION_STATE_FILE
$previousTrayVisibleFile = $env:MIMI_UI_TEST_TRAY_VISIBLE_FILE
$previousSettingsActivationFile = $env:MIMI_UI_TEST_SETTINGS_ACTIVATION_FILE
$previousFrontendReadyDir = $env:MIMI_UI_TEST_FRONTEND_READY_DIR
$previousPreferencesDir = $env:MIMI_UI_TEST_PREFERENCES_DIR
$startupGateReadyFile = Join-Path ([IO.Path]::GetTempPath()) "mimi-startup-$([Guid]::NewGuid().ToString('N')).ready"
$sessionStateFile = Join-Path ([IO.Path]::GetTempPath()) "mimi-session-$([Guid]::NewGuid().ToString('N')).state"
$trayVisibleFile = Join-Path ([IO.Path]::GetTempPath()) "mimi-tray-$([Guid]::NewGuid().ToString('N')).ready"
$settingsActivationFile = Join-Path ([IO.Path]::GetTempPath()) "mimi-settings-activation-$([Guid]::NewGuid().ToString('N')).state"
$frontendReadyDir = Join-Path ([IO.Path]::GetTempPath()) "mimi-frontend-$([Guid]::NewGuid().ToString('N'))"
$preferencesDir = $null
$process = $null
$spoofedProcess = $null
$spoofedPluginMutex = [IntPtr]::Zero
$spoofedRestrictedMutex = [IntPtr]::Zero
$spoofedPluginEvent = [IntPtr]::Zero
$fakeListenerStarted = $false
$secondaryProcesses = @()
try {
    $env:MIMI_UI_TEST = '1'
    $env:MIMI_UI_TEST_STANDARD_OVERLAY = '1'
    $env:MIMI_AUTO_START = '1'
    $env:MIMI_UI_TEST_SESSION_STATE_FILE = $sessionStateFile
    $env:MIMI_UI_TEST_TRAY_VISIBLE_FILE = $trayVisibleFile
    $env:MIMI_UI_TEST_SETTINGS_ACTIVATION_FILE = $settingsActivationFile
    $env:MIMI_UI_TEST_FRONTEND_READY_DIR = $frontendReadyDir
    if ($OverlayInteraction -ne 'none') {
        $preferencesDir = Join-Path ([IO.Path]::GetTempPath()) "mimi-ui-test-$([Guid]::NewGuid().ToString('N'))"
        New-Item -ItemType Directory -Path $preferencesDir | Out-Null
        Set-Content -LiteralPath (Join-Path $preferencesDir 'preferences.json') -Value '{"ui_language":"en"}' -Encoding ascii
        $env:MIMI_UI_TEST_PREFERENCES_DIR = $preferencesDir
    }

    if (@(Get-MatchingMimiProcesses $resolvedExecutable).Count -ne 0) {
        throw 'A matching mimi process was already running before the smoke test.'
    }

    # A same-session process can pre-create the plugin's mutex without its
    # hidden activation window. Mimi must fail closed in that state instead of
    # starting an unguarded full instance.
    $env:MIMI_UI_TEST_STARTUP_GATE_DELAY_MS = $null
    $env:MIMI_UI_TEST_STARTUP_GATE_READY_FILE = $null
    $spoofedPluginMutex = [MimiWindowSmoke]::CreateMutex(
        [IntPtr]::Zero,
        $true,
        'app.yuxino.mimi-sim'
    )
    if ($spoofedPluginMutex -eq [IntPtr]::Zero) {
        throw 'Could not create the single-instance spoof mutex.'
    }
    $spoofedProcess = Start-Process -FilePath $resolvedExecutable -PassThru
    if (-not $spoofedProcess.WaitForExit($FailureExitSeconds * 1000)) {
        throw 'mimi failed open when the plugin mutex had no activation listener.'
    }
    if ($spoofedProcess.ExitCode -eq 0) {
        throw 'mimi reported success without owning a valid single-instance listener.'
    }
    [MimiWindowSmoke]::ReleaseMutex($spoofedPluginMutex) | Out-Null
    [MimiWindowSmoke]::CloseHandle($spoofedPluginMutex) | Out-Null
    $spoofedPluginMutex = [IntPtr]::Zero

    # A matching listener owned by another executable must never receive
    # activation data or be accepted as Mimi's primary. The local protocol
    # compares executable file identities before sending its payload-free
    # registered message.
    $spoofedPluginMutex = [MimiWindowSmoke]::CreateMutex(
        [IntPtr]::Zero,
        $true,
        'app.yuxino.mimi-sim'
    )
    if ($spoofedPluginMutex -eq [IntPtr]::Zero) {
        throw 'Could not create the fake-listener mutex.'
    }
    $fakeListenerStarted = [MimiWindowSmoke]::StartFakeListenerWindow(
        'app.yuxino.mimi-sic',
        'app.yuxino.mimi-siw',
        'app.yuxino.mimi-activation-v1'
    )
    if (-not $fakeListenerStarted) {
        throw 'Could not create the external fake single-instance listener.'
    }
    $spoofedProcess = Start-Process -FilePath $resolvedExecutable -PassThru
    if (-not $spoofedProcess.WaitForExit($FailureExitSeconds * 1000)) {
        throw 'mimi did not fail closed for a listener owned by another executable.'
    }
    if ($spoofedProcess.ExitCode -eq 0) {
        throw 'mimi accepted a single-instance listener owned by another executable.'
    }
    if ([MimiWindowSmoke]::FakeActivationCount -ne 0) {
        throw 'mimi sent activation to a listener owned by another executable.'
    }
    if (-not [MimiWindowSmoke]::StopFakeListenerWindow()) {
        throw 'The external fake single-instance listener did not stop.'
    }
    $fakeListenerStarted = $false
    [MimiWindowSmoke]::ReleaseMutex($spoofedPluginMutex) | Out-Null
    [MimiWindowSmoke]::CloseHandle($spoofedPluginMutex) | Out-Null
    $spoofedPluginMutex = [IntPtr]::Zero

    # CreateMutexW requests MUTEX_ALL_ACCESS when a named mutex already exists.
    # A synchronize-only DACL therefore makes the plugin's call fail even
    # though the object type is correct; Mimi must detect that permission gap.
    $spoofedRestrictedMutex = [MimiWindowSmoke]::CreateSynchronizeOnlyMutex(
        'app.yuxino.mimi-sim'
    )
    if ($spoofedRestrictedMutex -eq [IntPtr]::Zero) {
        throw 'Could not create the access-restricted single-instance mutex.'
    }
    $spoofedProcess = Start-Process -FilePath $resolvedExecutable -PassThru
    if (-not $spoofedProcess.WaitForExit($FailureExitSeconds * 1000)) {
        throw 'mimi failed open when the plugin mutex denied its required access.'
    }
    if ($spoofedProcess.ExitCode -eq 0) {
        throw 'mimi reported success without full access to the plugin mutex.'
    }
    [MimiWindowSmoke]::ReleaseMutex($spoofedRestrictedMutex) | Out-Null
    [MimiWindowSmoke]::CloseHandle($spoofedRestrictedMutex) | Out-Null
    $spoofedRestrictedMutex = [IntPtr]::Zero

    # CreateMutexW also fails when another kind of kernel object owns the same
    # name. The upstream plugin treats that error as successful ownership, so
    # Mimi independently validates the object type before it can run.
    $spoofedPluginEvent = [MimiWindowSmoke]::CreateEvent(
        [IntPtr]::Zero,
        $true,
        $false,
        'app.yuxino.mimi-sim'
    )
    if ($spoofedPluginEvent -eq [IntPtr]::Zero) {
        throw 'Could not create the single-instance wrong-type kernel object.'
    }
    $spoofedProcess = Start-Process -FilePath $resolvedExecutable -PassThru
    if (-not $spoofedProcess.WaitForExit($FailureExitSeconds * 1000)) {
        throw 'mimi failed open when the plugin mutex name belonged to another object type.'
    }
    if ($spoofedProcess.ExitCode -eq 0) {
        throw 'mimi reported success without a valid plugin mutex object.'
    }
    [MimiWindowSmoke]::CloseHandle($spoofedPluginEvent) | Out-Null
    $spoofedPluginEvent = [IntPtr]::Zero

    # Hold the application-owned startup gate before Tauri initializes its
    # single-instance hidden window. Immediate contenders deterministically
    # exercise the cold-start race instead of waiting for a warm primary.
    $env:MIMI_UI_TEST_STARTUP_GATE_DELAY_MS = '1500'
    $env:MIMI_UI_TEST_STARTUP_GATE_READY_FILE = $startupGateReadyFile
    Reset-SmokeFrontendMarkers $frontendReadyDir
    $process = Start-Process -FilePath $resolvedExecutable -PassThru

    $gateReadyDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not (Test-Path -LiteralPath $startupGateReadyFile -PathType Leaf) -and [DateTime]::UtcNow -lt $gateReadyDeadline) {
        if ($process.HasExited) {
            throw "The intended primary mimi process exited before acquiring the startup gate with code $($process.ExitCode)."
        }
        Start-Sleep -Milliseconds 25
    }
    if (-not (Test-Path -LiteralPath $startupGateReadyFile -PathType Leaf)) {
        throw 'The intended primary mimi process did not acquire the startup gate.'
    }

    $coldLaunchDeadlines = @{}
    1..4 | ForEach-Object {
        $startedAt = [DateTime]::UtcNow
        $secondaryProcess = Start-Process -FilePath $resolvedExecutable -PassThru
        $secondaryProcesses += $secondaryProcess
        $coldLaunchDeadlines[$secondaryProcess.Id] = $startedAt.AddSeconds($HandoffExitSeconds)
    }
    foreach ($secondaryProcess in $secondaryProcesses) {
        # Each process gets the original deadline from its own launch, not a
        # fresh fifteen seconds after waiting for the previous contender.
        $deadline = $coldLaunchDeadlines[$secondaryProcess.Id]
        $remainingMs = [int][Math]::Max(0, [Math]::Ceiling(($deadline - [DateTime]::UtcNow).TotalMilliseconds))
        if (-not $secondaryProcess.WaitForExit($remainingMs) -or $secondaryProcess.ExitTime.ToUniversalTime() -gt $deadline) {
            $remaining = @(Get-MatchingMimiProcesses $resolvedExecutable)
            Write-Output "Handoff timeout: primaryExited=$($process.HasExited), pendingPid=$($secondaryProcess.Id), matchingProcesses=$($remaining.Count)."
            throw "A concurrent mimi launch did not hand off and exit within $HandoffExitSeconds seconds."
        }
        if ($secondaryProcess.ExitCode -ne 0) {
            throw "A concurrent mimi process exited with code $($secondaryProcess.ExitCode)."
        }
    }
    $env:MIMI_UI_TEST_STARTUP_GATE_DELAY_MS = $null
    $env:MIMI_UI_TEST_STARTUP_GATE_READY_FILE = $null

    if ($process.WaitForExit($StartupSeconds * 1000)) {
        throw "mimi exited during the $StartupSeconds-second startup smoke test with code $($process.ExitCode)."
    }
    # Native window titles and backend listening do not prove that the packaged
    # lazy imports, IPC capabilities and React surfaces mounted successfully.
    Wait-SmokeFrontendReady $process $frontendReadyDir

    $settingsWindow = [IntPtr]::Zero
    $settingsDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while ($settingsWindow -eq [IntPtr]::Zero -and [DateTime]::UtcNow -lt $settingsDeadline) {
        $settingsWindow = [MimiWindowSmoke]::FindUniqueWindow(
            [uint32]$process.Id,
            'mimi UI test settings'
        )
        if ($settingsWindow -eq [IntPtr]::Zero) {
            Start-Sleep -Milliseconds 100
        }
    }
    if ($settingsWindow -eq [IntPtr]::Zero) {
        throw 'The primary mimi process did not expose exactly one UI-test settings window.'
    }
    $sessionDeadline = [DateTime]::UtcNow.AddSeconds(5)
    $sessionState = $null
    while ($sessionState -ne 'listening' -and [DateTime]::UtcNow -lt $sessionDeadline) {
        if (Test-Path -LiteralPath $sessionStateFile -PathType Leaf) {
            $sessionState = [IO.File]::ReadAllText($sessionStateFile).Trim()
        }
        if ($sessionState -ne 'listening') {
            Start-Sleep -Milliseconds 100
        }
    }
    if ($sessionState -ne 'listening') {
        throw "The UI-test session did not reach listening state (last state: $sessionState)."
    }
    [MimiWindowSmoke]::ShowWindow($settingsWindow, 6) | Out-Null
    $minimizeDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not [MimiWindowSmoke]::IsIconic($settingsWindow) -and [DateTime]::UtcNow -lt $minimizeDeadline) {
        Start-Sleep -Milliseconds 100
    }
    if (-not [MimiWindowSmoke]::IsIconic($settingsWindow)) {
        throw 'The settings window could not be minimized for the activation smoke test.'
    }

    Remove-Item -LiteralPath $settingsActivationFile -Force -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $settingsActivationFile) {
        throw 'Could not reset the settings activation marker before the minimized-window handoff.'
    }
    $warmProcess = Start-Process -FilePath $resolvedExecutable -PassThru
    $secondaryProcesses += $warmProcess
    if (-not $warmProcess.WaitForExit($HandoffExitSeconds * 1000)) {
        throw "A second mimi launch did not hand off and exit within $HandoffExitSeconds seconds."
    }
    if ($warmProcess.ExitCode -ne 0) {
        throw "The secondary mimi process exited with code $($warmProcess.ExitCode)."
    }

    $restoreDeadline = [DateTime]::UtcNow.AddSeconds(5)
    $activationState = $null
    while (([MimiWindowSmoke]::IsIconic($settingsWindow) -or -not [MimiWindowSmoke]::IsWindowVisible($settingsWindow) -or $activationState -ne 'focus-requested') -and [DateTime]::UtcNow -lt $restoreDeadline) {
        if (Test-Path -LiteralPath $settingsActivationFile -PathType Leaf) {
            $activationState = [IO.File]::ReadAllText($settingsActivationFile).Trim()
        }
        Start-Sleep -Milliseconds 100
    }
    if ([MimiWindowSmoke]::IsIconic($settingsWindow) -or -not [MimiWindowSmoke]::IsWindowVisible($settingsWindow)) {
        $isIconic = [MimiWindowSmoke]::IsIconic($settingsWindow)
        $isVisible = [MimiWindowSmoke]::IsWindowVisible($settingsWindow)
        $foregroundWindow = [MimiWindowSmoke]::GetForegroundWindow()
        throw "A repeated launch did not restore the minimized settings window (iconic=$isIconic visible=$isVisible foreground=0x$($foregroundWindow.ToInt64().ToString('X')) expected=0x$($settingsWindow.ToInt64().ToString('X')))."
    }
    if ($activationState -ne 'focus-requested') {
        throw "A repeated launch restored the minimized settings window without completing its focus request (activation=$activationState)."
    }

    # The Windows close button intentionally moves Settings to the tray. The
    # process must stay alive, and another launch must restore the same native
    # window instead of creating a second application instance.
    if (-not [MimiWindowSmoke]::PostMessage($settingsWindow, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) {
        throw 'Could not send WM_CLOSE to the settings window.'
    }
    $closeDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([MimiWindowSmoke]::IsWindowVisible($settingsWindow) -and [DateTime]::UtcNow -lt $closeDeadline) {
        if ($process.HasExited) {
            throw 'WM_CLOSE exited mimi instead of moving Settings to the tray.'
        }
        Start-Sleep -Milliseconds 100
    }
    if ([MimiWindowSmoke]::IsWindowVisible($settingsWindow)) {
        throw 'WM_CLOSE did not hide the settings window.'
    }
    $trayDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not (Test-Path -LiteralPath $trayVisibleFile -PathType Leaf) -and [DateTime]::UtcNow -lt $trayDeadline) {
        Start-Sleep -Milliseconds 100
    }
    if (-not (Test-Path -LiteralPath $trayVisibleFile -PathType Leaf) -or [IO.File]::ReadAllText($trayVisibleFile).Trim() -ne 'visible') {
        throw 'WM_CLOSE hid Settings without confirming the native tray icon was visible.'
    }
    $sessionRetentionDeadline = [DateTime]::UtcNow.AddSeconds(1)
    while ([DateTime]::UtcNow -lt $sessionRetentionDeadline) {
        if (-not (Test-Path -LiteralPath $sessionStateFile -PathType Leaf) -or [IO.File]::ReadAllText($sessionStateFile).Trim() -ne 'listening') {
            throw 'WM_CLOSE stopped the active subtitle session instead of keeping it resident.'
        }
        Start-Sleep -Milliseconds 100
    }

    Remove-Item -LiteralPath $settingsActivationFile -Force -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $settingsActivationFile) {
        throw 'Could not reset the settings activation marker before the tray-hidden handoff.'
    }
    $closeRestoreProcess = Start-Process -FilePath $resolvedExecutable -PassThru
    $secondaryProcesses += $closeRestoreProcess
    if (-not $closeRestoreProcess.WaitForExit($HandoffExitSeconds * 1000)) {
        throw "A launch after closing Settings did not hand off within $HandoffExitSeconds seconds."
    }
    if ($closeRestoreProcess.ExitCode -ne 0) {
        throw "The launch after closing Settings exited with code $($closeRestoreProcess.ExitCode)."
    }

    $closeRestoreDeadline = [DateTime]::UtcNow.AddSeconds(5)
    $activationState = $null
    while (([MimiWindowSmoke]::IsIconic($settingsWindow) -or -not [MimiWindowSmoke]::IsWindowVisible($settingsWindow) -or $activationState -ne 'focus-requested') -and [DateTime]::UtcNow -lt $closeRestoreDeadline) {
        if (Test-Path -LiteralPath $settingsActivationFile -PathType Leaf) {
            $activationState = [IO.File]::ReadAllText($settingsActivationFile).Trim()
        }
        Start-Sleep -Milliseconds 100
    }
    if ([MimiWindowSmoke]::IsIconic($settingsWindow) -or -not [MimiWindowSmoke]::IsWindowVisible($settingsWindow)) {
        throw 'A repeated launch did not restore the tray-hidden settings window.'
    }
    if ($activationState -ne 'focus-requested') {
        throw "A repeated launch restored the tray-hidden settings window without completing its focus request (activation=$activationState)."
    }
    $restoredSettingsWindow = [MimiWindowSmoke]::FindUniqueWindow(
        [uint32]$process.Id,
        'mimi UI test settings'
    )
    if ($restoredSettingsWindow -ne $settingsWindow) {
        throw 'Restoring tray-hidden Settings did not reuse the original native window.'
    }
    if (-not (Test-Path -LiteralPath $sessionStateFile -PathType Leaf) -or [IO.File]::ReadAllText($sessionStateFile).Trim() -ne 'listening') {
        throw 'Restoring tray-hidden Settings did not preserve the active subtitle session.'
    }

    if ($OverlayInteraction -ne 'none') {
        $overlayWindow = [MimiWindowSmoke]::FindUniqueWindow([uint32]$process.Id, 'Mimi Subtitles')
        if ($overlayWindow -eq [IntPtr]::Zero -or -not [MimiWindowSmoke]::IsWindowVisible($overlayWindow)) {
            throw 'The primary did not expose its visible subtitle overlay for the interaction check.'
        }
        Remove-Item -LiteralPath $settingsActivationFile -Force
        # Windows PowerShell ships with the desktop UI Automation assemblies.
        # A stalled accessibility provider must not block package verification.
        $interactionProbe = [Diagnostics.ProcessStartInfo]::new('powershell.exe')
        $interactionProbe.UseShellExecute = $false
        foreach ($argument in @('-NoProfile', '-NonInteractive', '-Mta', '-File', "$PSScriptRoot/windows-overlay-interaction-smoke.ps1",
            '-OverlayWindow', $overlayWindow.ToInt64(), '-SettingsWindow', $settingsWindow.ToInt64(),
            '-SessionStateFile', $sessionStateFile, '-SettingsActivationFile', $settingsActivationFile,
            '-Interaction', $OverlayInteraction)) {
            $interactionProbe.ArgumentList.Add([string]$argument)
        }
        $probeProcess = [Diagnostics.Process]::Start($interactionProbe)
        try {
            if (-not $probeProcess.WaitForExit(95000)) {
                $probeProcess.Kill()
                $probeProcess.WaitForExit(5000) | Out-Null
                throw "The Windows overlay $OverlayInteraction check did not finish."
            }
            if ($probeProcess.ExitCode -ne 0) {
                throw "The Windows overlay $OverlayInteraction check exited with code $($probeProcess.ExitCode)."
            }
        }
        finally {
            $probeProcess.Dispose()
        }
    }

    $matchingProcesses = @(Get-MatchingMimiProcesses $resolvedExecutable)
    if ($matchingProcesses.Count -ne 1 -or $matchingProcesses[0].Id -ne $process.Id) {
        throw "Expected the original mimi process $($process.Id) to be the only matching instance."
    }

    # Only restart the exact-path synthetic primary after all native handoff
    # checks. Clear one-shot markers: secondary handoffs reuse their WebViews,
    # but a new process must prove its own frontend mount and automatic start.
    Stop-SmokeProcess $process
    $restartExitDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while (@(Get-MatchingMimiProcesses $resolvedExecutable).Count -ne 0 -and [DateTime]::UtcNow -lt $restartExitDeadline) {
        Start-Sleep -Milliseconds 100
    }
    if (@(Get-MatchingMimiProcesses $resolvedExecutable).Count -ne 0) {
        throw 'The original smoke primary did not exit before the restart check.'
    }
    Reset-SmokeFrontendMarkers $frontendReadyDir
    Remove-Item -LiteralPath $sessionStateFile -Force -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $sessionStateFile) {
        throw 'Could not reset the session marker before the automatic-start restart.'
    }
    $process = Start-Process -FilePath $resolvedExecutable -PassThru
    Wait-SmokeFrontendReady $process $frontendReadyDir
    $restartSessionDeadline = [DateTime]::UtcNow.AddSeconds(30)
    $restartSessionState = $null
    while ($restartSessionState -ne 'listening' -and [DateTime]::UtcNow -lt $restartSessionDeadline) {
        if ($process.HasExited) {
            throw "The restarted mimi process exited before automatic start with code $($process.ExitCode)."
        }
        if (Test-Path -LiteralPath $sessionStateFile -PathType Leaf) {
            try {
                $restartSessionState = [IO.File]::ReadAllText($sessionStateFile).Trim()
            }
            catch [IO.IOException] {
                # The native publisher may still be writing this tiny marker.
                $restartSessionState = $null
            }
        }
        if ($restartSessionState -ne 'listening') {
            Start-Sleep -Milliseconds 100
        }
    }
    if ($restartSessionState -ne 'listening') {
        throw "The restarted session did not reach fresh automatic-start listening within 30 seconds (last state: $restartSessionState)."
    }
    $restartedSettingsWindow = [MimiWindowSmoke]::FindUniqueWindow(
        [uint32]$process.Id,
        'mimi UI test settings'
    )
    if ($restartedSettingsWindow -eq [IntPtr]::Zero -or -not [MimiWindowSmoke]::IsWindowVisible($restartedSettingsWindow)) {
        throw 'The restarted process did not expose exactly one visible UI-test settings window.'
    }
    $matchingProcesses = @(Get-MatchingMimiProcesses $resolvedExecutable)
    if ($matchingProcesses.Count -ne 1 -or $matchingProcesses[0].Id -ne $process.Id) {
        throw 'The restarted smoke primary was not the only matching mimi process.'
    }
    if ($process.WaitForExit($StartupSeconds * 1000)) {
        throw "The restarted mimi process exited during its $StartupSeconds-second health check with code $($process.ExitCode)."
    }

    Write-Output "mimi $actualArchitecture UI-test process passed cold-start, verified-listener handoff, tray/session retention, minimized/tray-hidden activation, fresh frontend mount/restart, and $StartupSeconds-second health checks."
}
finally {
    try {
        Stop-SmokeProcess $spoofedProcess
        [MimiWindowSmoke]::StopFakeListenerWindow() | Out-Null
        if ($spoofedPluginMutex -ne [IntPtr]::Zero) {
            [MimiWindowSmoke]::ReleaseMutex($spoofedPluginMutex) | Out-Null
            [MimiWindowSmoke]::CloseHandle($spoofedPluginMutex) | Out-Null
        }
        if ($spoofedRestrictedMutex -ne [IntPtr]::Zero) {
            [MimiWindowSmoke]::ReleaseMutex($spoofedRestrictedMutex) | Out-Null
            [MimiWindowSmoke]::CloseHandle($spoofedRestrictedMutex) | Out-Null
        }
        if ($spoofedPluginEvent -ne [IntPtr]::Zero) {
            [MimiWindowSmoke]::CloseHandle($spoofedPluginEvent) | Out-Null
        }
        foreach ($secondaryProcess in $secondaryProcesses) {
            Stop-SmokeProcess $secondaryProcess
        }
        Stop-SmokeProcess $process

        # Stop-Process can return before a cross-architecture process has fully
        # disappeared from the process table. Retry only exact-path candidates
        # so a smoke failure never leaves this build running or touches an
        # installed Mimi from another location.
        $cleanupDeadline = [DateTime]::UtcNow.AddSeconds(15)
        $remainingProcesses = @(Get-MatchingMimiProcesses $resolvedExecutable)
        while ($remainingProcesses.Count -ne 0 -and [DateTime]::UtcNow -lt $cleanupDeadline) {
            foreach ($remainingProcess in $remainingProcesses) {
                Stop-SmokeProcess $remainingProcess
            }
            Start-Sleep -Milliseconds 100
            $remainingProcesses = @(Get-MatchingMimiProcesses $resolvedExecutable)
        }
        if ($remainingProcesses.Count -ne 0) {
            $remainingProcessIds = ($remainingProcesses | ForEach-Object { $_.Id }) -join ', '
            throw "The smoke test left matching mimi processes behind (PIDs: $remainingProcessIds)."
        }
    }
    finally {
        Remove-Item -LiteralPath $startupGateReadyFile -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $sessionStateFile -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $trayVisibleFile -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $settingsActivationFile -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $frontendReadyDir -Recurse -Force -ErrorAction SilentlyContinue
        if ($null -ne $preferencesDir) {
            Remove-Item -LiteralPath $preferencesDir -Recurse -Force -ErrorAction SilentlyContinue
        }
        $env:MIMI_UI_TEST = $previousUiTest
        $env:MIMI_UI_TEST_STANDARD_OVERLAY = $previousStandardOverlay
        $env:MIMI_AUTO_START = $previousAutoStart
        $env:MIMI_UI_TEST_STARTUP_GATE_DELAY_MS = $previousStartupGateDelay
        $env:MIMI_UI_TEST_STARTUP_GATE_READY_FILE = $previousStartupGateReadyFile
        $env:MIMI_UI_TEST_SESSION_STATE_FILE = $previousSessionStateFile
        $env:MIMI_UI_TEST_TRAY_VISIBLE_FILE = $previousTrayVisibleFile
        $env:MIMI_UI_TEST_SETTINGS_ACTIVATION_FILE = $previousSettingsActivationFile
        $env:MIMI_UI_TEST_FRONTEND_READY_DIR = $previousFrontendReadyDir
        $env:MIMI_UI_TEST_PREFERENCES_DIR = $previousPreferencesDir
    }
}
