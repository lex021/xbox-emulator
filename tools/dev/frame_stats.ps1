param(
  [Parameter(Mandatory = $true)][string]$Csv,
  [double]$FromMs = 0,
  [double]$ToMs = 0,
  [double]$SkipSeconds = 2,
  [switch]$Json
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Csv)) { throw "no frame csv at $Csv" }
$rows = @(Import-Csv -LiteralPath $Csv)
if (-not $rows.Count) { throw "frame csv $Csv has no rows" }

function Num($v) { $d = 0.0; if ([double]::TryParse([string]$v, [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref]$d)) { return $d } return $null }

$start = $FromMs + $SkipSeconds * 1000.0
$sel = @($rows | Where-Object { $t = Num $_.t_ms; $null -ne $t -and $t -ge $start -and ($ToMs -le 0 -or $t -le $ToMs) })
if ($sel.Count -lt 10) { throw ("only {0} frames in the window {1:N0}..{2} ms of {3}" -f $sel.Count, $start, $(if ($ToMs -gt 0) { '{0:N0}' -f $ToMs } else { 'end' }), $Csv) }

function Stats([double[]]$a) {
  if (-not $a.Count) { return $null }
  $s = [double[]]($a | Sort-Object)
  $n = $s.Count
  $sum = 0.0; foreach ($x in $s) { $sum += $x }
  $p = { param($q) $s[[Math]::Min($n - 1, [Math]::Max(0, [int][Math]::Ceiling($q * $n) - 1))] }
  [ordered]@{ avg = $sum / $n; p50 = (& $p 0.50); p95 = (& $p 0.95); p99 = (& $p 0.99); max = $s[$n - 1]; n = $n }
}

$iv = [double[]]@($sel | ForEach-Object { Num $_.present_interval_ms } | Where-Object { $null -ne $_ -and $_ -gt 0 })
$cpu = [double[]]@($sel | ForEach-Object { Num $_.present_cpu_ms } | Where-Object { $null -ne $_ })
$wait = [double[]]@($sel | ForEach-Object { Num $_.fence_wait_ms } | Where-Object { $null -ne $_ })
$gpu = [double[]]@($sel | ForEach-Object { Num $_.gpu_frame_ms } | Where-Object { $null -ne $_ -and $_ -gt 0 })

$t0 = Num $sel[0].t_ms
$t1 = Num $sel[-1].t_ms
$span = ($t1 - $t0) / 1000.0
$avgFps = if ($span -gt 0) { ($sel.Count - 1) / $span } else { 0 }
$sortedDesc = [double[]]($iv | Sort-Object -Descending)
$k = [Math]::Max(1, [int][Math]::Floor($sortedDesc.Count * 0.01))
$worst = 0.0; for ($i = 0; $i -lt $k; $i++) { $worst += $sortedDesc[$i] }
$low1 = if ($worst -gt 0) { 1000.0 / ($worst / $k) } else { 0 }
$k01 = [Math]::Max(1, [int][Math]::Floor($sortedDesc.Count * 0.001))
$w01 = 0.0; for ($i = 0; $i -lt $k01; $i++) { $w01 += $sortedDesc[$i] }
$low01 = if ($w01 -gt 0) { 1000.0 / ($w01 / $k01) } else { 0 }
$hitches = @($iv | Where-Object { $_ -gt 33.4 }).Count

$si = Stats $iv; $sc = Stats $cpu; $sw = Stats $wait; $sg = Stats $gpu
$bound = 'unknown'
if ($sc -and $sw) {
  if ($sw.avg -gt 0.25 * $si.avg) { $bound = 'GPU (CPU waits on the GPU fence {0:P0} of the frame)' -f ($sw.avg / $si.avg) }
  elseif ($sc.avg -gt 0.6 * $si.avg) { $bound = 'CPU present path ({0:P0} of the frame inside PresentX)' -f ($sc.avg / $si.avg) }
  else { $bound = 'CPU title/emulator work outside PresentX' }
}

$cols = @($rows[0].PSObject.Properties.Name)
$classes = @($cols | Where-Object { $_ -match '^(.+)_ms$' -and $cols -contains ($Matches[1] + '_n') -and $_ -notmatch '^bg_' } | ForEach-Object { $_ -replace '_ms$', '' })
$inPresent = @('upload')
$offThread = @('io_read_bg')
$hitchRows = @($sel | Where-Object { $v = Num $_.present_interval_ms; $null -ne $v -and $v -gt 33.4 })
$hitchList = @()
$classTotal = [ordered]@{}
foreach ($c in $classes) { $classTotal[$c] = 0.0 }
$classTotal['present_cpu'] = 0.0
$classTotal['unattributed'] = 0.0
$hitchMs = 0.0
foreach ($h in $hitchRows) {
  $iv1 = Num $h.present_interval_ms
  $cpu1 = Num $h.present_cpu_ms; if ($null -eq $cpu1) { $cpu1 = 0.0 }
  $hitchMs += $iv1
  $parts = @()
  $blocked = 0.0
  foreach ($c in $classes) {
    $n = Num $h.($c + '_n'); $m = Num $h.($c + '_ms')
    if ($n -and $n -gt 0) {
      $parts += ('{0} {1}x {2:N1}ms' -f $c, $n, $m)
      if ($inPresent -notcontains $c -and $offThread -notcontains $c) { $blocked += $m; $classTotal[$c] += $m }
      elseif ($inPresent -contains $c) { $classTotal[$c] += $m }
    }
  }
  foreach ($x in @('bg_pso_n', 'bg_xlat_n', 'bg_dxc_n', 'qpc_calls', 'gtc_calls')) { $v = Num $h.$x; if ($v -and $v -gt 0) { $parts += ('{0} {1}' -f $x, $v) } }
  $presentOther = [Math]::Max(0.0, $cpu1 - $(if ($cols -contains 'upload_ms') { [double](Num $h.upload_ms) } else { 0.0 }))
  $classTotal['present_cpu'] += $presentOther
  $un = [Math]::Max(0.0, $iv1 - $cpu1 - $blocked)
  $classTotal['unattributed'] += $un
  $hitchList += [ordered]@{ frame = [int](Num $h.frame); t_ms = [Math]::Round((Num $h.t_ms), 0); ms = [Math]::Round($iv1, 1); present_cpu_ms = [Math]::Round($cpu1, 2); unattributed_ms = [Math]::Round($un, 1); counters = ($parts -join ', ') }
}
$dominant = $null
if ($hitchRows.Count -and $classes.Count) {
  $dominant = ($classTotal.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 1).Key
}
$ratio = $null; $gsum = 0.0; $wsum = 0.0; $gframes = 0
if ($cols -contains 'game_dt_ms') {
  foreach ($x in $sel) { $g = Num $x.game_dt_ms; $w = Num $x.present_interval_ms; if ($null -ne $g -and $null -ne $w -and $w -gt 0) { $gsum += $g; $wsum += $w; $gframes++ } }
  if ($wsum -gt 0) { $ratio = $gsum / $wsum }
}
$qpcCalls = 0.0; if ($cols -contains 'qpc_calls') { foreach ($x in $sel) { $v = Num $x.qpc_calls; if ($v) { $qpcCalls += $v } } }
$gtcCalls = 0.0; if ($cols -contains 'gtc_calls') { foreach ($x in $sel) { $v = Num $x.gtc_calls; if ($v) { $gtcCalls += $v } } }
$allTotals = [ordered]@{}
foreach ($c in $classes) { $s = 0.0; $n = 0.0; foreach ($x in $sel) { $v = Num $x.($c + '_ms'); if ($v) { $s += $v }; $k2 = Num $x.($c + '_n'); if ($k2) { $n += $k2 } }; if ($n -gt 0) { $allTotals[$c] = @{ n = $n; ms = [Math]::Round($s, 1) } } }

$r = [ordered]@{
  csv = $Csv; window_ms = @([Math]::Round($t0, 0), [Math]::Round($t1, 0)); frames = $sel.Count; seconds = [Math]::Round($span, 2)
  avg_fps = [Math]::Round($avgFps, 1); low1_fps = [Math]::Round($low1, 1); low01_fps = [Math]::Round($low01, 1)
  frame_ms = $si; present_cpu_ms = $sc; fence_wait_ms = $sw; gpu_frame_ms = $sg; hitches_over_33ms = $hitches; bound = $bound
  min60 = ($low1 -ge 60.0)
  hitch_ms_total = [Math]::Round($hitchMs, 1); hitch_class_ms = $classTotal; hitch_dominant = $dominant; hitches = $hitchList
  event_totals = $allTotals
  game_speed_ratio = $(if ($null -ne $ratio) { [Math]::Round($ratio, 4) } else { $null }); game_speed_ok = $(if ($null -ne $ratio) { $ratio -ge 0.98 -and $ratio -le 1.02 } else { $null })
  qpc_calls = $qpcCalls; gtc_calls = $gtcCalls
}
if ($Json) { $r | ConvertTo-Json -Depth 5; return }
function F($s) { if (-not $s) { 'n/a' } else { 'avg {0:N2} p50 {1:N2} p99 {2:N2} max {3:N2}' -f $s.avg, $s.p50, $s.p99, $s.max } }
"FPS: avg {0:N1}, 1% low {1:N1}, 0.1% low {2:N1} over {3} frames / {4:N1} s (t {5:N0}..{6:N0} ms){7}" -f $avgFps, $low1, $low01, $sel.Count, $span, $t0, $t1, $(if ($low1 -ge 60) { '  [60+ min OK]' } else { '  [below 60 min]' })
"  frame ms       : " + (F $si) + ("  hitches>33ms {0}" -f $hitches)
"  PresentX cpu ms: " + (F $sc)
"  fence wait ms  : " + (F $sw)
"  gpu frame ms   : " + (F $sg)
"  bound          : $bound"
if ($classes.Count) {
  "  hitches (>33.4 ms):"
  foreach ($h in $hitchList) { "    frame {0} t={1:N0} ms: {2:N1} ms (PresentX {3:N2}, unattributed {4:N1}) {5}" -f $h.frame, $h.t_ms, $h.ms, $h.present_cpu_ms, $h.unattributed_ms, $(if ($h.counters) { $h.counters } else { '(no counters)' }) }
  if ($hitchRows.Count) {
    "  hitch time by class ({0:N1} ms in {1} hitches; title-thread exclusive ms; present_cpu = PresentX minus upload):" -f $hitchMs, $hitchRows.Count
    foreach ($e in ($classTotal.GetEnumerator() | Where-Object { $_.Value -gt 0 } | Sort-Object Value -Descending)) { "    {0,-14} {1,9:N1} ms  {2,5:P0}" -f $e.Key, $e.Value, ($e.Value / [Math]::Max(1e-9, $hitchMs)) }
    "  dominant hitch class: $dominant"
  }
  "  event totals in window: " + (($allTotals.GetEnumerator() | ForEach-Object { '{0} {1}x {2}ms' -f $_.Key, $_.Value.n, $_.Value.ms }) -join ', ')
} else {
  "  (no per-frame event counter columns in this csv)"
}
if ($null -ne $ratio) {
  "  game speed     : {0:N4} (sum game_dt {1:N0} ms / sum wall {2:N0} ms over {3} frames, QPC calls {4:N0}, GetTickCount calls {5:N0}){6}" -f $ratio, $gsum, $wsum, $gframes, $qpcCalls, $gtcCalls, $(if ($ratio -lt 0.98 -or $ratio -gt 1.02) { '  [OUTSIDE 0.98-1.02]' } else { '  [ok]' })
} elseif ($cols -contains 'game_dt_ms') {
  "  game speed     : n/a (title made no QueryPerformanceCounter calls through its imports; GetTickCount calls {0:N0})" -f $gtcCalls
}
