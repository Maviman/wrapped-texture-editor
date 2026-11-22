param([string]$Exe, [string]$Out)
# Gate stage 3: the core timing probes of round 4, one after the other, release test binary, one thread.
$ErrorActionPreference = 'Continue'
$env:RUST_TEST_THREADS = '1'
$list = @(
  @('stroke_bench_tips', ''),
  @('text::styled::bench', ''),
  @('gate_classic_painter', ''),
  @('text_document_bench', ''),
  @('font_timings', ''),
  @('tip_bake::tests::load_times', '')
)
foreach ($t in $list) {
  "===== $(Get-Date -f s) $($t[0])" | Add-Content $Out
  $sw = [Diagnostics.Stopwatch]::StartNew()
  & $Exe $t[0] --ignored --nocapture --test-threads=1 *>> $Out
  "===== exit $LASTEXITCODE after $([int]$sw.Elapsed.TotalSeconds) s" | Add-Content $Out
}
"===== $(Get-Date -f s) done" | Add-Content $Out
