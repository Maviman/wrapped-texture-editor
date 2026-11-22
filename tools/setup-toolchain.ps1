<#
Sets up the Rust toolchain this project was built with, on a fresh Windows machine.

  powershell -ExecutionPolicy Bypass -File tools\setup-toolchain.ps1 [-InstallRustup]

What it does:
  1. Checks for rustup. With -InstallRustup it downloads the official installer from
     https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/rustup-init.exe and runs it
     (minimal profile, GNU host, no default toolchain). Without the switch it only tells you how.
  2. Installs the pinned toolchain from rust-toolchain.toml: 1.99.1-x86_64-pc-windows-gnu with
     clippy, rustfmt and llvm-tools-preview (rust-mingw comes with the GNU host).
  3. Builds the dlltool shim in .toolchain\dlltool-shim (git-ignored): the GNU dlltool bundled with
     the toolchain needs an assembler that is not shipped, so the build uses LLVM's dlltool instead.
     LLVM's llvm-ar.exe acts as dlltool when it is named dlltool.exe, so the shim is that file under
     two names plus the two MinGW runtime DLLs from the same folder. (On the original machine this
     was E:\claude_downloads\rust-toolchain\dlltool-shim; the files are byte-identical copies.)
  -SkipInstall only rebuilds the shim for an already installed -Channel (default 1.99.1-x86_64-pc-windows-gnu).
  4. Runs a quick check: rustc -V, cargo -V, and that the shim answers.

Afterwards build with  .\cargo-gnu.cmd test --workspace   (or build --release -p wte_app).
#>
param([switch]$InstallRustup, [switch]$SkipInstall, [string]$Channel = '1.99.1-x86_64-pc-windows-gnu')
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$channel = $Channel

if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    if (-not $InstallRustup) {
        Write-Host 'rustup is not installed. Re-run with -InstallRustup, or install it from https://rustup.rs'
        Write-Host '(choose the x86_64-pc-windows-gnu host, minimal profile), then run this script again.'
        exit 1
    }
    $init = Join-Path $env:TEMP 'rustup-init.exe'
    Invoke-WebRequest -Uri 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/rustup-init.exe' -OutFile $init
    & $init -y --default-host x86_64-pc-windows-gnu --profile minimal --default-toolchain none
    $env:PATH = (Join-Path $env:USERPROFILE '.cargo\bin') + ';' + $env:PATH
}

if (-not $SkipInstall) {
    rustup toolchain install $channel --profile minimal --component clippy --component rustfmt --component llvm-tools-preview
    if ($LASTEXITCODE -ne 0) { throw 'rustup toolchain install failed' }
}

$sysroot = (& rustup run $channel rustc --print sysroot).Trim()
$bin = Join-Path $sysroot 'lib\rustlib\x86_64-pc-windows-gnu\bin'
$shim = Join-Path $repo '.toolchain\dlltool-shim'
New-Item -ItemType Directory -Force -Path $shim | Out-Null
Copy-Item (Join-Path $bin 'llvm-ar.exe') (Join-Path $shim 'dlltool.exe') -Force
Copy-Item (Join-Path $bin 'llvm-ar.exe') (Join-Path $shim 'llvm-dlltool.exe') -Force
foreach ($dll in 'libgcc_s_seh-1.dll', 'libwinpthread-1.dll') {
    $src = Join-Path $bin $dll
    if (Test-Path $src) { Copy-Item $src (Join-Path $shim $dll) -Force }
}

& rustup run $channel rustc -V
& rustup run $channel cargo -V
& (Join-Path $shim 'dlltool.exe') --version
Write-Host "Toolchain ready. Shim: $shim"
Write-Host 'Build with: .\cargo-gnu.cmd test --workspace'
