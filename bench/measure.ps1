<#
.SYNOPSIS
  Measures Persona's startup time and memory for a fixed set of pages. Engine-agnostic: it
  measures the whole process tree of the exe (so it covers WebView2's child processes today and an
  in-process engine tomorrow), so the numbers are comparable across engines.

.DESCRIPTION
  Three scenarios, each in a fresh, throwaway profile (never your own profile):
    idle  - no URL: the new-tab page only
    one   - the first URL of the corpus
    all   - every URL of the corpus, one tab each
  Reports startup time (process start to main window), and for each scenario the process count,
  private working set (the number that matters), total working set and commit, plus the marginal
  private working set per tab ((all - idle) / tabs).

  Not covered yet: scroll frame time and JavaScript benchmarks (see bench/README.md).

.EXAMPLE
  cargo build --release
  powershell -ExecutionPolicy Bypass -File bench\measure.ps1 -Label webview2
#>
param(
  [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\Persona.exe'),
  [string]$Corpus = (Join-Path $PSScriptRoot 'corpus.txt'),
  [int]$SettleSeconds = 30,
  [int]$Runs = 3,
  [string]$Label = 'webview2'
)

$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
$urls = Get-Content $Corpus | Where-Object { $_.Trim() -and -not $_.StartsWith('#') } | ForEach-Object { $_.Trim() }
if (-not $urls) { throw "No URLs in $Corpus" }

function Get-ProcessTree([int]$RootPid) {
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId
  $ids = [System.Collections.Generic.List[int]]::new()
  $ids.Add($RootPid)
  for ($i = 0; $i -lt $ids.Count; $i++) {
    foreach ($c in $all | Where-Object { $_.ParentProcessId -eq $ids[$i] }) {
      if (-not $ids.Contains([int]$c.ProcessId)) { $ids.Add([int]$c.ProcessId) }
    }
  }
  return $ids
}

function Measure-Tree([int]$RootPid) {
  $ids = Get-ProcessTree $RootPid
  $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $ids -contains [int]$_.IDProcess }
  $ws = 0; $commit = 0
  foreach ($id in $ids) {
    $p = Get-Process -Id $id -ErrorAction SilentlyContinue
    if ($p) { $ws += $p.WorkingSet64; $commit += $p.PrivateMemorySize64 }
  }
  [pscustomobject]@{
    processes          = $ids.Count
    privateWorkingSetMB = [math]::Round((($perf | Measure-Object WorkingSetPrivate -Sum).Sum) / 1MB, 1)
    workingSetMB       = [math]::Round($ws / 1MB, 1)
    commitMB           = [math]::Round($commit / 1MB, 1)
  }
}

function Invoke-Scenario([string]$Name, [string[]]$CmdArgs) {
  $scratch = Join-Path $env:TEMP ("persona-bench-" + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $scratch | Out-Null
  $oldLocal = $env:LOCALAPPDATA; $oldNoUpdate = $env:PERSONA_NO_UPDATE
  $env:LOCALAPPDATA = $scratch; $env:PERSONA_NO_UPDATE = '1'
  try {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    if ($CmdArgs -and $CmdArgs.Count -gt 0) {
      $proc = Start-Process -FilePath $Exe -ArgumentList $CmdArgs -PassThru
    } else {
      $proc = Start-Process -FilePath $Exe -PassThru
    }
    while ($sw.Elapsed.TotalSeconds -lt 30) {
      $proc.Refresh()
      if ($proc.MainWindowHandle -ne 0) { break }
      Start-Sleep -Milliseconds 10
    }
    $startupMs = [math]::Round($sw.Elapsed.TotalMilliseconds)
    Start-Sleep -Seconds $SettleSeconds
    $m = Measure-Tree $proc.Id
    $m | Add-Member -NotePropertyName startupMs -NotePropertyValue $startupMs
    $m | Add-Member -NotePropertyName scenario -NotePropertyValue $Name
    return $m
  } finally {
    if ($proc -and -not $proc.HasExited) { & taskkill /PID $proc.Id /T /F | Out-Null }
    $env:LOCALAPPDATA = $oldLocal; $env:PERSONA_NO_UPDATE = $oldNoUpdate
    Start-Sleep -Seconds 1
    Remove-Item -Recurse -Force $scratch -ErrorAction SilentlyContinue
  }
}

$results = @()
for ($r = 1; $r -le $Runs; $r++) {
  Write-Host "Run $r of $Runs"
  $idle = Invoke-Scenario 'idle' @()
  $one  = Invoke-Scenario 'one' @($urls[0])
  $all  = Invoke-Scenario 'all' $urls
  $perTab = [math]::Round(($all.privateWorkingSetMB - $idle.privateWorkingSetMB) / $urls.Count, 1)
  $results += [pscustomobject]@{ run = $r; idle = $idle; one = $one; all = $all; tabs = $urls.Count; marginalPrivateMBPerTab = $perTab }
  $idle, $one, $all | Format-Table scenario, startupMs, processes, privateWorkingSetMB, workingSetMB, commitMB
  Write-Host "Marginal private working set per tab: $perTab MB"
}

$outDir = Join-Path $PSScriptRoot 'results'
New-Item -ItemType Directory -Path $outDir -Force | Out-Null
$out = Join-Path $outDir ("$Label-" + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.json')
[pscustomobject]@{ label = $Label; exe = $Exe; settleSeconds = $SettleSeconds; urls = $urls; results = $results } |
  ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 $out
Write-Host "Saved $out"
