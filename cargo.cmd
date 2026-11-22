@echo off
rem Runs cargo from the self-contained Rust toolchain on E: (not on the system PATH).
rem Usage:  .\cargo.cmd build --release
set "RUSTUP_HOME=E:\claude_downloads\rust-toolchain\rustup"
set "CARGO_HOME=E:\claude_downloads\rust-toolchain\cargo"
set "WTE_TC=E:\claude_downloads\rust-toolchain\rustup\toolchains\stable-x86_64-pc-windows-gnu"
rem dlltool-shim = LLVM's dlltool (the bundled GNU dlltool needs an assembler that isn't shipped).
rem self-contained = the bundled mingw linker, so no Visual Studio / MinGW install is needed.
set "PATH=E:\claude_downloads\rust-toolchain\cargo\bin;E:\claude_downloads\rust-toolchain\dlltool-shim;%WTE_TC%\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained;%WTE_TC%\lib\rustlib\x86_64-pc-windows-gnu\bin;%WTE_TC%\bin;%PATH%"
rem Slow/flaky network: retry crate downloads instead of failing.
set "CARGO_NET_RETRY=10"
set "CARGO_HTTP_TIMEOUT=120"
cargo %*
