# The package smoke owns the executable and private fixture directory. Each
# launch must mount both WebViews again before its localized controls are used.
function Invoke-LocalizedOverlaySession([hashtable]$Preferences, [string]$EvidencePath) {
    if (@(Get-MatchingMimiProcesses $resolvedExecutable).Count -ne 0) {
        throw 'The previous test process must exit before a fresh locale is started.'
    }
    Reset-SmokeFrontendMarkers $frontendReadyDir
    Remove-Item -LiteralPath $sessionStateFile, $settingsActivationFile -Force -ErrorAction SilentlyContinue
    $Preferences | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $preferencesDir 'preferences.json') -Encoding utf8NoBOM
    $primary = Start-Process -FilePath $resolvedExecutable -PassThru
    try {
        Wait-SmokeFrontendReady $primary $frontendReadyDir
        $settingsWindow = [MimiWindowSmoke]::FindUniqueWindow([uint32]$primary.Id, 'mimi UI test settings')
        if ($settingsWindow -eq [IntPtr]::Zero) { throw 'The mounted Settings window could not be identified.' }
        Invoke-OverlayInteractionProbe $primary $settingsWindow $OverlayInteraction 60000 -EvidencePath $EvidencePath -Locale $Preferences.ui_language
    }
    finally {
        Stop-SmokeProcess $primary
        if (-not $primary.HasExited) { throw 'The fixture is still running; the next language cannot start.' }
        $primary.Dispose()
    }
}

function Test-CaptionHistorySessions {
    Add-Type -Path (Join-Path $PSScriptRoot 'CaptionDragDesktop.cs')
    $env:MIMI_UI_TEST_CAPTION_HISTORY = '1'
    $null = New-Item -ItemType Directory -Force -Path $frontendReadyDir
    foreach ($locale in @('zh', 'de')) {
        $evidence = New-Item -ItemType Directory -Force -Path (Join-Path $EvidenceDirectory $locale)
        Write-Output "Checking the bilingual caption reading position in the $locale interface."
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

function Test-PanelLanguageSessions {
    Add-Type -Path (Join-Path $PSScriptRoot 'CaptionDragDesktop.cs')
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
    foreach ($labels in @($languages | Where-Object { $_.locale -ceq 'en' })) {
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
