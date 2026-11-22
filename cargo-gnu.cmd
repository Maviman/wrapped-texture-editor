@echo off
rem Portable cargo wrapper for any machine (the old cargo.cmd only works on the original laptop's E: paths).
rem Uses the pinned toolchain from rust-toolchain.toml and the dlltool shim made by tools\setup-toolchain.ps1.
rem Usage:  .\cargo-gnu.cmd build --release -p wte_app
setlocal
if not defined WTE_CH set "WTE_CH=1.99.1-x86_64-pc-windows-gnu"
for /f "delims=" %%S in ('rustup run %WTE_CH% rustc --print sysroot') do set "WTE_TC=%%S"
if not defined WTE_TC (
  echo Pinned toolchain %WTE_CH% not found. Run: powershell -ExecutionPolicy Bypass -File tools\setup-toolchain.ps1
  exit /b 1
)
if not exist "%~dp0.toolchain\dlltool-shim\dlltool.exe" (
  echo dlltool shim missing. Run: powershell -ExecutionPolicy Bypass -File tools\setup-toolchain.ps1
  exit /b 1
)
rem dlltool-shim = LLVM's dlltool (the bundled GNU dlltool needs an assembler that isn't shipped).
rem self-contained = the bundled mingw linker, so no Visual Studio / MinGW install is needed.
set "PATH=%~dp0.toolchain\dlltool-shim;%WTE_TC%\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained;%WTE_TC%\lib\rustlib\x86_64-pc-windows-gnu\bin;%WTE_TC%\bin;%PATH%"
set "CARGO_NET_RETRY=10"
set "CARGO_HTTP_TIMEOUT=120"
rustup run %WTE_CH% cargo %*
