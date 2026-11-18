# FeatherShot

A lightweight, portable screenshot tool for Windows 10/11, built with Rust and native Win32 controls. No WebView. Event-driven hotkeys and tray handling, with no background polling.

**[Download the latest Windows release](https://github.com/Samsxx001/FeatherShot/releases/latest)** · **[คู่มือภาษาไทย](README-TH.md)**

## Install

Download `FeatherShot-0.1.6-win64.zip` from Releases, extract it into a writable folder, and run `FeatherShot.exe`. Screenshots and settings are saved beside the executable. To update, quit FeatherShot and replace the executable; keep your `settings.ini` and `Screenshots` folder.

## Features

- Capture a region, a visible window, or all screens; save PNG and copy to clipboard.
- Customize global shortcuts, including Win combinations and Print Screen.
- Presets for **Win+Shift+S** (region) and **PrtSc** (all screens).
- Optional **Block Snipping Tool shortcuts while running** in Customize hotkeys. Save to apply. Blocks unassigned bare PrtSc and Win+Shift+S; assigned FeatherShot actions still capture. Off by default; quitting restores Windows behavior without registry changes. Snipping Tool can still be opened from Start.
- Annotate with arrows, rectangles, pen strokes, and opaque redaction; undo/redo.
- Capture delay, cursor option, and startup in a window or system tray.
- Clicking or double-clicking the tray icon opens the main window.

Default shortcuts: Ctrl+Shift+S (region), Ctrl+Shift+W (window), Ctrl+Shift+F (all screens). Set the Win/PrtSc presets in **Customize hotkeys**, then save. For PrtSc, disable Windows' “Use the Print screen key to open screen capture” setting if it interferes.

Window capture uses visible screen pixels, including anything covering the window. HDR, protected content, and mixed-DPI setups have limitations. See [validation notes](VALIDATION-TH.md). Video, scrolling capture, OCR, cloud uploads, and automatic updates are not included.

## Build

Install Rust stable plus Visual Studio C++ Build Tools and Windows SDK on Windows:

```powershell
cargo build --release --locked
cargo test --locked -- --test-threads=1
```

Tests that capture the desktop or register hotkeys require an interactive Windows session. The executable is at `target/release/feathershot.exe`.

GNU toolchain build and packaging are also available through `build.ps1` and `package.ps1`; see the Thai guide. Release packages include dependency license notices.

## Contributing

Open an issue with Windows version, app version, reproduction steps, and expected behavior. Avoid including private screenshots or settings. Pull requests are welcome.

Licensed under [MIT](LICENSE).
