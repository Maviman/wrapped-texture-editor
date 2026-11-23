# Stable gate (fix pass 2): run one self-test with a copy of the exe and sample the foreground window every 25 ms (read-only, no input).
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File focus_probe.ps1 -Exe <copy of the exe> -Out <folder> -Scenario smoke
param([string]$Exe, [string]$Out, [string]$Scenario)
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class FG { [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid); }
"@
function fgpid { $p = 0; [void][FG]::GetWindowThreadProcessId([FG]::GetForegroundWindow(), [ref]$p); $p }
$before = fgpid
$proc = Start-Process -FilePath $Exe -ArgumentList @('--screenshot', "$Out\$Scenario.png", '--scenario', $Scenario, '--size', '1280x800') -PassThru -WindowStyle Normal
$samples = 0; $own = 0; $changed = 0
while (-not $proc.HasExited) { $p = fgpid; $samples++; if ($p -eq $proc.Id) { $own++ }; if ($p -ne $before) { $changed++ }; Start-Sleep -Milliseconds 25 }
$after = fgpid
$name = (Get-Process -Id $before -ErrorAction SilentlyContinue).ProcessName
"scenario $Scenario exit $($proc.ExitCode): foreground before pid $before ($name), after pid $after; samples $samples, app in foreground $own, foreground changed $changed"
