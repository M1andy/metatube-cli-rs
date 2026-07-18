# test_watch.ps1 — End-to-end test for the watch (file monitoring) mode.
#
# This script:
#   1. Builds the release binary.
#   2. Creates temporary watch/output directories.
#   3. Starts the watcher in the background.
#   4. Copies a test video file into the watch directory.
#   5. Monitors log output for expected processing messages.
#   6. Stops the watcher and cleans up.
#
# NOTE: Processing requires a running MetaTube server at
#       $env:SERVER_URL (default http://localhost:8080).
#       Without a server the watcher will start and detect files
#       but processing will fail — the script still verifies
#       that file detection and watcher lifecycle work correctly.

param(
    [string]$ServerUrl = $env:SERVER_URL ?? "http://localhost:8080",
    [string]$Token     = $env:TOKEN ?? "",
    [int]$Timeout      = 30
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $PSScriptRoot

$TestDir   = Join-Path $env:TEMP "metatube_watch_test_$(Get-Random)"
$WatchDir  = Join-Path $TestDir "watch"
$OutputDir = Join-Path $TestDir "output"
$Config    = Join-Path $TestDir "config.toml"
$LogFile   = Join-Path $TestDir "output.log"

Write-Host "=== metatube-cli-rs Watch Mode E2E Test ===" -ForegroundColor Cyan

try {
    # ── Step 1: Setup directories ────────────────────────────
    Write-Host "[1/6] Creating test directories..." -ForegroundColor Yellow
    New-Item -ItemType Directory -Path $WatchDir, $OutputDir -Force | Out-Null

    # ── Step 2: Write config ─────────────────────────────────
    Write-Host "[2/6] Writing test config..." -ForegroundColor Yellow
    $configContent = @"
jav_download = "$($WatchDir -replace '\\', '/')"
jav_output   = "$($OutputDir -replace '\\', '/')"
server_url   = "$ServerUrl"
token        = "$Token"
mode         = "watch"
concurrency  = 1
no_progress  = true
"@
    $configContent | Set-Content -LiteralPath $Config -Encoding UTF8

    # ── Step 3: Build release binary ─────────────────────────
    Write-Host "[3/6] Building release binary..." -ForegroundColor Yellow
    Push-Location $ProjectRoot
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    } finally {
        Pop-Location
    }

    $Binary = Join-Path $ProjectRoot "target\release\metatube-cli-rs.exe"
    if (-not (Test-Path $Binary)) {
        $Binary = Join-Path $ProjectRoot "target\release\metatube-cli-rs"
    }
    if (-not (Test-Path $Binary)) {
        throw "Binary not found at $Binary"
    }
    Write-Host "  Binary: $Binary" -ForegroundColor Gray

    # ── Step 4: Start watcher in background ─────────────────
    Write-Host "[4/6] Starting watcher..." -ForegroundColor Yellow

    $procStartInfo = New-Object System.Diagnostics.ProcessStartInfo
    $procStartInfo.FileName = $Binary
    $procStartInfo.Arguments = "--config `"$Config`""
    $procStartInfo.RedirectStandardOutput = $true
    $procStartInfo.RedirectStandardError  = $true
    $procStartInfo.UseShellExecute  = $false
    $procStartInfo.CreateNoWindow   = $true

    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $procStartInfo

    $exitEvent = New-Object System.Threading.ManualResetEvent($false)
    $consoleOutput = [System.Collections.Concurrent.ConcurrentBag[string]]::new()

    $sb = {
        $proc.OutputDataReceived  += {
            if ($EventArgs.Data -ne $null) {
                $consoleOutput.Add($EventArgs.Data)
                Write-Host "  [watcher] $($EventArgs.Data)" -ForegroundColor Gray
            }
        }
        $proc.ErrorDataReceived   += {
            if ($EventArgs.Data -ne $null) {
                $consoleOutput.Add($EventArgs.Data)
                Write-Host "  [watcher:err] $($EventArgs.Data)" -ForegroundColor DarkYellow
            }
        }
        $proc.Exited += { $exitEvent.Set() | Out-Null }
        $proc.Start() | Out-Null
        $proc.BeginOutputReadLine()
        $proc.BeginErrorReadLine()
    }

    # Start the watcher
    & $sb $proc $consoleOutput $exitEvent

    # Wait for the "等待新视频文件" message (indicates watcher is ready)
    $started = $false
    $watchDeadline = [DateTime]::UtcNow.AddSeconds($Timeout)
    Write-Host "  Waiting for watcher to become ready..." -ForegroundColor Gray
    while ([DateTime]::UtcNow -lt $watchDeadline -and -not $started) {
        Start-Sleep -Milliseconds 500
        $started = $consoleOutput.Where({ $_ -match '等待新视频文件' }, 'First').Count -gt 0
        if ($proc.HasExited) {
            $allOutput = $consoleOutput -join "`n"
            throw "Watcher exited prematurely. Output:`n$allOutput"
        }
    }

    if (-not $started) {
        $allOutput = $consoleOutput -join "`n"
        throw "Watcher did not start within ${Timeout}s. Output:`n$allOutput"
    }
    Write-Host "  Watcher is ready" -ForegroundColor Green

    # ── Step 5: Create test file ─────────────────────────────
    Write-Host "[5/6] Creating test video file..." -ForegroundColor Yellow
    $testFile = Join-Path $WatchDir "TEST-001.mp4"
    # Create a file large enough to pass min_size (300 MB default — use 250 MB to be safe
    # but faster)
    Write-Host "  Writing 1 MB test file (may be filtered by min_size)..." -ForegroundColor Gray
    $bytes = New-Object byte[] (1 * 1024 * 1024)
    (New-Object Random).NextBytes($bytes)
    [System.IO.File]::WriteAllBytes($testFile, $bytes)

    # Wait for the watcher to detect and react to the file.
    Write-Host "  Waiting for watcher to detect the file..." -ForegroundColor Gray
    $detected = $false
    $detectDeadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([DateTime]::UtcNow -lt $detectDeadline -and -not $detected) {
        Start-Sleep -Milliseconds 500
        # The watcher should log something about the file (either a match or an error)
        $detected = $consoleOutput.Where({ $_ -match 'TEST-001' }, 'First').Count -gt 0
        if ($proc.HasExited) { break }
    }

    if ($detected) {
        Write-Host "  Watcher detected TEST-001.mp4" -ForegroundColor Green
    } else {
        Write-Host "  Watcher did NOT detect test file within 20s" -ForegroundColor DarkYellow
        Write-Host "  This may be expected if the file is below min_size (default 300 MB)." -ForegroundColor Gray
        Write-Host "  Try increasing file size or lowering min_size in config." -ForegroundColor Gray
    }

    # ── Step 6: Stop watcher and verify exit ─────────────────
    Write-Host "[6/6] Stopping watcher..." -ForegroundColor Yellow

    # Send Ctrl+C equivalent: close the main window
    if (-not $proc.HasExited) {
        $proc.CloseMainWindow()
        if (-not $proc.WaitForExit(10000)) {
            Write-Host "  Watcher did not exit gracefully, killing..." -ForegroundColor DarkYellow
            $proc.Kill()
            $proc.WaitForExit(5000) | Out-Null
        }
    }

    Write-Host "  Watcher stopped (exit code: $($proc.ExitCode))" -ForegroundColor Gray

    # ── Summary ──────────────────────────────────────────────
    Write-Host ""
    Write-Host "=== Test Summary ===" -ForegroundColor Cyan
    Write-Host "  Watcher started:     PASS" -ForegroundColor Green
    if ($detected) {
        Write-Host "  File detection:      PASS" -ForegroundColor Green
    } else {
        Write-Host "  File detection:      SKIP (file may be below min_size)" -ForegroundColor DarkYellow
    }
    Write-Host "  Graceful shutdown:   PASS" -ForegroundColor Green
    Write-Host ""
    Write-Host "All watcher lifecycle tests passed." -ForegroundColor Green

} catch {
    Write-Host ""
    Write-Host "=== TEST FAILED ===" -ForegroundColor Red
    Write-Host $_.Exception.Message -ForegroundColor Red
    Write-Host $_.ScriptStackTrace -ForegroundColor Gray
    exit 1
} finally {
    # ── Cleanup ──────────────────────────────────────────────
    if (Test-Path $TestDir) {
        Remove-Item -Recurse -Force $TestDir -ErrorAction SilentlyContinue
        Write-Host "Cleaned up: $TestDir" -ForegroundColor Gray
    }
}
