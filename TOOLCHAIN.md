# Toolchain and everything needed to build and test

The compiler itself is **not** in this repository: GitHub refuses files over 100 MB, and the toolchain's main files
are 205-341 MB each (1.7 GB in all: `rustc_driver-*.dll` 341 MB, `rust-lld.exe` 218 MB, `opt.exe` / `llc.exe` 205 MB).
It does not need to be: the exact same toolchain is reinstalled from the official Rust servers by version, and the
one hand-made piece (the dlltool shim) is rebuilt by a script from files inside that toolchain.

## What the original machine used

| Piece | Value |
|---|---|
| Rust | **rustc 1.99.1 (58cf8f9c3 2026-10-13)**, toolchain `x86_64-pc-windows-gnu`, rustup profile `minimal` |
| Components | `cargo`, `rustc`, `rust-std`, `rust-mingw`, `clippy`, `rustfmt`, `llvm-tools-preview` |
| Linker | the toolchain's own `rust-lld`, self-contained (set in `.cargo/config.toml`): no Visual Studio, no MinGW install |
| dlltool shim | a folder put first on PATH with `dlltool.exe` and `llvm-dlltool.exe` (both are the toolchain's `lib\rustlib\x86_64-pc-windows-gnu\bin\llvm-ar.exe`, which acts as dlltool under that name) plus `libgcc_s_seh-1.dll` and `libwinpthread-1.dll` from the same folder. Reason: the GNU dlltool bundled with the toolchain needs an assembler that is not shipped |
| Crates | exact versions and checksums in `Cargo.lock` (downloaded from crates.io on the first build; ~45 MB of archives, ~350 MB unpacked) |
| Wrapper | `cargo.cmd` = the original machine's wrapper (hard-coded `E:\claude_downloads\rust-toolchain` paths; kept unchanged for that machine). `cargo-gnu.cmd` = the portable version for any machine |

## Set up a new machine

```powershell
powershell -ExecutionPolicy Bypass -File tools\setup-toolchain.ps1 -InstallRustup
.\cargo-gnu.cmd test --workspace
.\cargo-gnu.cmd clippy --workspace --all-targets
.\cargo-gnu.cmd build --release -p wte_app
```

`tools\setup-toolchain.ps1` installs rustup if asked (official `rustup-init.exe`, GNU host, minimal profile),
installs `1.99.1-x86_64-pc-windows-gnu` with the components above, and builds the shim in `.toolchain\dlltool-shim`
(git-ignored). `rust-toolchain.toml` pins `1.99.1` for plain `cargo` commands inside the repo.

Alternative: if the new machine has the Visual Studio Build Tools, the MSVC host (`1.99.1-x86_64-pc-windows-msvc`)
may build too (then no shim and no `.cargo/config.toml` linker setting are used). That was never tried; the GNU
setup above is the tested one.

## Other things the workflow needs

| Need | Where |
|---|---|
| Python 3 with **Pillow** and **numpy** | the gate tools in `tools/` (export checks, screenshot diffs, frame analysis) and `test-assets/make_shapes.py` |
| Test meshes | `test-assets/shapes/`; regenerate `high_poly_sphere.obj` (55 MB, not committed) with `python -I test-assets/make_shapes.py <out_dir>` |
| Khronos sample models (ABeautifulGame, DamagedHelmet, Avocado, WaterBottle, Lantern, BoxTextured) | `tools/fetch-samples.ps1` downloads them into `samples/` (git-ignored) |
| Windows fonts | text tests and the font picker read the Windows fonts folder |

## This laptop (the original machine)

Its toolchain is installed under the name `stable-x86_64-pc-windows-gnu` (= 1.99.1), so a rustup directory override
(`rustup override set stable-x86_64-pc-windows-gnu` for the project folder) keeps `cargo.cmd` working there despite
`rust-toolchain.toml`.
