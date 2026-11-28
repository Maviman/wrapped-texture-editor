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

## Stickers (version 0.4.0, branch `stickers`)

- Every image, text and sticker stamp can have **layer styles**: a die-cut contour (the white sticker border), stroke,
  drop shadow, inner shadow, outer and inner glow, colour and gradient overlay, bevel, with presets (Classic sticker,
  Outline, Glow, Badge, Shadow only). The **Sticker** tab of the side panel styles the selected stamps without opening
  an editor; Copy / Paste / Clear Style (also in the Edit menu) move a style between stamps.
- A **sticker** is a small layered picture: open a stamp in the **Sticker view** (key 4, or Edit Sticker) to change its
  layers with the model beside it, which follows every change. Picture, text and shape layers with opacity, blend
  mode, mask, clipping, adjustments and their own styles; Brush, Eraser (pen pressure), Fill, Gradient, Eyedropper;
  Marquee, Lasso, Magic Wand, Magic Eraser, Remove Background, Cut Out Box (drag a box around the object of a photo:
  GrabCut, colour models and a graph cut, no AI), Color to Alpha, Defringe, Blur, Sharpen; Move with
  transform handles, Crop, Trim, Canvas Size, Resize, Rotate, Flip, Merge Down, Flatten. Help > Shortcuts lists the
  Sticker view's keys.
- The **sticker tray** keeps the project's stickers: make one from a picture or blank, place it with the Stamp tool
  (several stamps can share one; editing a shared one asks "Edit all / Only this one"), "Edit before placing", Export
  Picture as PNG (in the Sticker view also Sticker > Export as PSD), and a **library** folder of `.wtes` sticker files
  shared between projects.
- Projects with stickers are saved as project format 4 (older versions say the project was made by a newer version);
  projects made with 0.3.0 open and export exactly as before.

## Working with PhotoCraft and Photoshop

- **Edit in <editor>** (Sticker tab, Layer menu, a layer's right-click menu; "Edit in..." lists every editor) opens a
  sticker, or a texture paint layer, in PhotoCraft or Photoshop through a linked PSD file in `<project>_links\`. Press
  **Ctrl+S** there and the sticker updates on the model by itself, in place; moving or scaling the sticker in WTE never
  touches the link. WTE never starts an editor by itself and never comes to the front when the editor saves. The
  project must be saved first; WTE never rewrites the linked file at a project save.
- Layers come back one by one (names, visibility, opacity, seven blend modes, masks, clipping); WTE's own text and
  shape layers you did not change stay editable. Layer effects, adjustment layers, groups, smart objects (Photoshop)
  and other blend modes make the sticker come back as one finished picture (Undo brings the layers back); WTE's
  styles are not written into the file and are added again to what comes back.
- **Photoshop:** set Edit > Preferences > File Handling > "Maximize PSD and PSB File Compatibility" to Always; answer
  "Leave as is" to a colour-profile question; drop the "PSD" handle on Photoshop's tab bar to keep it linked. 16-bit
  files work; CMYK, Lab, Indexed and 32-bit files are refused with a sentence that says what to choose in Photoshop.
- **Drag and drop, copy and paste:** pictures dropped from Explorer or a browser land where you let them go (on the
  model, the 2D view, the sticker canvas, a layer row); a layer row, the Sticker tab's picture or a tray sticker dragged
  out of the window becomes a PNG; Ctrl+C / Ctrl+V move pictures and text to and from other programs. There is no drag
  out of PhotoCraft or Photoshop into WTE (use Ctrl+S on a link, copy / paste, or the saved file).
- **Two more doors** (plain files): "Keep a PNG copy up to date" on a sticker (Sticker tab) or a paint layer (Layer
  menu, a row's right-click menu) keeps a PNG in `<project>_links\` written half a second after every change, for
  PhotoCraft's File > Place Linked; **File > Watch Folder for Stickers...** turns every PNG of a folder (such as the
  folder PhotoCraft's File > Generate > Image Assets writes) into a tray sticker that updates when the file changes.
- Help > **Working with PhotoCraft and Photoshop** has the details; the hand tests are in `dist/HOW_TO_TEST.txt`.

## Licences

Wrapped Texture Editor is MIT-licensed (`Cargo.toml`). Parts of the sticker editor (layer styles, selections,
cut-out, adjustments, shapes) are adapted from PhotoCraft's source code under the MIT licence in
[`licenses/photocraft-MIT.txt`](licenses/photocraft-MIT.txt); the installer puts that file into the program's
`licenses` folder and Help > About > **Licences...** shows it. The PSD test files under
`crates/wte_core/tests/fixtures/psd/photoshop/` come with their own licence file there and are not shipped.

Adobe and Photoshop are either registered trademarks or trademarks of Adobe in the United States and/or other
countries. PhotoCraft belongs to its authors. Wrapped Texture Editor is not affiliated with or endorsed by either.

## Pen tablets

- Graphics tablets and pen displays (Wacom, Huion, XP-Pen, Surface) work through **Windows Ink**: keep it on in the
  driver (Wacom: Mapping > "Use Windows Ink"; Huion: "Enable Windows Ink"; XP-Pen: "Windows Ink"), or add the editor
  to the driver's application list with it on.
- Press harder for a wider stroke; Tool tab > Brush options (and Eraser options) > **Pen pressure** sets whether
  pressure drives the size and / or the flow and their minimums. The pressure curve is set in the tablet driver.
- Turn the pen around to erase; hold the lower pen button while the tip touches for the right mouse button (orbit in
  3D, pan in 2D, menus). On a pen display with touch, turn touch off in the driver (or Windows' "Ignore touch input
  when I'm using my pen"), or a resting hand acts as a mouse.
- The status bar shows "Pen N %" while the pen touches the tablet; **Help > Tablet setup** has the details.
- A driver without Windows Ink: Help > Tablet setup > Tablet input: **WinTab** (pressure and eraser from WinTab).
- A mouse paints exactly as before: it always paints at full pressure.

## Tablet companion (Android)

Draw into the editor from an Android tablet (made for a Samsung Galaxy Tab S8 Ultra with S Pen; any Android 10+ tablet
should work): the tablet shows the editor's window live and sends the pen (with pressure, eraser, side button), your
fingers and the keyboard back. The tablet app is **Tablet Companion**, a separate project:
https://github.com/Maviman/tablet-companion — download its `.apk` on the tablet from
https://github.com/Maviman/tablet-companion/releases and follow that README to install and connect it.

On the PC: **Help > Tablet companion...** (off until you turn a link on):

- **USB cable** (recommended): needs Google's adb.exe. Download "SDK Platform-Tools for Windows" from
  https://developer.android.com/tools/releases/platform-tools, unpack it, and if the dialog says "No adb found",
  click **Browse for adb.exe...**. On the tablet turn on USB debugging (Settings > About tablet > Software information,
  tap "Build number" seven times; then Settings > Developer options > USB debugging), plug in the cable and tap Allow.
  The dialog line reads "Ready"; choose USB in the tablet app and click **Allow** on the PC the first time. If Windows
  does not see the tablet, install the Samsung Android USB driver.
- **Wi-Fi**: turn it on (Windows asks once to allow the app on private networks: allow it), pick the PC in the tablet
  app and type the code the dialog shows. Later connections need no code.
- **Bluetooth**: pair the tablet with the PC once in Windows Settings > Bluetooth & devices, turn the switch on, choose
  the PC in the tablet app's Bluetooth list, click Allow. Coarser and slower picture; the pen still feels immediate.

The status bar shows "Tablet: ..." while a link is on (click it for the dialog). Options: let the tablet resize the
window to its shape ("Fit to tablet"), and what one finger does (navigate in the view or act as the mouse).

**Security:** the connection is checked with a pairing key, but the picture and the input are not encrypted. Use Wi-Fi
only on a network you trust; USB does not use the network. Paired tablets are listed in the dialog (Forget removes one).

Troubleshooting: the firewall prompt was dismissed -> Windows Security > Firewall & network protection > Allow an app
through firewall > Wrapped Texture Editor (private); "No adb found" -> Browse for adb.exe; the S Pen button opens Air
command -> turn Air command off in the tablet's S Pen settings (or use the app's eraser toggle).

## Develop

- Build: `cargo build --release -p wte_app` → `target/release/wrapped_texture_editor.exe`
- Test: `cargo test --workspace`; headless scenarios: `wrapped_texture_editor.exe [MODEL] --screenshot out.png --scenario <name>`
- Design contract: [`ARCHITECTURE.md`](ARCHITECTURE.md)
- State, decisions, open issues and the checklist for picking the project up: [`HANDOFF.md`](HANDOFF.md)
