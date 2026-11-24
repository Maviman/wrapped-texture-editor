# Wrapped Texture Editor

A Rust desktop app (egui + wgpu) for painting textures directly on 3D models: brushes, layers, image and text
stamps (projected, UV, or wrapped on the surface), PNG / PSD / ORA export and `.wtep` projects.

## Install (Windows)

1. Download `WrappedTextureEditor-<version>-Setup.exe` from the
   [latest release](https://github.com/Maviman/wrapped-texture-editor/releases/latest) and run it. No admin rights,
   terminal, Python or cloned repo needed: it installs for your user only (`%LOCALAPPDATA%\Programs\Wrapped Texture
   Editor`), adds Start Menu shortcuts and, optionally, a desktop shortcut, the `.wtep` file type, example models and
   the graphics troubleshooting start options (DirectX 12 / Vulkan without overlays).
2. **"Windows protected your PC" (SmartScreen):** the installer is not code-signed. Click **More info** →
   **Run anyway**. (If the browser blocks the download, choose **Keep**; you can compare the file with the release's
   `SHA256SUMS.txt` via `Get-FileHash`.)
3. The setup's **System check** page reports a missing or software-only graphics driver. The editor needs Vulkan or
   DirectX 12 (64-bit Windows 10 1809+ or 11); install the current driver from NVIDIA, AMD or Intel if it complains.
   No Visual C++ runtime or other prerequisite is needed.
4. **Upgrade:** run a newer Setup.exe; it replaces the old version in place. **Uninstall:** Settings → Apps → Installed
   apps. Settings and logs in `%APPDATA%\WrappedTextureEditor` are kept unless you tick Yes when asked (silent:
   `unins000.exe /VERYSILENT /PURGEUSERDATA`). Projects and exported textures are never touched.

Silent install: `Setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART [/COMPONENTS=main,gpulaunchers,examples]
[/TASKS=desktopicon,assoc]`. Releases are built and smoke-tested (install, start, upgrade, uninstall) by
[`.github/workflows/installer.yml`](.github/workflows/installer.yml) when a `v<version>` tag matching `Cargo.toml` is
pushed; the installer script is [`installer/WrappedTextureEditor.iss`](installer/WrappedTextureEditor.iss).

## Develop

- Build: `cargo build --release -p wte_app` → `target/release/wrapped_texture_editor.exe`
- Test: `cargo test --workspace`; headless scenarios: `wrapped_texture_editor.exe [MODEL] --screenshot out.png --scenario <name>`
- Design contract: [`ARCHITECTURE.md`](ARCHITECTURE.md)
- State, decisions, open issues and the checklist for picking the project up: [`HANDOFF.md`](HANDOFF.md)
