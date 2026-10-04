# Nimble

<img src="assets/nimble.png" width="96" alt="Nimble logo"/>

A blazing fast, featherweight native notepad for Windows — written in Rust on raw Win32.

## Why

- **Instant**: pure Win32, no GUI framework, no runtime. Cold start is measured in milliseconds.
- **Tiny**: ~380 KB single exe with zero dependencies beyond system DLLs. Fully portable — no install required.
- **Familiar**: File / Edit / View menus, tabs, status bar, word wrap, zoom — like Notepad, only snappier.

## Features

- Tabs — `Ctrl+T` new tab, `Ctrl+W` close, middle-click a tab to close it, `Ctrl+Tab` / `Ctrl+Shift+Tab` to cycle, `+` button, double-click empty tab bar for a new tab
- Multiple windows (`Ctrl+N`)
- **Themes** — View → Theme: Light, Dark, or **Use System Setting** to follow Windows' dark mode automatically (reacts live to OS changes). Preference persists.
- Drag & drop files onto the window, or open from the command line: `nimble.exe file.txt`
- Encoding detection — UTF-8, UTF-8 BOM, UTF-16 LE/BE, ANSI — preserved on save
- **Find bar** — `Ctrl+F` find (as you type), `Ctrl+H` replace, `Ctrl+G` go to line; `F3` / `Shift+F3` next / previous. Replace all is a single undo step.
- Line numbers, auto-indent, `Ctrl+D` duplicate line
- Word wrap toggle, status bar (Ln/Col, zoom %, line endings, encoding)
- **Settings** (`Ctrl+,`) — theme, editor options, font size, and every keyboard shortcut is rebindable
- Zoom via `Ctrl`+wheel, `Ctrl`+`+` / `Ctrl`+`-`, `Ctrl`+`0` to reset
- Unsaved-changes prompts when closing tabs or the window
- Per-tab undo history, scroll position, and selection preserved while switching

## Install

The exe is self-installing — no separate installer needed:

```
nimble.exe --install
```

Installs per-user to `%LOCALAPPDATA%\Programs\Nimble` (no admin), adds a Start
Menu shortcut, registers itself in Settings → Apps, and sets up automatic
updates: a background check runs at every sign-in and every launch, and
**Help → Check for Updates** forces a check. New versions swap in silently —
just relaunch Nimble to use them.

```
nimble.exe --uninstall
```

removes everything. Or just use the exe portably — it never writes outside its
folder except preferences and custom shortcuts under `HKCU\Software\Nimble`.

## Build

Requires the MSVC Rust toolchain (`x86_64-pc-windows-msvc`).

```
cargo build --release
```

Output: `target\release\nimble.exe` (~380 KB).

## Roadmap

- File associations
- Session restore / recent files

## License

MIT
