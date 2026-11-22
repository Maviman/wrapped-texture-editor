param([string]$Phase = 'all')
# Gate stage 3: the app performance runs, one process at a time, 1280x800, WTE_FRAME_LOG=1 (per-frame lines).
$R4 = 'E:\claude_programs\wrapped_texture_editor\target\round4'
$sweep = 'E:\claude_programs\wrapped_texture_editor\target\round3_audit\sweep.ps1'
$gate = "$R4\wte_gate.exe"
$r3 = 'E:\claude_programs\wrapped_texture_editor\target\round3\wte_round3.exe'
$p0 = "$R4\perf\wte_p0_instrumented.exe"
$d = "$R4\runs_gate3"
$abg = 'E:\claude_downloads\glb_samples\ABeautifulGame.glb'
$t8 = 'E:\claude_programs\wrapped_texture_editor\target\compat\topologies\models\tex_8192\tex_8192.obj'
$t4 = 'E:\claude_programs\wrapped_texture_editor\target\compat\topologies\models\tex_4096\tex_4096.obj'
$ngons = 'E:\claude_programs\wrapped_texture_editor\target\compat\harness\shapes\ngons_tiling.obj'
$log = "$d\perf_driver.log"

function Run([string]$exe, [string]$job, [string]$runs, [string]$csv, [string]$envs) {
  foreach ($k in @('WTE_FRAME_LOG', 'WTE_P0_FRAMES', 'WTE_P0_SIZE_STEP', 'WTE_P0_SAME_SIDE', 'WTE_P0_PROBE')) { Remove-Item "Env:$k" -ErrorAction SilentlyContinue }
  foreach ($kv in ($envs -split ';' | Where-Object { $_ -ne '' })) { $a = $kv.Split('='); Set-Item "Env:$($a[0])" $a[1] }
  $jf = "$d\job_tmp.txt"; Set-Content $jf $job -Encoding utf8
  "$(Get-Date -f s) $runs | $job | $envs" | Add-Content $log
  & powershell -ExecutionPolicy Bypass -File $sweep -Jobs $jf -Csv $csv -Exe $exe -Runs $runs -Parallel 1 -TimeoutSec 3600 | Add-Content $log
}

if ($Phase -in @('all', 'brush')) {
  foreach ($rep in 1, 2) {
    foreach ($m in @($t8, $t4, $abg)) { Run $gate "brush_types_path|$m" "$d\btp_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1' }
    Run $p0 "workflow_path|$abg" "$d\p0_on_$rep" "$d\perf_p0.csv" 'WTE_P0_FRAMES=1;WTE_P0_SIZE_STEP=0'
    Run $p0 "workflow_path|$abg" "$d\p0_off_$rep" "$d\perf_p0.csv" 'WTE_P0_FRAMES=1;WTE_P0_SIZE_STEP=0;WTE_P0_SAME_SIDE=0'
    Run $gate "brush_hover|-" "$d\hover_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1'
  }
}
if ($Phase -in @('all', 'text')) {
  foreach ($rep in 1, 2) {
    Run $gate "text_warp|$abg" "$d\text_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1'
    Run $gate "text_path|$abg" "$d\text_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1'
    Run $gate "text_preview_path|$t8" "$d\text_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1'
    Run $gate "text_preview_path|$abg" "$d\text_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1'
    Run $gate "text_fonts|-" "$d\text_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1'
  }
}
if ($Phase -in @('all', 'r3')) {
  foreach ($rep in 1, 2) {
    foreach ($m in @($ngons, $abg)) {
      Run $gate "workflow_path|$m" "$d\wf_r4_$rep" "$d\perf.csv" ''
      Run $r3 "workflow_path|$m" "$d\wf_r3_$rep" "$d\perf_r3.csv" ''
    }
    Run $gate "fuzz_path|$ngons|7|1000" "$d\fuzz_r4_$rep" "$d\perf.csv" ''
    Run $r3 "fuzz_path|$ngons|7|1000" "$d\fuzz_r3_$rep" "$d\perf_r3.csv" ''
  }
}
if ($Phase -in @('all', 'text2')) {
  foreach ($rep in 3, 4, 5) {
    Run $gate "text_warp|$abg" "$d\text_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1'
    Run $gate "text_path|$abg" "$d\text_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1'
    Run $gate "text_fonts|-" "$d\text_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1'
  }
  foreach ($rep in 3, 4) {
    Run $gate "text_preview_path|$t8" "$d\text_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1'
    Run $gate "text_preview_path|$abg" "$d\text_$rep" "$d\perf.csv" 'WTE_FRAME_LOG=1'
  }
}
"$(Get-Date -f s) phase $Phase done" | Add-Content $log
