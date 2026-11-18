param(
  [Parameter(Mandatory = $true)][string]$ExecutablePath,
  [Parameter(Mandatory = $true)][string]$Version,
  [Parameter(Mandatory = $true)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
  throw "Invalid portable package version: $Version"
}
$executable = (Resolve-Path -LiteralPath $ExecutablePath).Path
if ((Split-Path -Leaf $executable) -ne 'mimi.exe') {
  throw 'Expected the release mimi.exe executable.'
}

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$tempRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
$packageDir = Join-Path $tempRoot 'mimi-portable-package'
$verifyDir = Join-Path $tempRoot 'mimi-portable-verify'
Remove-Item -LiteralPath $packageDir, $verifyDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $packageDir | Out-Null
Copy-Item -LiteralPath $executable -Destination (Join-Path $packageDir 'mimi.exe')
New-Item -ItemType File -Path (Join-Path $packageDir 'mimi.portable') | Out-Null

$archive = Join-Path $OutputDirectory "mimi_${Version}_x64-portable.zip"
Compress-Archive -Path (Join-Path $packageDir '*') -DestinationPath $archive -Force
Expand-Archive -LiteralPath $archive -DestinationPath $verifyDir -Force
$entries = @(Get-ChildItem -LiteralPath $verifyDir -Force)
$actualNames = @($entries | ForEach-Object { $_.Name } | Sort-Object)
if ($entries.Count -ne 2 -or (Compare-Object $actualNames @('mimi.exe', 'mimi.portable'))) {
  throw 'Portable archive must contain only mimi.exe and mimi.portable.'
}
if (-not (Test-Path -LiteralPath (Join-Path $verifyDir 'mimi.exe') -PathType Leaf) -or
    -not (Test-Path -LiteralPath (Join-Path $verifyDir 'mimi.portable') -PathType Leaf)) {
  throw 'Portable archive contains an unexpected file type.'
}
if ((Get-FileHash -LiteralPath (Join-Path $verifyDir 'mimi.exe')).Hash -ne
    (Get-FileHash -LiteralPath $executable).Hash) {
  throw 'Portable executable does not match the Windows release build.'
}
Write-Output $archive
