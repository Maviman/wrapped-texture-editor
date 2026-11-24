## Unreleased (branch `companion`)

- **Tablet companion:** draw into the editor from an Android tablet (USB cable, Wi-Fi or Bluetooth). The tablet shows
  this window live and sends the pen with pressure, your fingers and the keyboard back. Help > Tablet companion...
  The tablet app "Tablet Companion" is downloaded separately from
  https://github.com/Maviman/tablet-companion/releases (it is not part of this installer). Off until you turn a link
  on; the picture and input are not encrypted (use Wi-Fi only on a network you trust).

## What's new in 0.2.0

- **Objects:** a model with several objects (e.g. a chess set) now keeps them apart. The new **Objects** tab lists
  them; click one there, or in the 3D view with the Navigate tool, to select it. **Object** menu: Hide (H),
  Isolate (Shift+H), Show all (Alt+H), Frame selection (Shift+F). Hidden objects are not drawn and do not block
  painting, fills, stamps or wraps. Projects remember which objects are hidden.
- **Tabs:** the side panel is now two tab groups, like Photoshop: **Tool | Color | Texture sets | Objects** above and
  **Layers | Brush | Text** below. Drag the line between them to resize; double-click it to go back to automatic.
  Click the colour chip in the toolbar to jump to the Color tab.

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
