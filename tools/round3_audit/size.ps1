$exe = "E:\claude_programs\wrapped_texture_editor\target\round3_audit\wte.exe"
foreach ($j in @(@("size_small","default","900x600"), @("size_tiny","stamp","640x480"))) {
  $out = "E:\claude_programs\wrapped_texture_editor\target\round3_audit\runs\$($j[0])\shot.png"
  $p = Start-Process -FilePath $exe -ArgumentList @("--screenshot", "`"$out`"", "--scenario", $j[1], "--size", $j[2]) -PassThru -Wait
  "$($j[0]) $($j[1]) $($j[2]) exit $($p.ExitCode)"
}
