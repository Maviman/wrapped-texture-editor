# Runs one self-test scenario of the release exe on a model and records the outcome (exit code, HUNG, native crash).
# Usage: powershell -ExecutionPolicy Bypass -File run_scenario.ps1 -Scenario workflow_path -Model <path> -Out <dir\name.png>
#        [-TimeoutSec 900] [-Seed 1] [-Steps 400] [-Size 1280x800]
param(
    [Parameter(Mandatory = $true)][string]$Scenario,
    [Parameter(Mandatory = $true)][string]$Model,
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$TimeoutSec = 900,
    [string]$Seed = "",
    [string]$Steps = "",
    [string]$Size = "1280x800"
)
$exe = "E:\claude_programs\wrapped_texture_editor\target\release\wrapped_texture_editor.exe"
$dir = Split-Path -Parent $Out
New-Item -ItemType Directory -Force $dir | Out-Null
if ($Seed -ne "") { $env:WTE_FUZZ_SEED = $Seed } else { Remove-Item Env:WTE_FUZZ_SEED -ErrorAction SilentlyContinue }
if ($Steps -ne "") { $env:WTE_FUZZ_STEPS = $Steps } else { Remove-Item Env:WTE_FUZZ_STEPS -ErrorAction SilentlyContinue }
$t0 = Get-Date
$p = Start-Process -FilePath $exe -ArgumentList @("`"$Model`"", "--screenshot", "`"$Out`"", "--scenario", $Scenario, "--size", $Size) -PassThru
$null = $p.Handle  # keeps the handle so ExitCode is available after exit
$done = $p.WaitForExit($TimeoutSec * 1000)
$secs = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
if (-not $done) {
    try { $p.Kill() } catch {}
    $p.WaitForExit(10000) | Out-Null
    $result = "HUNG"
} else {
    $code = $p.ExitCode
    $hex = "0x{0:X8}" -f ([uint32]([int64]$code -band 0xFFFFFFFF))
    $result = switch ($code) { 0 { "ok" } 2 { "assert-failed" } 3 { "timeout/runner" } 101 { "panic" } default { if ($code -lt 0 -or $code -gt 255) { "NATIVE-CRASH $hex" } else { "exit $code" } } }
    $result = "$result (exit $code / $hex)"
}
$json = [IO.Path]::ChangeExtension($Out, ".json")
$log = [IO.Path]::ChangeExtension($Out, ".log")
$errors = ""
if (Test-Path $json) { $errors = (Get-Content $json -Raw | ConvertFrom-Json).errors -join " | " }
$last = ""
if (Test-Path $log) { $last = (Get-Content $log -Tail 3) -join " || " }
"{0} | {1} | {2} s | {3}`n  errors: {4}`n  log tail: {5}" -f $Scenario, (Split-Path -Leaf $Model), $secs, $result, $errors, $last
