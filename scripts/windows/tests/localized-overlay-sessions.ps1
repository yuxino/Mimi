# The readiness marker belongs to this parent script, before the native probe
# process is launched. Its marker reader is not in the parent's function scope.
function Test-HistoryProbeReady([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
    try { return [IO.File]::ReadAllText($Path).Trim() -ceq 'ready' }
    catch [IO.IOException] { return $false }
}

# The package smoke owns the executable and private fixture directory. Each
# launch must mount both WebViews again before its localized controls are used.
function Invoke-LocalizedOverlaySession([hashtable]$Preferences, [string]$EvidencePath) {
    if (@(Get-MatchingMimiProcesses $resolvedExecutable).Count -ne 0) {
        throw 'The previous test process must exit before a fresh locale is started.'
    }
    Reset-SmokeFrontendMarkers $frontendReadyDir
    $historyTrace = $OverlayInteraction -eq 'caption-history'
    if ($historyTrace) {
        foreach ($name in @('history-pointer-probe-enabled', 'history-pointer-probe-ready', 'history-pointer-probe.jsonl')) {
            Remove-Item -LiteralPath (Join-Path $frontendReadyDir $name) -Force -ErrorAction SilentlyContinue
        }
        [IO.File]::WriteAllBytes((Join-Path $frontendReadyDir 'history-pointer-probe-enabled'), [byte[]]@())
    }
    Remove-Item -LiteralPath $sessionStateFile, $settingsActivationFile -Force -ErrorAction SilentlyContinue
    $Preferences | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $preferencesDir 'preferences.json') -Encoding utf8NoBOM
    $primary = Start-Process -FilePath $resolvedExecutable -PassThru
    try {
        Wait-SmokeFrontendReady $primary $frontendReadyDir
        if ($historyTrace) {
            $traceReady = Join-Path $frontendReadyDir 'history-pointer-probe-ready'
            $deadline = [Diagnostics.Stopwatch]::StartNew()
            while (-not (Test-HistoryProbeReady $traceReady) -and $deadline.ElapsedMilliseconds -lt 5000) {
                if ($primary.HasExited) { throw 'History trace app exited before its observer was ready.' }
                Start-Sleep -Milliseconds 25
            }
            if (-not (Test-HistoryProbeReady $traceReady)) { throw 'History metadata observer was not ready within 5000ms.' }
        }
        $settingsWindow = [MimiWindowSmoke]::FindUniqueWindow([uint32]$primary.Id, 'mimi UI test settings')
        if ($settingsWindow -eq [IntPtr]::Zero) { throw 'The mounted Settings window could not be identified.' }
        Invoke-OverlayInteractionProbe $primary $settingsWindow $OverlayInteraction 60000 -EvidencePath $EvidencePath -Locale $Preferences.ui_language
    }
    finally {
        Stop-SmokeProcess $primary
        if ($historyTrace) {
            $trace = Join-Path $frontendReadyDir 'history-pointer-probe.jsonl'
            if (Test-Path -LiteralPath $trace -PathType Leaf) {
                Copy-Item -LiteralPath $trace -Destination (Join-Path $EvidencePath 'history-pointer-probe.jsonl') -Force
            }
        }
        if (-not $primary.HasExited) { throw 'The fixture is still running; the next language cannot start.' }
        $primary.Dispose()
    }
}

function Test-CaptionHistorySessions {
    $binaryHash = (Get-FileHash -LiteralPath $resolvedExecutable -Algorithm SHA256).Hash
    if ($binaryHash -cne '1714D0FE84ED2BEED893A0B12FA7A91A4C2ED31F10F5E30719D7953B2EE42E22') {
        throw 'This bounded history observation requires the unchanged diagnostic executable from run 44894562404.'
    }
    [void][IO.Directory]::CreateDirectory($EvidenceDirectory)
    $manifest = [ordered]@{ productBase = '810e5a15'; binarySource = '8ffb58aa2017c03ff88b3664fd742248520b039c'
        binarySha256 = $binaryHash; reusedRun = 44894562404; cycles = 3; locales = @('zh', 'de')
        preconditions = @('pointer-both-drag-preferences', 'nine-language-panels'); stopAtFirstFailure = $true }
    [IO.File]::WriteAllText((Join-Path $EvidenceDirectory 'observation-matrix.json'), ($manifest | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))

    # Match the bundle job's two preceding checks on this same hosted desktop.
    # Separate PowerShell processes preserve the outer fixture environment and
    # the original scripts' exact input, private-directory and cleanup rules.
    $smokeScript = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../windows-ci-smoke.ps1'))
    $probeArguments = @('-NoProfile', '-NonInteractive', '-File', $smokeScript,
        '-ExecutablePath', $resolvedExecutable, '-ExpectedArchitecture', 'x64')
    Write-Output "History observation uses unchanged executable SHA-256 $binaryHash."
    Write-Output 'History observation precondition: native pointer controls and both Windows drag preferences.'
    & (Join-Path $PSHOME 'pwsh.exe') @probeArguments -OverlayInteraction pointer -VerifyBothDragModes 2>&1 |
        Tee-Object -FilePath (Join-Path $EvidenceDirectory 'precondition-pointer.log')
    if ($LASTEXITCODE -ne 0) { throw 'The pointer precondition failed; no history cycle was started.' }
    Write-Output 'History observation precondition: all nine native panel languages.'
    $panelEvidence = Join-Path $EvidenceDirectory 'precondition-panels'
    & (Join-Path $PSHOME 'pwsh.exe') @probeArguments -OverlayInteraction panel-languages -EvidenceDirectory $panelEvidence 2>&1 |
        Tee-Object -FilePath (Join-Path $EvidenceDirectory 'precondition-panels.log')
    if ($LASTEXITCODE -ne 0) { throw 'The panel precondition failed; no history cycle was started.' }

    Add-Type -Path (Join-Path $PSScriptRoot 'CaptionDragDesktop.cs')
    $env:MIMI_UI_TEST_CAPTION_HISTORY = '1'
    $null = New-Item -ItemType Directory -Force -Path $frontendReadyDir
    # Do not catch a failed session and continue: keep the first red's trace,
    # screenshot and original failure rather than searching for a green run.
    foreach ($cycle in 1..3) {
        foreach ($locale in @('zh', 'de')) {
            $casePath = Join-Path $EvidenceDirectory ('cycle-{0:00}/{1}' -f $cycle, $locale)
            $evidence = New-Item -ItemType Directory -Force -Path $casePath
            Write-Output "History observation cycle=$cycle locale=$locale after both native preconditions."
            Invoke-LocalizedOverlaySession @{
                ui_language = $locale
                source_language = 'en'
                target_language = 'zh'
                subtitle_display_mode = 'bilingual'
                subtitle_animation = $false
                subtitle_background_opacity = 100
                audio_input = 'system'
            } $evidence.FullName
        }
    }
}

function Test-PanelLanguageSessions {
    $labelsPath = Join-Path $preferencesDir 'panel-languages.json'
    node (Join-Path $PSScriptRoot '../../frontend/write-panel-language-fixtures.mjs') $labelsPath
    if ($LASTEXITCODE -ne 0) { throw 'Could not read the panel language catalog.' }
    $languages = [IO.File]::ReadAllText($labelsPath) | ConvertFrom-Json
    $null = New-Item -ItemType Directory -Force -Path $frontendReadyDir, $EvidenceDirectory
    $env:MIMI_UI_TEST_DUAL_SUBTITLES = $null
    $env:MIMI_UI_TEST_DUAL_LIVE_SUBTITLES = $null
    $env:MIMI_UI_TEST_STARTUP_GATE_DELAY_MS = $null
    $env:MIMI_UI_TEST_STARTUP_GATE_READY_FILE = $null
    $sha = (Get-FileHash -LiteralPath $resolvedExecutable -Algorithm SHA256).Hash
    Write-Output "Panel language review for Windows x64 executable SHA-256 $sha."

    $failures = @()
    foreach ($labels in $languages) {
        $locale = $labels.locale
        $labels | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $preferencesDir 'panel-labels.json') -Encoding utf8NoBOM
        Write-Output "Starting panel review for $locale."
        try {
            Invoke-LocalizedOverlaySession @{
                ui_language = $locale
                source_language = 'en'
                target_language = 'zh_tw'
                subtitle_display_mode = 'bilingual'
            } $EvidenceDirectory
        }
        catch {
            $failures += $locale
            Write-Warning "Panel review failed for ${locale}: $_"
        }
    }
    if ($failures.Count) { throw "Panel language review failed for $($failures -join ', ')." }
    Write-Output "Verified $($languages.Count) Windows panel languages on the same executable."
}
