# Round 2 compatibility sweep: runs self-test scenarios on many models with a small concurrency limit and writes
# sweep.csv (model, format, scenario, seed, ok, result, failing step, seconds, exit code, hex, out dir).
# powershell -ExecutionPolicy Bypass -File sweep.ps1 -Jobs jobs.txt -Csv sweep.csv [-Parallel 3] [-TimeoutSec 900]
# jobs.txt: one job per line: scenario|model path|seed|steps|timeout(optional)
param(
    [Parameter(Mandatory = $true)][string]$Jobs,
    [Parameter(Mandatory = $true)][string]$Csv,
    [int]$Parallel = 3,
    [int]$TimeoutSec = 900,
    [string]$Exe = "E:\claude_programs\wrapped_texture_editor\target\round3_audit\wte.exe",
    [string]$Runs = "E:\claude_programs\wrapped_texture_editor\target\round3_audit\runs"
)
$runs = $Runs
New-Item -ItemType Directory -Force $runs | Out-Null
if (-not (Test-Path $Csv)) {
    "model,format,scenario,seed,ok,result,failing_step,seconds,exit_code,exit_hex,out" | Set-Content -LiteralPath $Csv -Encoding utf8
}
$queue = [System.Collections.Generic.Queue[string]]::new()
Get-Content -LiteralPath $Jobs -Encoding utf8 | Where-Object { $_.Trim() -ne "" -and -not $_.StartsWith("#") } | ForEach-Object { $queue.Enqueue($_) }
$active = @()
function Csv([string]$s) { '"' + ($s -replace '"', '""') + '"' }
function Finish($j) {
    $p = $j.P
    $secs = [math]::Round(((Get-Date) - $j.T0).TotalSeconds, 1)
    $code = ""; $hex = ""
    if ($j.Hung) { $result = "HUNG" } else {
        $code = $p.ExitCode
        $hex = "0x{0:X8}" -f ([uint32]([int64]$code -band 0xFFFFFFFF))
        $result = switch ($code) { 0 { "ok" } 2 { "assert-failed" } 3 { "runner-error" } 101 { "PANIC" } default { if ($code -lt 0 -or $code -gt 255) { "NATIVE-CRASH" } else { "exit $code" } } }
    }
    $json = [IO.Path]::ChangeExtension($j.Out, ".json")
    $errors = ""
    if (Test-Path -LiteralPath $json) { try { $errors = (@((Get-Content -LiteralPath $json -Raw -Encoding utf8 | ConvertFrom-Json).errors) -join " | ") } catch { $errors = "unreadable json" } }
    elseif ($result -eq "ok") { $result = "no-json" }
    $ok = if ($result -eq "ok") { "true" } else { "false" }
    $fmt = [IO.Path]::GetExtension($j.Model).TrimStart('.').ToLower()
    $line = @((Csv $j.Model), $fmt, $j.Scenario, $j.Seed, $ok, $result, (Csv $errors), $secs, $code, $hex, (Csv $j.Dir)) -join ","
    Add-Content -LiteralPath $Csv -Value $line -Encoding utf8
    "{0,-6} {1,6}s {2} {3} {4} {5}" -f $ok, $secs, $j.Scenario, $j.Seed, (Split-Path -Leaf $j.Model), $errors
}
while ($queue.Count -gt 0 -or $active.Count -gt 0) {
    while ($queue.Count -gt 0 -and $active.Count -lt $Parallel) {
        $f = $queue.Dequeue().Split('|')
        $scenario = $f[0]; $model = $f[1]; $seed = if ($f.Count -gt 2) { $f[2] } else { "" }; $steps = if ($f.Count -gt 3) { $f[3] } else { "" }
        # A '*' in the path is resolved here, so names that cannot be written to jobs.txt (a lone surrogate) still run.
        if ($model.Contains('*')) { $model = (Get-ChildItem -Path $model | Select-Object -First 1).FullName }
        $to = if ($f.Count -gt 4 -and $f[4] -ne "") { [int]$f[4] } else { $TimeoutSec }
        $safe = if ($model -eq "-") { "builtin" } else { ((Split-Path -Leaf (Split-Path -Parent $model)) + "_" + (Split-Path -Leaf $model)) -replace '[^A-Za-z0-9_.-]', '_' }
        $tag = if ($f.Count -gt 5) { "_" + $f[5] } else { "" }
        $dir = "$runs\$scenario" + "_" + $safe + $tag + $(if ($seed -ne "") { "_s$seed" } else { "" })
        if (Test-Path -LiteralPath $dir) { Remove-Item -Recurse -Force -LiteralPath $dir }
        New-Item -ItemType Directory -Force $dir | Out-Null
        $out = "$dir\shot.png"
        if ($seed -ne "") { $env:WTE_FUZZ_SEED = $seed } else { Remove-Item Env:WTE_FUZZ_SEED -ErrorAction SilentlyContinue }
        if ($steps -ne "") { $env:WTE_FUZZ_STEPS = $steps } else { Remove-Item Env:WTE_FUZZ_STEPS -ErrorAction SilentlyContinue }
        # Model "-": a built-in scenario without a PATH.
        $argv = @("--screenshot", "`"$out`"", "--scenario", $scenario, "--size", "1280x800")
        if ($model -ne "-") { $argv = @("`"$model`"") + $argv }
        $p = Start-Process -FilePath $Exe -ArgumentList $argv -PassThru
        $null = $p.Handle
        $active += [pscustomobject]@{ P = $p; T0 = Get-Date; To = $to; Model = $model; Scenario = $scenario; Seed = $seed; Out = $out; Dir = $dir; Hung = $false }
    }
    Start-Sleep -Milliseconds 500
    $still = @()
    foreach ($j in $active) {
        if ($j.P.HasExited) { Finish $j; continue }
        if (((Get-Date) - $j.T0).TotalSeconds -gt $j.To) {
            try { $j.P.Kill() } catch {}
            $j.P.WaitForExit(15000) | Out-Null
            $j.Hung = $true
            Finish $j
            continue
        }
        $still += $j
    }
    $active = $still
}
Remove-Item Env:WTE_FUZZ_SEED -ErrorAction SilentlyContinue
Remove-Item Env:WTE_FUZZ_STEPS -ErrorAction SilentlyContinue
