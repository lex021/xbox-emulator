param(
    [Parameter(Mandatory = $true)][string]$RunDir,
    [double]$ThresholdMs = 300,
    [double]$After = 0,
    [int]$MaxShots = 2,
    [double]$CooldownSec = 5,
    [double]$ConsumeTimeoutSec = 20,
    [double]$IdleExitSec = 180,
    [string]$CsvPath = '',
    [string]$NowPath = ''
)

$ErrorActionPreference = 'Stop'
if (-not $CsvPath) { $CsvPath = Join-Path $RunDir 'live\tombox_frames.csv' }
if (-not $NowPath) { $NowPath = Join-Path $RunDir 'live\passcap\now' }
$logPath = Join-Path $RunDir 'passcap_on_burst.txt'

function Write-Burst([string]$m) {
    $line = '[{0}] {1}' -f (Get-Date -Format 'HH:mm:ss'), $m
    Write-Output $line
    Add-Content -LiteralPath $logPath -Value $line -Encoding utf8
}

function Test-RunOver {
    Test-Path -LiteralPath (Join-Path $RunDir 'result.json')
}

Write-Burst ("watching {0} threshold {1} ms after {2} s max {3} shots trigger {4}" -f $CsvPath, $ThresholdMs, $After, $MaxShots, $NowPath)

$start = Get-Date
while (-not (Test-Path -LiteralPath $CsvPath)) {
    if ((Test-RunOver) -or ((Get-Date) - $start).TotalSeconds -gt $IdleExitSec) {
        Write-Burst 'frame CSV never appeared (is TOMBOX_FRAME_CSV=1 set?); exiting'
        exit 2
    }
    Start-Sleep -Milliseconds 250
}

$fs = [System.IO.File]::Open($CsvPath, 'Open', 'Read', 'ReadWrite,Delete')
$sr = New-Object System.IO.StreamReader($fs)
$header = $null
$iT = -1; $iI = -1; $iF = -1
$shots = 0
$cooldownUntil = [datetime]::MinValue
$pendingSince = $null
$lastGrowth = Get-Date
$partial = ''

while ($shots -lt $MaxShots -or $pendingSince) {
    $chunk = $sr.ReadToEnd()
    if ($chunk.Length -gt 0) {
        $lastGrowth = Get-Date
        $text = $partial + $chunk
        $lines = $text -split "`n"
        $partial = $lines[-1]
        foreach ($raw in $lines[0..($lines.Length - 2)]) {
            $l = $raw.TrimEnd("`r")
            if (-not $l) { continue }
            if (-not $header) {
                $header = $l.Split(',')
                $iF = [array]::IndexOf($header, 'frame')
                $iT = [array]::IndexOf($header, 't_ms')
                $iI = [array]::IndexOf($header, 'present_interval_ms')
                if ($iT -lt 0 -or $iI -lt 0) { Write-Burst "unexpected CSV header: $l"; exit 3 }
                continue
            }
            if ($shots -ge $MaxShots -or $pendingSince -or (Get-Date) -lt $cooldownUntil) { continue }
            $c = $l.Split(',')
            if ($c.Length -le $iI) { continue }
            $t = 0.0; $iv = 0.0
            if (-not [double]::TryParse($c[$iT], [ref]$t)) { continue }
            if (-not [double]::TryParse($c[$iI], [ref]$iv)) { continue }
            if ($t / 1000.0 -lt $After) { continue }
            if ($iv -gt $ThresholdMs) {
                $dir = Split-Path -Parent $NowPath
                if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
                Set-Content -LiteralPath $NowPath -Value ("frame {0} t_ms {1} interval_ms {2}" -f $c[$iF], $c[$iT], $c[$iI]) -Encoding ascii
                $shots++
                $pendingSince = Get-Date
                Write-Burst ("shot {0}: frame {1} at t={2:N1} s took {3:N1} ms > {4} ms; touched {5}" -f $shots, $c[$iF], ($t / 1000.0), $iv, $ThresholdMs, $NowPath)
            }
        }
    }
    if ($pendingSince) {
        if (-not (Test-Path -LiteralPath $NowPath)) {
            Write-Burst ("shot {0}: trigger consumed by the game after {1:N2} s" -f $shots, ((Get-Date) - $pendingSince).TotalSeconds)
            $pendingSince = $null
            $cooldownUntil = (Get-Date).AddSeconds($CooldownSec)
        } elseif (((Get-Date) - $pendingSince).TotalSeconds -gt $ConsumeTimeoutSec) {
            Remove-Item -LiteralPath $NowPath -Force -ErrorAction SilentlyContinue
            Write-Burst ("shot {0}: trigger not consumed within {1} s (passcap not armed with 'file', or game not presenting); removed it so it cannot fire late" -f $shots, $ConsumeTimeoutSec)
            $pendingSince = $null
            $cooldownUntil = (Get-Date).AddSeconds($CooldownSec)
        }
    }
    if ($shots -ge $MaxShots -and -not $pendingSince) { break }
    if ((Test-RunOver) -and $chunk.Length -eq 0 -and -not $pendingSince) { Write-Burst 'run finished'; break }
    if (((Get-Date) - $lastGrowth).TotalSeconds -gt $IdleExitSec -and -not $pendingSince) { Write-Burst "CSV idle for $IdleExitSec s; exiting"; break }
    Start-Sleep -Milliseconds 100
}
$sr.Close()
Write-Burst ("done: {0} shot(s) triggered" -f $shots)
