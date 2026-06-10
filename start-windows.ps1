param(
  [int]$Port = 8766,
  [switch]$NoBrowser
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$artifactsDir = Join-Path $root "artifacts"
$logsDir = Join-Path $root "logs"
$pidFile = Join-Path $artifactsDir "viewer_pid.txt"
$stdout = Join-Path $logsDir "viewer_stdout.log"
$stderr = Join-Path $logsDir "viewer_stderr.log"
$bundledSolver = Join-Path $root "bin\\windows\\item-waterway-solver.exe"
$cargoManifest = Join-Path $root "rust-backend\\Cargo.toml"
$builtSolver = Join-Path $root "rust-backend\\target\\release\\item-waterway-solver.exe"

New-Item -ItemType Directory -Force -Path $artifactsDir | Out-Null
New-Item -ItemType Directory -Force -Path $logsDir | Out-Null

function Resolve-OptionalCommand {
  param([string[]]$Candidates)
  foreach ($candidate in $Candidates) {
    if (-not $candidate) { continue }
    $command = Get-Command $candidate -ErrorAction SilentlyContinue
    if ($command) {
      return $command.Source
    }
  }
  return $null
}

function Get-LatestSourceTick {
  param([string]$RootPath)
  if (-not (Test-Path -LiteralPath $RootPath)) {
    return 0L
  }
  $latest = 0L
  Get-ChildItem -LiteralPath $RootPath -Recurse -File | ForEach-Object {
    if ($_.LastWriteTimeUtc.Ticks -gt $latest) {
      $latest = $_.LastWriteTimeUtc.Ticks
    }
  }
  return $latest
}

function Build-RustSolver {
  $cargo = Resolve-OptionalCommand @($env:CARGO, "cargo")
  if (-not $cargo) {
    throw "cargo is unavailable, and no usable bundled solver was found."
  }
  & $cargo build --release --bin item-waterway-solver --manifest-path $cargoManifest
  if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed."
  }
  if (-not (Test-Path -LiteralPath $builtSolver)) {
    throw "Built solver was not produced at $builtSolver"
  }
  return (Resolve-Path -LiteralPath $builtSolver).Path
}

function Resolve-SolverPath {
  if (Test-Path -LiteralPath $bundledSolver) {
    $sourceTick = Get-LatestSourceTick (Join-Path $root "rust-backend\\src")
    $bundledTick = (Get-Item -LiteralPath $bundledSolver).LastWriteTimeUtc.Ticks
    if ($sourceTick -gt $bundledTick -and (Test-Path -LiteralPath $cargoManifest)) {
      $cargo = Resolve-OptionalCommand @($env:CARGO, "cargo")
      if ($cargo) {
        return Build-RustSolver
      }
      Write-Warning "Bundled solver is older than rust-backend/src, but cargo is unavailable. Falling back to bundled solver."
    }
    return (Resolve-Path -LiteralPath $bundledSolver).Path
  }
  if (Test-Path -LiteralPath $builtSolver) {
    return (Resolve-Path -LiteralPath $builtSolver).Path
  }
  if (Test-Path -LiteralPath $cargoManifest) {
    return Build-RustSolver
  }
  throw "Rust solver is unavailable."
}

if (Test-Path -LiteralPath $pidFile) {
  $existingPid = Get-Content -LiteralPath $pidFile | Select-Object -First 1
  if ($existingPid) {
    $running = Get-Process -Id ([int]$existingPid) -ErrorAction SilentlyContinue
    if ($running) {
      Write-Output ("Viewer already running at http://127.0.0.1:{0} (PID {1})" -f $Port, $existingPid)
      exit 0
    }
  }
  Remove-Item -LiteralPath $pidFile -ErrorAction SilentlyContinue
}

$solver = Resolve-SolverPath

$env:WATERWAY_HOME = $root
$env:WATERWAY_APP_DIR = $root
$env:WATERWAY_DATA_DIR = Join-Path $root "data"
$env:MC_VIEWER_DATA_DIR = Join-Path $root "data\\viewer_data"
$env:MC_VIEWER_STATIC_DIR = Join-Path $root "viewer"
$env:WATERWAY_ASSET_DIR = Join-Path $root "assets\\minecraft\\textures\\block"
$env:WATERWAY_PARTS_CONFIG = Join-Path $root "model\\config\\waterway-structure-parts.json"
$env:WATERWAY_SOLVER = $solver
$env:MC_VIEWER_HOST = "127.0.0.1"
$env:MC_VIEWER_PORT = [string]$Port

$process = Start-Process -FilePath $solver `
  -ArgumentList @("serve-web") `
  -WorkingDirectory $root `
  -WindowStyle Hidden `
  -RedirectStandardOutput $stdout `
  -RedirectStandardError $stderr `
  -PassThru

Start-Sleep -Milliseconds 800
if ($process.HasExited) {
  $stderrTail = if (Test-Path -LiteralPath $stderr) {
    (Get-Content -LiteralPath $stderr -Tail 40 -ErrorAction SilentlyContinue) -join [Environment]::NewLine
  } else {
    ""
  }
  throw ("Viewer failed to start." + $(if ($stderrTail) { "`n$stderrTail" } else { "" }))
}

Set-Content -LiteralPath $pidFile -Value $process.Id
Write-Output ("Started viewer PID {0}" -f $process.Id)
Write-Output ("Viewer URL http://127.0.0.1:{0}" -f $Port)

if (-not $NoBrowser) {
  Start-Process ("http://127.0.0.1:{0}" -f $Port) | Out-Null
}
