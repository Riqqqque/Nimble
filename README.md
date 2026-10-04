# Nimble

<img src="assets/nimble.png" width="96" alt="Nimble logo"/>

A blazing fast, featherweight native notepad for Windows — written in Rust on raw Win32.

## Why

- **Instant**: pure Win32, no GUI framework, no runtime. Cold start is measured in milliseconds.
- **Tiny**: ~600 KB single exe with zero dependencies beyond system DLLs. Fully portable — no install required.
- **Familiar**: File / Edit / View menus, tabs, status bar, word wrap, zoom — like Notepad, only snappier.
- **Big files**: files are memory-mapped and streamed in, never copied whole; line numbers come from an index that only scans as far as the screen, so typing in a 60 MB file stays responsive.

## Features

- Tabs — `Ctrl+T` new tab, `Ctrl+W` close, middle-click a tab to close it, `Ctrl+Tab` / `Ctrl+Shift+Tab` to cycle, `+` button, double-click empty tab bar for a new tab
- Multiple windows (`Ctrl+N`). Opening a file while Nimble is running adds a tab to the open window instead of starting another copy.
- **Themes** — View menu: Light, Dark, or **Use System Setting** to follow Windows' dark mode automatically (reacts live to OS changes). Preference persists.
- Drag & drop files onto the window, or open from the command line: `nimble.exe file.txt`
- Encoding detection — UTF-8, UTF-8 BOM, UTF-16 LE/BE, ANSI — and line endings (CRLF, LF, CR), both preserved on save
- **Find bar** — `Ctrl+F` find (as you type), `Ctrl+H` replace, `Ctrl+G` go to line; `F3` / `Shift+F3` next / previous. Replace all is a single undo step.
- Line numbers, auto-indent, `Ctrl+D` duplicate line
- Word wrap toggle, status bar (Ln/Col, zoom %, line endings, encoding)
- **Settings** (`Ctrl+,`) — theme, editor options, font size, and every keyboard shortcut is rebindable
- Zoom via `Ctrl`+wheel, `Ctrl`+`+` / `Ctrl`+`-`, `Ctrl`+`0` to reset
- Unsaved-changes prompts when closing tabs or the window
- Per-tab undo history, scroll position, and selection preserved while switching

## Code features

Everything here can be switched off in **Settings → Code**, and plain `.txt`
and `.log` files stay plain text.

- **Syntax highlighting** for JSON, YAML, TOML, INI, Markdown, XML/HTML, CSS,
  JavaScript/TypeScript, Rust, Python, C/C++, C#, Java, Go, shell, PowerShell,
  batch, SQL and Lua — picked from the file name or content, or chosen from
  the language in the status bar. Only the text near the screen is colored,
  so huge files and megabyte-long minified lines stay fast.
- **Rainbow brackets**, matching-bracket highlight (`Ctrl+Shift+\` jumps to
  the partner), and highlights on other uses of the word at the caret.
- **JSON and YAML checking as you type**: mistakes get a squiggle, a dot in the
  gutter and a plain-words explanation (hover the squiggle, or read the status
  bar). `F8` / `Shift+F8` walk through them. Checking keeps going after the
  first mistake, so one missing quote doesn't bury the rest. Catches missing
  or trailing commas, single quotes, unquoted keys, comments in strict JSON,
  duplicate keys, unclosed strings and brackets, tabs in YAML indentation, bad
  indentation, `key: value` mistakes, unknown aliases and more. `.jsonc`,
  `tsconfig.json`, VS Code settings and friends allow comments; `.jsonl` is
  checked one value per line.
- **Where am I**: the status bar shows the path to the value under the caret
  (`$.spec.containers[0].image`); `Ctrl+Alt+C` copies it.
- **Code menu**: Format Document (`Shift+Alt+F`, keeps JSON comments), Minify
  JSON, Sort JSON Keys, Convert JSON / YAML (into a new tab), Toggle Comment
  (`Ctrl+/`), Indent / Outdent (`Ctrl+]` / `Ctrl+[`, or `Tab` / `Shift+Tab` on
  a selection), Move Line Up / Down (`Alt+Up` / `Alt+Down`), Select Line
  (`Ctrl+L`), Delete Line (`Ctrl+Shift+K`), Trim Trailing Whitespace.
- **Typing help**: brackets and quotes close themselves (and are typed over),
  Enter between brackets opens an indented block, and a line ending in `{`,
  `[`, `(` — or `:` in YAML and Python — indents the next one.
- **Indentation** is detected per file (tabs or 2/4/8 spaces) and shown in the
  status bar; click it to switch, or convert the file between tabs and spaces.
  Tab stops follow the detected width.
- The status bar's language, line-ending and encoding segments are clickable:
  change the language, or the line endings / encoding the file is saved with.
- Optional on save: trim trailing whitespace, end the file with a newline.

## Install

The exe is self-installing — no separate installer needed:

```
nimble.exe --install
```

Installs per-user to `%LOCALAPPDATA%\Programs\Nimble` (no admin), adds a Start
Menu shortcut, registers itself in Settings → Apps, and sets up automatic
updates: a background check runs at every sign-in and at most once a day at
launch, and **Help → Check for Updates** forces a check. New versions swap in
silently — just relaunch Nimble to use them.

Install also registers Nimble with Windows as a text editor (Open With, and
Settings → Default apps). Windows only lets you choose default apps
yourself, so **Help → Set as Default Text Editor** opens Nimble's page there;
click **Set default** to have it open `.txt`, `.log`, `.ini`, `.cfg`, `.conf`,
and `.md` files.

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

Output: `target\release\nimble.exe` (~600 KB). `cargo test --release` runs the
line-index, encoding, syntax, JSON and YAML tests.

## Roadmap

- Session restore / recent files

## License

MIT
