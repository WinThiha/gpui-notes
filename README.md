# gpui-notes

A fast plain-text note taker for Windows, built with Rust and [GPUI](https://gpui.rs) (through [gpui-kit](https://github.com/longbridge/gpui-kit)).

Notes are ordinary `.md` and `.txt` files in a folder, so they stay readable in any other editor. The app adds what a plain editor lacks: folders, tags, instant search across every note, and markdown preview.

## Install

You need Windows 10 or 11. Nothing else has to be installed first, and no administrator rights are needed.

1. Press the Windows key, type `powershell` and press Enter.
2. Paste this line and press Enter:

   ```
   irm https://raw.githubusercontent.com/WinThiha/gpui-notes/main/install.ps1 | iex
   ```

3. Open Notes: press **Ctrl+Alt+N** anywhere, or press the Windows key and type `Notes`.

The installer downloads `notes.exe` from the [latest release](https://github.com/WinThiha/gpui-notes/releases/latest) into `%LOCALAPPDATA%\Programs\Notes` and adds a Start menu shortcut. Run the same line again to update.

Prefer to do it by hand? Download `notes.exe` from the [latest release](https://github.com/WinThiha/gpui-notes/releases/latest) and double-click it; it is a single file and needs no installation. Windows may show a "Windows protected your PC" notice because the file is not signed: choose **More info**, then **Run anyway**.

### First steps

- Your notes are plain files in `Documents\Notes`. The folder is created the first time you start the app.
- **Ctrl+N** makes a note. Typing is saved automatically.
- **Ctrl+P** finds a note by its title or by any text inside it.
- **Ctrl+O** brings in `.md` and `.txt` files you already have. You can also drag them from Explorer into the window.
- Right-click a note or a folder to move, copy, rename or delete it.
- Pressing Ctrl+Alt+N while Notes is open brings its window to the front.

### Options and removal

Save [install.ps1](install.ps1) to use its options:

| Command | What it does |
|---|---|
| `.\install.ps1 -Hotkey 'Ctrl+Alt+J'` | Use another shortcut key (`-Hotkey ''` for none) |
| `.\install.ps1 -Uninstall` | Remove the app and its shortcut. Your notes are kept. |
| `.\install.ps1 -Build` | Build from the source folder and install that (see [Build](#build)) |

To keep notes in a different folder, start the app with it: `notes.exe D:\MyNotes`. Any folder of `.md` / `.txt` files works.

## Features

- **One find box** (Ctrl+P): type a title or any text. Notes containing the words come first with the matching line; choosing one opens the note at the match. The box above the note list only filters that list.
- **Folders**: subfolders of the notes folder show up as a tree in the sidebar. Right-click a folder to rename it, delete it, add a note or subfolder, or import files into it. Drag a folder onto another to move it.
- **Organise notes**: right-click a note to move it, copy it to another folder, duplicate it or delete it, or drag it onto a folder.
- **Import**: Ctrl+O, or dropping files from Explorer onto the window or a folder, copies existing `.md` / `.txt` files into the selected folder. The originals are left alone.
- **Markdown preview**: Ctrl+Shift+V shows a `.md` note rendered, with headings, lists, checkboxes, tables and code.
- **Tags**: write `#tag` anywhere in a note and it appears in the sidebar with a count.
- **VS Code-style editor**: line numbers, find and replace, multiple cursors, line move/copy/delete.
- **Autosave** about half a second after you stop typing. Windows line endings are kept.
- **Hideable panels** and a Zen mode that shows only the text.
- **Safe delete**: deleted notes and folders move to a hidden `.trash` folder inside the notes folder.
- **One window**: starting the app while it is already open on the same folder brings that window to the front.

## Shortcuts

| Keys | Action |
|---|---|
| Ctrl+P or Ctrl+Shift+F | Find a note by title or text |
| Ctrl+O | Import `.md` / `.txt` files |
| Ctrl+Shift+V | Markdown preview on or off |
| Ctrl+N / Ctrl+Shift+N | New note / new folder |
| Ctrl+B | Show or hide the folder sidebar |
| Ctrl+Alt+B | Show or hide the note list |
| Ctrl+K then Z | Zen mode (text only) |
| F11 | Fullscreen |
| Ctrl+S / Ctrl+W | Save now / close note |
| Ctrl+F / Ctrl+H | Find / replace in the note |
| Ctrl+G | Go to line |
| Ctrl+D | Select word, then add the next occurrence |
| Ctrl+Alt+Up/Down | Add a cursor above/below |
| Alt+Up/Down | Move line up/down |
| Shift+Alt+Up/Down | Copy line up/down |
| Ctrl+Shift+K | Delete line |
| Ctrl+L | Select line |
| Ctrl+Enter / Ctrl+Shift+Enter | Insert line below / above |
| Ctrl+Z / Ctrl+Y | Undo / redo |

## Build

Requires a recent stable Rust toolchain.

```
pwsh -File build.ps1
```

The executable is written to `target\release\notes.exe`. `pwsh -File install.ps1 -Build` builds and installs it in one step.

The project builds with the GNU toolchain (`x86_64-pc-windows-gnu` plus MinGW), which needs no administrator rights or Visual Studio. With [scoop](https://scoop.sh):

```
scoop install rustup-gnu mingw
```

GPUI compiles its shaders into the executable with `fxc.exe` from the Windows SDK, so the app does not have to compile them at every start. `build.ps1` first builds `tools/fxc.rs`, a small stand-in that uses the compiler DLL shipped with Windows, so the SDK is not needed. With the SDK installed, a plain `cargo build --release` works too.

Pushing a tag such as `v0.2.0` makes GitHub build `notes.exe` and publish it as a release (`.github/workflows/release.yml`).

## Layout

| File | Purpose |
|---|---|
| `src/main.rs` | Window, panels, key bindings |
| `src/store.rs` | Reading and writing notes, search, tags, fuzzy title match |
| `src/edit.rs` | Line commands for the editor |
| `src/single.rs` | Brings the open window forward when the app is started again |
| `assets/`, `build.rs` | The icon and the build script that embeds it (`python assets/make_icon.py assets` redraws it; needs Pillow) |
| `install.ps1` | Downloads or builds the app, adds the Start menu shortcut and shortcut key |
| `build.ps1`, `tools/fxc.rs` | Release build, and the shader compiler stand-in it uses |
