## Install

1. Download **WrappedTextureEditor-@VERSION@-Setup.exe** below.
2. Run it. No admin rights are needed: it installs for your Windows user only, into
   `%LOCALAPPDATA%\Programs\Wrapped Texture Editor`, and adds Start Menu shortcuts (a desktop shortcut is optional).
3. The setup checks your graphics driver and tells you if Vulkan or DirectX 12 is missing.

**Windows SmartScreen:** the installer is not code-signed, so Windows may show "Windows protected your PC".
Click **More info**, then **Run anyway**. If your browser marks the download as unsafe, choose **Keep**. You can check
the file against `SHA256SUMS.txt` first (PowerShell: `Get-FileHash .\WrappedTextureEditor-@VERSION@-Setup.exe`).

**Upgrading:** run a newer Setup.exe. It replaces the installed version in place and keeps your settings and choices.

**Uninstalling:** Settings > Apps > Installed apps > Wrapped Texture Editor > Uninstall. Your settings are kept unless
you choose to delete them; your projects and exported textures are never touched.

Requirements: 64-bit Windows 10 (1809) or 11 and a graphics driver with Vulkan or DirectX 12 (any NVIDIA, AMD or Intel
GPU from the last ~10 years with a current driver). Nothing else: no Visual C++ runtime, Python or other download.
