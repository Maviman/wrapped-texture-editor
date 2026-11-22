<#
Downloads the Khronos glTF sample models the gates used into samples\ (git-ignored).
  powershell -ExecutionPolicy Bypass -File tools\fetch-samples.ps1
Source: https://github.com/KhronosGroup/glTF-Sample-Assets (each model has its own license file there).
ABeautifulGame is the heavy-model benchmark (1.5 M triangles, about 60 MB).
#>
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$out = Join-Path $repo 'samples'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$base = 'https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/main/Models'
$models = 'ABeautifulGame', 'DamagedHelmet', 'Avocado', 'WaterBottle', 'Lantern', 'BoxTextured'
foreach ($m in $models) {
    $dest = Join-Path $out "$m.glb"
    if (Test-Path $dest) { Write-Host "have $m"; continue }
    Write-Host "downloading $m"
    Invoke-WebRequest -Uri "$base/$m/glTF-Binary/$m.glb" -OutFile $dest
}
Get-ChildItem $out -Filter *.glb | Select-Object Name, @{n='MB';e={[math]::Round($_.Length/1MB,1)}}
