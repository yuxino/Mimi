# The readiness marker belongs to this parent script, before the native probe
# process is launched. Its marker reader is not in the parent's function scope.
function Test-HistoryProbeReady([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
    try { return [IO.File]::ReadAllText($Path).Trim() -ceq 'ready' }
    catch [IO.IOException] { return $false }
}

# The package smoke owns the executable and private fixture directory. Each
# launch must mount both WebViews again before its localized controls are used.
function Invoke-LocalizedOverlaySession([hashtable]$Preferences, [string]$EvidencePath, [string]$HistoryScenario = 'normal') {
    if (@(Get-MatchingMimiProcesses $resolvedExecutable).Count -ne 0) {
        throw 'The previous test process must exit before a fresh locale is started.'
    }
    Reset-SmokeFrontendMarkers $frontendReadyDir
    $historyTrace = $OverlayInteraction -eq 'caption-history'
    if ($historyTrace) {
        foreach ($name in @('history-pointer-probe-enabled', 'history-pointer-probe-ready', 'history-pointer-probe.jsonl', 'history-pointer-probe-busy-180-enabled', 'history-pointer-probe-fast-release-enabled')) {
            Remove-Item -LiteralPath (Join-Path $frontendReadyDir $name) -Force -ErrorAction SilentlyContinue
        }
        [IO.File]::WriteAllBytes((Join-Path $frontendReadyDir 'history-pointer-probe-enabled'), [byte[]]@())
        if ($HistoryScenario -eq 'busy180') {
            [IO.File]::WriteAllBytes((Join-Path $frontendReadyDir 'history-pointer-probe-busy-180-enabled'), [byte[]]@())
        }
        if ($HistoryScenario -eq 'fast-release') {
            [IO.File]::WriteAllBytes((Join-Path $frontendReadyDir 'history-pointer-probe-fast-release-enabled'), [byte[]]@())
        }
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
    [void][IO.Directory]::CreateDirectory($EvidenceDirectory)
    $manifest = [ordered]@{ productBase = '810e5a15'; binarySha256 = $binaryHash
        scenarios = @('normal', 'fast-release', 'busy180'); locales = @('zh', 'de')
        busyDurationMs = 180; busyGapPx = @(2, 70); stopAtFirstFailure = $true }
    [IO.File]::WriteAllText((Join-Path $EvidenceDirectory 'observation-matrix.json'), ($manifest | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
    Write-Output "History observation uses one diagnostic executable SHA-256 $binaryHash."
    Add-Type -Path (Join-Path $PSScriptRoot 'CaptionDragDesktop.cs')
    $env:MIMI_UI_TEST_CAPTION_HISTORY = '1'
    $null = New-Item -ItemType Directory -Force -Path $frontendReadyDir
    # No retries: preserve the first red, including a failed normal control.
    foreach ($scenario in @('normal', 'fast-release', 'busy180')) {
        foreach ($locale in @('zh', 'de')) {
            $casePath = Join-Path $EvidenceDirectory "$scenario/$locale"
            $evidence = New-Item -ItemType Directory -Force -Path $casePath
            Write-Output "History observation scenario=$scenario locale=$locale."
            Invoke-LocalizedOverlaySession @{
                ui_language = $locale
                source_language = 'en'
                target_language = 'zh'
                subtitle_display_mode = 'bilingual'
                subtitle_animation = $false
                subtitle_background_opacity = 100
                audio_input = 'system'
            } $evidence.FullName $scenario
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
