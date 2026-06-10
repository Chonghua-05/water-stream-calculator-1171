$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$pidFile = Join-Path $root "artifacts\\viewer_pid.txt"

if (-not (Test-Path -LiteralPath $pidFile)) {
  Write-Output "No viewer PID file found."
  exit 0
}

$viewerPid = Get-Content -LiteralPath $pidFile | Select-Object -First 1
if ($viewerPid) {
  Stop-Process -Id ([int]$viewerPid) -ErrorAction SilentlyContinue
}

Remove-Item -LiteralPath $pidFile -ErrorAction SilentlyContinue
Write-Output "Stopped viewer."
