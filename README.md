# Nimble

A blazing fast, featherweight native notepad for Windows — written in Rust on raw Win32.

## Why

- **Instant**: pure Win32, no GUI framework, no runtime. Cold start is measured in milliseconds.
- **Tiny**: ~230 KB single exe with zero dependencies beyond system DLLs. Fully portable — no install required.
- **Familiar**: File / Edit / View menus, tabs, status bar, word wrap, zoom — like Notepad, only snappier.

## Features

- Tabs — `Ctrl+T` new tab, `Ctrl+W` close, middle-click a tab to close it, `Ctrl+Tab` / `Ctrl+Shift+Tab` to cycle, `+` button, double-click empty tab bar for a new tab
- Multiple windows (`Ctrl+N`)
- Drag & drop files onto the window, or open from the command line: `nimble.exe file.txt`
- Encoding detection — UTF-8, UTF-8 BOM, UTF-16 LE/BE, ANSI — preserved on save
- Word wrap toggle, status bar (Ln/Col, zoom %, line endings, encoding)
- Zoom via `Ctrl`+wheel, `Ctrl`+`+` / `Ctrl`+`-`, `Ctrl`+`0` to reset
- Unsaved-changes prompts when closing tabs or the window
- Per-tab undo history, scroll position, and selection preserved while switching

## Build

Requires the MSVC Rust toolchain (`x86_64-pc-windows-msvc`).

```
cargo build --release
```

Output: `target\release\nimble.exe` (~230 KB).

## Roadmap

- Custom icon
- Installer (NSIS/WiX) and file associations
- Find & replace, go-to line
- Session restore / recent files
- Dark mode

## License

MIT
