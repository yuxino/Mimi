param(
    [Parameter(Mandatory = $true)][string]$ExecutablePath,
    [ValidateRange(1, 15)][int]$Samples = 5
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase

function Wait-StartupCondition($Process, [string]$Phase, [scriptblock]$Find) {
    $wait = [Diagnostics.Stopwatch]::StartNew()
    while ($wait.Elapsed.TotalSeconds -lt 30) {
        if ($Process.HasExited) {
            throw "Mimi exited before $Phase (exit code $($Process.ExitCode))."
        }
        try {
            $found = & $Find
            if ($found) { return $found }
        }
        catch [System.Windows.Automation.ElementNotAvailableException] {
            # The native accessibility tree can change during the first render.
        }
        Start-Sleep -Milliseconds 15
    }
    throw "Mimi did not reach $Phase within 30 seconds."
}

function Find-MimiWindow([int]$ProcessId, [string]$Name) {
    $condition = [System.Windows.Automation.AndCondition]::new(
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $ProcessId),
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, $Name))
    $window = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst(
        [System.Windows.Automation.TreeScope]::Children, $condition)
    if ($window -and -not $window.Current.IsOffscreen) { return $window }
}

$exe = (Resolve-Path -LiteralPath $ExecutablePath).Path
if (Get-Process -Name 'mimi' -ErrorAction SilentlyContinue) {
    throw 'The startup measurement requires an isolated desktop with no running Mimi instance.'
}

$switchCondition = [System.Windows.Automation.AndCondition]::new(
    [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, 'Live Subtitles'),
    [System.Windows.Automation.OrCondition]::new(
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::IsTogglePatternAvailableProperty, $true),
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::IsInvokePatternAvailableProperty, $true)))
$root = Join-Path ([IO.Path]::GetTempPath()) "mimi-ui-test-startup-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $root | Out-Null
try {
    # The existing restart fixture provides an in-memory demo profile. Only
    # the language and the synthetic run markers are written to this directory.
    Set-Content -LiteralPath (Join-Path $root 'preferences.json') -Value '{"ui_language":"en"}' -Encoding ascii
    $startInfo = [Diagnostics.ProcessStartInfo]::new($exe)
    $startInfo.UseShellExecute = $false
    $startInfo.EnvironmentVariables['MIMI_UI_TEST'] = '1'
    $startInfo.EnvironmentVariables['MIMI_UI_TEST_STANDARD_OVERLAY'] = '1'
    $startInfo.EnvironmentVariables['MIMI_UI_TEST_PREFERENCES_DIR'] = $root
    $startInfo.EnvironmentVariables.Remove('MIMI_AUTO_START')
    $startInfo.EnvironmentVariables.Remove('MIMI_UI_TEST_STARTUP_GATE_DELAY_MS')
    $startInfo.EnvironmentVariables.Remove('MIMI_UI_TEST_STARTUP_GATE_READY_FILE')

    for ($sample = 1; $sample -le $Samples; $sample++) {
        $readyDirectory = Join-Path $root "ready-$sample"
        $stateFile = Join-Path $root "state-$sample"
        $startInfo.EnvironmentVariables['MIMI_UI_TEST_FRONTEND_READY_DIR'] = $readyDirectory
        $startInfo.EnvironmentVariables['MIMI_UI_TEST_SESSION_STATE_FILE'] = $stateFile
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $process = [Diagnostics.Process]::Start($startInfo)
        try {
            $settings = Wait-StartupCondition $process 'visible Settings window' {
                Find-MimiWindow $process.Id 'mimi UI test settings'
            }
            $windowMs = $clock.Elapsed.TotalMilliseconds
            $startControl = Wait-StartupCondition $process 'enabled Live Subtitles switch' {
                $button = $settings.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $switchCondition)
                if ($button -and $button.Current.IsEnabled -and -not $button.Current.IsOffscreen) {
                    $toggle = $null
                    if ($button.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$toggle)) {
                        if ($toggle.Current.ToggleState -ne [System.Windows.Automation.ToggleState]::Off) {
                            throw 'The subtitle switch was already on before the explicit start.'
                        }
                        return $toggle
                    }
                    $invoke = $null
                    if ($button.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$invoke)) {
                        return $invoke
                    }
                }
            }
            $settingsMs = $clock.Elapsed.TotalMilliseconds
            if (Test-Path -LiteralPath $stateFile) {
                $initial = [IO.File]::ReadAllText($stateFile).Trim()
                if ($initial -and $initial -ne 'idle') { throw "Mimi started before the switch action ($initial)." }
            }

            $clock.Restart()
            if ($startControl -is [System.Windows.Automation.TogglePattern]) { $startControl.Toggle() }
            else { $startControl.Invoke() }
            Wait-StartupCondition $process 'listening session' {
                if (Test-Path -LiteralPath $stateFile -PathType Leaf) {
                    try { [IO.File]::ReadAllText($stateFile).Trim() -eq 'listening' }
                    catch [IO.IOException] { $false } # The native writer can still own the file.
                }
            } | Out-Null
            $listeningMs = $clock.Elapsed.TotalMilliseconds
            Wait-StartupCondition $process 'visible overlay with a rendered listening state' {
                (Test-Path -LiteralPath (Join-Path $readyDirectory 'overlay') -PathType Leaf) -and
                    ($null -ne (Find-MimiWindow $process.Id 'Mimi Subtitles'))
            } | Out-Null
            $overlayMs = $clock.Elapsed.TotalMilliseconds
            Write-Output ('MimiStartup sample={0} window_ms={1:F1} settings_ms={2:F1} listening_ms={3:F1} overlay_ms={4:F1}' -f
                $sample, $windowMs, $settingsMs, $listeningMs, $overlayMs)
        }
        finally {
            try {
                if (-not $process.HasExited) { $process.Kill() }
                if (-not $process.WaitForExit(5000)) { throw 'The isolated startup sample failed to exit.' }
            }
            finally { $process.Dispose() }
        }
    }
}
finally { Remove-Item -LiteralPath $root -Recurse -Force }
