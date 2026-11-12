# gpui-notes

A fast plain-text note taker for Windows, built with Rust and [GPUI](https://gpui.rs) (through [gpui-kit](https://github.com/longbridge/gpui-kit)).

Notes are ordinary `.md` and `.txt` files in a folder, so they stay readable in any other editor. The app adds what a plain editor lacks: folders, tags, instant search across every note, and a quick switcher.

## Features

- **Full-text search** across all notes as you type. Clicking a result opens the note and jumps to the match.
- **Folders**: subfolders of the notes folder show up as a tree in the sidebar.
- **Tags**: write `#tag` anywhere in a note and it appears in the sidebar with a count.
- **Quick switcher**: fuzzy jump to a note by title.
- **VS Code-style editor**: line numbers, find and replace, multiple cursors, line move/copy/delete.
- **Autosave** about half a second after you stop typing. Windows line endings are kept.
- **Hideable panels** and a Zen mode that shows only the text.
- **Safe delete**: deleted notes move to a hidden `.trash` folder inside the notes folder.

## Run

```
notes.exe [notes-folder]
```

Without an argument the app uses `Documents\Notes` in your user profile and creates it if needed. Point it at an existing folder of `.md`/`.txt` files to use those.

## Shortcuts

| Keys | Action |
|---|---|
| Ctrl+P | Jump to note |
| Ctrl+Shift+F | Search all notes |
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
cargo build --release
```

The executable is written to `target\release\notes.exe`.

The project builds with the GNU toolchain (`x86_64-pc-windows-gnu` plus MinGW), which needs no administrator rights or Visual Studio. With [scoop](https://scoop.sh):

```
scoop install rustup-gnu mingw
```

GPUI normally precompiles its shaders in release builds with `fxc.exe` from the Windows SDK. `Cargo.toml` turns that off for the `gpui-pre-windows` crate so the build works without the SDK; the shaders are compiled at startup instead. If you have the SDK installed, you can remove that override for a slightly faster start.

## Layout

| File | Purpose |
|---|---|
| `src/main.rs` | Window, panels, key bindings |
| `src/store.rs` | Reading and writing notes, search, tags, fuzzy title match |
| `src/edit.rs` | Line commands for the editor |
