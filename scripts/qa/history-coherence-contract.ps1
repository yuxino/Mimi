$ErrorActionPreference = 'Stop'
$scriptPath = Join-Path $PSScriptRoot '../windows/tests/overlay-caption-history.ps1'
$tokens = $null; $parseErrors = $null
$null = [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path $scriptPath), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
Add-Type -AssemblyName WindowsBase
Add-Type -ReferencedAssemblies WindowsBase -TypeDefinition @'
using System;
public static class MimiPointerSmoke {
    public static System.Windows.Rect ReadBounds(IntPtr ignored) {
        return new System.Windows.Rect(222, 408, 580, 240);
    }
}
'@
. $scriptPath

# These contract doubles do not enumerate windows or dispatch any input.
function Read-CaptionHistoryPosition($Context) { return $script:positions.Dequeue() }
function Get-CaptionHistoryRows($Context) {
    return @{ translation10 = @{ Box = [System.Windows.Rect]::new(320, $script:rowTop, 378, 21)
        Area = 7938; Name = $script:rowName; Offscreen = $false } }
}
function Position([double]$Top, [double]$Height, [double]$Percent = 60, [double]$Size = 20) {
    return @{ View = [System.Windows.Rect]::new(234, $Top, 556, $Height); Percent = $Percent; ViewSize = $Size }
}
function Queue-Positions($Before, $After) {
    $script:positions = [Collections.Queue]::new()
    $script:positions.Enqueue($Before); $script:positions.Enqueue($After)
}
function Require([bool]$Passed, [string]$Message) { if (-not $Passed) { throw $Message } }
function Require-AnchorFailure($Before, $After) {
    $failed = $false
    try { Assert-CaptionHistoryAnchor $Before $After 'translation10' }
    catch { $failed = $true }
    Require $failed 'The unchanged anchor assertion accepted a stable wrong sample.'
}
$context = @{ Window = [IntPtr]::Zero; Anchor = 'translation10'; RejectedSamples = 0; FirstRejectedSamples = [ordered]@{} }
$script:rowName = 'Synthetic caption lane'
$script:rowTop = 467
Queue-Positions (Position 460 150) (Position 460 150)
$before = Get-CaptionHistoryView $context
Require ($null -ne $before) 'Stable hover snapshot rejected.'

$script:rowTop = 427
Queue-Positions (Position 460 150 58 16) (Position 420 190 62 20)
Require ($null -eq (Get-CaptionHistoryView $context 'quiet-reading')) 'Mixed viewport snapshot accepted.'
Require ($context.FirstRejectedSamples['quiet-reading'].anchorBounds[1] -eq 427) 'First mixed anchor evidence was lost.'

Queue-Positions (Position 420 190 62 20) (Position 420 190 63 20)
Require ($null -eq (Get-CaptionHistoryView $context 'quiet-reading')) 'Changing scroll percent accepted.'
Queue-Positions (Position 420 190 62 20) (Position 420 190 62 21)
Require ($null -eq (Get-CaptionHistoryView $context 'quiet-reading')) 'Changing visible-content percentage accepted.'
Require ($context.RejectedSamples -eq 3 -and $context.FirstRejectedSamples.Count -eq 1) 'First rejected sample was overwritten or rejection count changed.'

Queue-Positions (Position 420 190) (Position 420 190)
$quiet = Get-CaptionHistoryView $context 'quiet-reading'
Assert-CaptionHistoryAnchor $before $quiet 'translation10'

$script:rowTop = 467
Queue-Positions (Position 420 190) (Position 420 190)
$wrong = Wait-CaptionHistory 'one coherent contract snapshot' { Get-CaptionHistoryView $context }
Require-AnchorFailure $before $wrong
Require ($script:positions.Count -eq 0) 'Coherent sampling retried toward the expected anchor.'

$script:rowTop = 427; $script:rowName = 'Different synthetic caption lane'
Queue-Positions (Position 420 190) (Position 420 190)
Require-AnchorFailure $before (Get-CaptionHistoryView $context)
Write-Host 'Coherent snapshot contract passed: mixed bounds/percent/view-size rejected; first evidence retained; coherent correct accepted; coherent wrong position/text still fail unchanged assertions.'
