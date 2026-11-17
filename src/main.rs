#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod edit;
mod single;
mod store;

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{
    Editor, EditorState, Input, InputEvent, InputState, Position, Redo, RopeExt,
};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme, Icon, Selectable, Sizable, Theme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use edit::Edit;
use store::{SearchHit, Store};

actions!(
    notes,
    [
        QuickOpen,
        NewNote,
        NewFolder,
        ImportFiles,
        TogglePreview,
        GoToLine,
        SaveNow,
        CloseNote,
        ToggleSidebar,
        ToggleList,
        ToggleZen,
        ToggleFullscreen,
        OverlayUp,
        OverlayDown,
        OverlayClose,
        MoveLinesUp,
        MoveLinesDown,
        CopyLinesUp,
        CopyLinesDown,
        DeleteLines,
        SelectLines,
        InsertLineBelow,
        InsertLineAbove,
        SelectNext,
    ]
);

// Folder icons and the like that the default icon set does not embed.
gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [FolderPlus, FilePlus, Import, Hash, Trash, FolderInput, CopyPlus, Pencil]
);

/// The bundled icons plus `ExtraIcons`.
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

const OVERLAY_ROWS: usize = 12;
/// Key context of the note editor, so line commands only apply there.
const EDITOR_KEYS: &str = "NoteEditor > Input";

#[derive(Clone, PartialEq)]
enum Filter {
    All,
    Folder(String),
    Tag(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Overlay {
    None,
    Switcher,
    NewFolder,
    GoToLine,
    /// Folder pickers for moving / copying `overlay_note`.
    MoveTo,
    CopyTo,
    /// Asks for a new name for `overlay_folder`.
    RenameFolder,
}

/// A note row being dragged from the list.
struct DraggedNote {
    path: PathBuf,
    title: SharedString,
}

/// A folder row being dragged in the sidebar.
struct DraggedFolder {
    folder: String,
    name: SharedString,
}

/// The small label that follows the cursor while dragging.
struct DragPreview(SharedString);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_3()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.foreground)
            .text_sm()
            .child(self.0.clone())
    }
}

struct NotesApp {
    store: Store,
    filter: Filter,
    folders: Vec<String>,
    tags: Vec<(String, usize)>,
    hits: Vec<SearchHit>,
    current: Option<PathBuf>,
    dirty: bool,
    save_task: Option<Task<()>>,
    status: SharedString,

    show_sidebar: bool,
    show_list: bool,
    /// Only the text: hides both panels and the note header.
    zen: bool,
    /// Show the rendered Markdown instead of the editor.
    preview: bool,

    search: Entity<InputState>,
    title: Entity<InputState>,
    editor: Entity<EditorState>,

    overlay: Overlay,
    overlay_input: Entity<InputState>,
    /// Note indices for the switcher, indices into `folder_choices` for the pickers.
    overlay_items: Vec<usize>,
    /// The matching line of each switcher row; empty for a title match.
    overlay_snips: Vec<String>,
    overlay_sel: usize,
    /// "" (the top level) followed by every folder.
    folder_choices: Vec<String>,
    overlay_note: Option<PathBuf>,
    overlay_folder: String,

    _subs: Vec<Subscription>,
}

impl NotesApp {
    fn new(store: Store, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter this list…"));
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Title"));
        let editor =
            cx.new(|cx| EditorState::new(window, cx).soft_wrap(true).line_number(true));
        let overlay_input = cx.new(|cx| InputState::new(window, cx));

        let subs = vec![
            cx.subscribe_in(&search, window, Self::on_search_event),
            cx.subscribe_in(&title, window, Self::on_title_event),
            cx.subscribe_in(&editor, window, Self::on_editor_event),
            cx.subscribe_in(&overlay_input, window, Self::on_overlay_event),
            cx.observe_window_activation(window, Self::on_activation),
        ];

        let mut this = Self {
            store,
            filter: Filter::All,
            folders: Vec::new(),
            tags: Vec::new(),
            hits: Vec::new(),
            current: None,
            dirty: false,
            save_task: None,
            status: SharedString::default(),
            show_sidebar: true,
            show_list: true,
            zen: false,
            preview: false,
            search,
            title,
            editor,
            overlay: Overlay::None,
            overlay_input,
            overlay_items: Vec::new(),
            overlay_snips: Vec::new(),
            overlay_sel: 0,
            folder_choices: Vec::new(),
            overlay_note: None,
            overlay_folder: String::new(),
            _subs: subs,
        };
        this.refresh(cx);
        this.search.update(cx, |s, cx| s.focus(window, cx));
        this
    }

    /// Recompute everything derived from the store: sidebar and note list.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.folders = self.store.folders();
        self.tags = self.store.tags();
        if let Filter::Folder(f) = &self.filter {
            if !self.folders.contains(f) {
                self.filter = Filter::All;
            }
        }
        let query = self.search.read(cx).value().to_string();
        let (folder, tag) = match &self.filter {
            Filter::All => (None, None),
            Filter::Folder(f) => (Some(f.as_str()), None),
            Filter::Tag(t) => (None, Some(t.as_str())),
        };
        self.hits = self.store.search(&query, folder, tag);
        cx.notify();
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.refresh(cx);
    }

    fn fail(&mut self, what: &str, err: std::io::Error, cx: &mut Context<Self>) {
        self.status = format!("{what}: {err}").into();
        cx.notify();
    }

    fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |s, cx| s.focus(window, cx));
    }

    // ---- current note ----

    fn open_note(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.current.as_ref() == Some(&path) {
            return;
        }
        self.flush_save(cx);
        let Some(i) = self.store.index_of(&path) else { return };
        let note = &self.store.notes[i];
        let (title, content) = (note.title.clone(), note.content.clone());
        self.current = Some(path);
        self.preview = false;
        self.title.update(cx, |s, cx| s.set_value(title, window, cx));
        self.editor.update(cx, |s, cx| s.set_value(content, window, cx));
        self.dirty = false;
        cx.notify();
    }

    /// Open a note found by `query` (empty: just open it) and jump to and
    /// select the first match inside the note.
    fn open_hit(
        &mut self,
        path: PathBuf,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_note(path.clone(), window, cx);
        let found = self
            .store
            .index_of(&path)
            .and_then(|i| self.store.match_range(i, &query));
        if let Some(range) = found {
            self.preview = false;
            self.focus_editor(window, cx);
            // The editor can only scroll to text it has already laid out.
            Self::after_layout(window, cx, move |this, window, cx| {
                this.jump_to(range, window, cx)
            });
        }
    }

    /// Run `f` once the editor has drawn its current text.
    fn after_layout(
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        cx.on_next_frame(window, |_, window, cx| cx.on_next_frame(window, f));
        cx.notify();
    }

    /// Select `range` in the editor and scroll it into view.
    fn jump_to(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |s, cx| {
            // Moving the cursor is what scrolls.
            let position = s.text().offset_to_position(range.start);
            s.set_cursor_position(position, window, cx);
            s.set_selected_range(range, cx);
        });
    }

    fn close_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.current = None;
        self.dirty = false;
        self.editor.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(600))
                .await;
            this.update(cx, |this, cx| this.flush_save(cx)).ok();
        }));
    }

    /// Write the editor's text to disk if the user changed it.
    fn flush_save(&mut self, cx: &mut Context<Self>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let Some(path) = self.current.clone() else { return };
        let Some(i) = self.store.index_of(&path) else { return };
        let text = self.editor.read(cx).value().to_string();
        if self.store.notes[i].content == text {
            return;
        }
        match self.store.save(&path, &text) {
            Ok(()) => self.status = SharedString::default(),
            Err(e) => self.fail("Could not save", e, cx),
        }
        self.refresh(cx);
    }

    fn commit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.current.clone() else { return };
        let Some(i) = self.store.index_of(&path) else { return };
        let old = self.store.notes[i].title.clone();
        let new = self.title.read(cx).value().to_string();
        if new.trim() == old {
            return;
        }
        match self.store.rename(&path, &new) {
            Ok(new_path) => {
                self.current = Some(new_path);
                self.status = SharedString::default();
            }
            Err(e) => {
                self.title.update(cx, |s, cx| s.set_value(old, window, cx));
                self.fail("Could not rename", e, cx);
            }
        }
        self.refresh(cx);
    }

    fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_save(cx);
        let folder = match &self.filter {
            Filter::Folder(f) => f.clone(),
            _ => String::new(),
        };
        if matches!(self.filter, Filter::Tag(_)) {
            self.filter = Filter::All;
        }
        self.zen = false;
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        match self.store.create(&folder, "Untitled") {
            Ok(path) => {
                self.refresh(cx);
                self.open_note(path, window, cx);
                self.title.update(cx, |s, cx| {
                    s.focus(window, cx);
                    s.select_all(window, cx);
                });
            }
            Err(e) => self.fail("Could not create note", e, cx),
        }
    }

    fn delete_note(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let is_current = self.current.as_ref() == Some(&path);
        if is_current {
            self.dirty = false;
        }
        match self.store.delete(&path) {
            Ok(()) => {
                if is_current {
                    self.close_note(window, cx);
                }
                self.status = "Moved to the .trash folder".into();
            }
            Err(e) => self.fail("Could not delete", e, cx),
        }
        self.refresh(cx);
    }

    /// The folder new and imported notes go to: the selected one, else the root.
    fn current_folder(&self) -> String {
        match &self.filter {
            Filter::Folder(f) => f.clone(),
            _ => String::new(),
        }
    }

    // ---- organising notes and folders ----

    /// Move a note. The open note stays open under its new path.
    fn move_note_to(
        &mut self,
        path: PathBuf,
        folder: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Pending edits must reach the file before it moves.
        self.flush_save(cx);
        match self.store.move_note(&path, &folder) {
            Ok(new_path) if new_path != path => {
                if self.current.as_ref() == Some(&path) {
                    // A name clash can change the title.
                    if let Some(i) = self.store.index_of(&new_path) {
                        let title = self.store.notes[i].title.clone();
                        self.title.update(cx, |s, cx| s.set_value(title, window, cx));
                    }
                    self.current = Some(new_path);
                }
                self.status = format!("Moved to {}", folder_label(&folder)).into();
            }
            Ok(_) => {}
            Err(e) => self.fail("Could not move", e, cx),
        }
        self.refresh(cx);
    }

    fn copy_note_to(&mut self, path: PathBuf, folder: String, cx: &mut Context<Self>) {
        self.flush_save(cx);
        match self.store.copy_note(&path, &folder) {
            Ok(copy) => {
                let name = copy.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                self.status = format!("Copied as \"{name}\" in {}", folder_label(&folder)).into();
            }
            Err(e) => self.fail("Could not copy", e, cx),
        }
        self.refresh(cx);
    }

    /// Copy a note within its own folder.
    fn duplicate_note(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(i) = self.store.index_of(&path) else { return };
        let folder = self.store.notes[i].folder.clone();
        self.copy_note_to(path, folder, cx);
    }

    /// Ask where to move or copy a note, using the overlay as a folder picker.
    fn pick_folder(
        &mut self,
        overlay: Overlay,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overlay_note = Some(path);
        self.folder_choices = std::iter::once(String::new())
            .chain(self.folders.iter().cloned())
            .collect();
        self.show_overlay(overlay, window, cx);
    }

    fn rename_folder_prompt(&mut self, folder: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = folder.rsplit('/').next().unwrap_or(&folder).to_string();
        self.overlay_folder = folder;
        self.show_overlay(Overlay::RenameFolder, window, cx);
        self.overlay_input.update(cx, |s, cx| {
            s.set_value(name, window, cx);
            s.select_all(window, cx);
        });
    }

    /// A folder was renamed or moved: keep the open note and the filter on it.
    fn folder_moved(&mut self, old: &str, new: &str) {
        if let Filter::Folder(f) = &self.filter {
            if let Some(moved) = rebase_folder(f, old, new) {
                self.filter = Filter::Folder(moved);
            }
        }
        let root = &self.store.root;
        let relative = self.current.as_ref().and_then(|p| p.strip_prefix(root.join(old)).ok());
        if let Some(relative) = relative {
            self.current = Some(root.join(new).join(relative));
        }
    }

    fn rename_folder_to(&mut self, folder: String, name: String, cx: &mut Context<Self>) {
        self.flush_save(cx);
        match self.store.rename_folder(&folder, &name) {
            Ok(new) => self.folder_moved(&folder, &new),
            Err(e) => self.fail("Could not rename folder", e, cx),
        }
        self.refresh(cx);
    }

    fn move_folder_to(&mut self, folder: String, into: String, cx: &mut Context<Self>) {
        self.flush_save(cx);
        match self.store.move_folder(&folder, &into) {
            Ok(new) if new != folder => {
                self.folder_moved(&folder, &new);
                self.status = format!("Moved folder to {}", folder_label(&into)).into();
            }
            Ok(_) => {}
            Err(e) => self.fail("Could not move folder", e, cx),
        }
        self.refresh(cx);
    }

    fn delete_folder_now(&mut self, folder: String, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_save(cx);
        let inside = self.store.root.join(&folder);
        let holds_current = self.current.as_ref().is_some_and(|p| p.starts_with(&inside));
        match self.store.delete_folder(&folder) {
            Ok(()) => {
                if holds_current {
                    self.close_note(window, cx);
                }
                self.status = "Folder moved to the .trash folder".into();
            }
            Err(e) => self.fail("Could not delete folder", e, cx),
        }
        self.refresh(cx);
    }

    fn new_note_in(&mut self, folder: String, window: &mut Window, cx: &mut Context<Self>) {
        self.filter = Filter::Folder(folder);
        self.new_note(window, cx);
    }

    fn new_subfolder(&mut self, folder: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_filter(Filter::Folder(folder), cx);
        self.show_overlay(Overlay::NewFolder, window, cx);
    }

    // ---- import ----

    /// Let the user pick .md / .txt files and copy them into `folder`.
    fn import_dialog(&mut self, folder: String, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(files))) = picked.await else { return };
            this.update_in(cx, |this, window, cx| this.import_paths(files, folder, window, cx))
                .ok();
        })
        .detach();
    }

    fn import_paths(
        &mut self,
        files: Vec<PathBuf>,
        folder: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.store.import(&files, &folder) {
            Ok((notes, skipped)) => {
                let mut status = format!("Imported {} note{}", notes.len(), plural(notes.len()));
                if skipped > 0 {
                    status.push_str(&format!(
                        ", skipped {skipped} unsupported file{}",
                        plural(skipped)
                    ));
                }
                self.status = status.into();
                self.refresh(cx);
                if let Some(first) = notes.into_iter().next() {
                    self.open_note(first, window, cx);
                }
            }
            Err(e) => self.fail("Could not import", e, cx),
        }
    }

    fn toggle_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.current.as_deref().is_some_and(is_markdown) {
            return;
        }
        self.flush_save(cx);
        self.preview = !self.preview;
        if !self.preview {
            self.focus_editor(window, cx);
        }
        cx.notify();
    }

    // ---- editor line commands ----

    /// Run a line command on the editor's text and selection.
    fn edit_lines(
        &mut self,
        command: impl FnOnce(&str, Range<usize>) -> Option<Edit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.current.is_none() {
            return;
        }
        let state = self.editor.read(cx);
        let Some(edit) = command(&state.value(), state.selected_range()) else { return };
        self.editor.update(cx, |s, cx| {
            s.set_selected_range(edit.range, cx);
            s.replace(edit.text, window, cx);
            s.set_selected_range(edit.select, cx);
        });
        self.schedule_save(cx);
    }

    fn select_in_editor(
        &mut self,
        pick: impl FnOnce(&str, Range<usize>) -> Option<Range<usize>>,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        let state = self.editor.read(cx);
        let had_selection = !state.selected_range().is_empty();
        let Some(range) = pick(&state.value(), state.selected_range()) else { return };
        self.editor.update(cx, |s, cx| {
            if add && had_selection {
                s.add_selection(range, cx);
            } else {
                s.set_selected_range(range, cx);
            }
        });
    }

    // ---- overlay (find / new folder / go to line / folder picker / rename) ----

    fn show_overlay(&mut self, overlay: Overlay, window: &mut Window, cx: &mut Context<Self>) {
        if overlay == Overlay::GoToLine && self.current.is_none() {
            return;
        }
        self.overlay = overlay;
        let placeholder = match overlay {
            Overlay::NewFolder => "New folder name  (use / for subfolders)",
            Overlay::GoToLine => "Go to line number",
            Overlay::MoveTo => "Move to folder…",
            Overlay::CopyTo => "Copy to folder…",
            Overlay::RenameFolder => "New folder name",
            _ => "Find a note by title or text…",
        };
        self.overlay_input.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.set_placeholder(placeholder, window, cx);
            s.focus(window, cx);
        });
        self.update_overlay_items(cx);
    }

    fn update_overlay_items(&mut self, cx: &mut Context<Self>) {
        let query = self.overlay_input.read(cx).value().to_string();
        self.overlay_items.clear();
        self.overlay_snips.clear();
        match self.overlay {
            Overlay::Switcher => {
                // Notes containing the words first (title matches lead), each
                // with its matching line; then looser fuzzy title matches.
                if !query.trim().is_empty() {
                    for hit in self.store.search(&query, None, None) {
                        self.overlay_items.push(hit.index);
                        self.overlay_snips.push(hit.snippet);
                    }
                }
                let listed: HashSet<usize> = self.overlay_items.iter().copied().collect();
                for index in self.store.fuzzy_titles(&query) {
                    if !listed.contains(&index) {
                        self.overlay_items.push(index);
                        self.overlay_snips.push(String::new());
                    }
                }
            }
            Overlay::MoveTo | Overlay::CopyTo => {
                let query = query.trim().to_lowercase();
                self.overlay_items = (0..self.folder_choices.len())
                    .filter(|&i| folder_label(&self.folder_choices[i]).to_lowercase().contains(&query))
                    .collect();
            }
            _ => {}
        }
        self.overlay_sel = 0;
        cx.notify();
    }

    fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == Overlay::None {
            return;
        }
        self.overlay = Overlay::None;
        if self.current.is_some() {
            self.focus_editor(window, cx);
        } else {
            self.search.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    fn confirm_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.overlay_input.read(cx).value().to_string();
        match self.overlay {
            Overlay::Switcher => {
                if let Some(&i) = self.overlay_items.get(self.overlay_sel) {
                    let path = self.store.notes[i].path.clone();
                    // A row with a snippet matched in the text: jump to the match.
                    if self.overlay_snips.get(self.overlay_sel).is_some_and(|s| !s.is_empty()) {
                        self.open_hit(path, typed, window, cx);
                    } else {
                        self.open_note(path, window, cx);
                    }
                }
            }
            Overlay::MoveTo | Overlay::CopyTo => {
                let choice = self.overlay_items.get(self.overlay_sel).map(|&i| self.folder_choices[i].clone());
                if let (Some(folder), Some(path)) = (choice, self.overlay_note.take()) {
                    if self.overlay == Overlay::MoveTo {
                        self.move_note_to(path, folder, window, cx);
                    } else {
                        self.copy_note_to(path, folder, cx);
                    }
                }
            }
            Overlay::RenameFolder => {
                let folder = std::mem::take(&mut self.overlay_folder);
                self.rename_folder_to(folder, typed, cx);
            }
            Overlay::NewFolder => {
                let folder = match &self.filter {
                    Filter::Folder(parent) => format!("{parent}/{typed}"),
                    _ => typed,
                };
                match self.store.create_folder(&folder) {
                    Ok(()) => {
                        self.refresh(cx);
                        let made = folder.trim_matches('/').replace('\\', "/");
                        if self.folders.contains(&made) {
                            self.filter = Filter::Folder(made);
                            self.refresh(cx);
                        }
                    }
                    Err(e) => self.fail("Could not create folder", e, cx),
                }
            }
            Overlay::GoToLine => {
                if let Ok(line) = typed.trim().parse::<u32>() {
                    let position = Position::new(line.saturating_sub(1), 0);
                    let offset = self.editor.read(cx).text().position_to_offset(&position);
                    self.jump_to(offset..offset, window, cx);
                }
            }
            Overlay::None => {}
        }
        self.close_overlay(window, cx);
    }

    fn move_overlay_sel(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.overlay_items.len().min(OVERLAY_ROWS);
        if count > 0 {
            self.overlay_sel =
                (self.overlay_sel as isize + delta).rem_euclid(count as isize) as usize;
            cx.notify();
        }
    }

    // ---- events ----

    fn on_search_event(
        &mut self,
        _: &Entity<InputState>,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            InputEvent::Change => self.refresh(cx),
            InputEvent::PressEnter { .. } => {
                if let Some(hit) = self.hits.first() {
                    let path = self.store.notes[hit.index].path.clone();
                    let query = self.search.read(cx).value().to_string();
                    self.open_hit(path, query, window, cx);
                    self.focus_editor(window, cx);
                }
            }
            _ => {}
        }
    }

    fn on_title_event(
        &mut self,
        _: &Entity<InputState>,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            InputEvent::PressEnter { .. } => {
                self.commit_title(window, cx);
                self.focus_editor(window, cx);
            }
            InputEvent::Blur => self.commit_title(window, cx),
            _ => {}
        }
    }

    fn on_editor_event(
        &mut self,
        _: &Entity<EditorState>,
        ev: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(ev, InputEvent::Change) && self.current.is_some() {
            self.schedule_save(cx);
        }
    }

    fn on_overlay_event(
        &mut self,
        _: &Entity<InputState>,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            InputEvent::Change => self.update_overlay_items(cx),
            InputEvent::PressEnter { .. } => self.confirm_overlay(window, cx),
            _ => {}
        }
    }

    /// Save when the window loses focus; pick up outside edits when it regains it.
    fn on_activation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_save(cx);
        if !window.is_window_active() {
            return;
        }
        self.store.reload();
        if let Some(path) = self.current.clone() {
            match self.store.index_of(&path) {
                Some(i) => {
                    let content = self.store.notes[i].content.clone();
                    if content != self.editor.read(cx).value().to_string() {
                        self.editor.update(cx, |s, cx| s.set_value(content, window, cx));
                        self.dirty = false;
                    }
                }
                None => self.close_note(window, cx),
            }
        }
        if self.overlay != Overlay::None {
            self.update_overlay_items(cx);
        }
        self.refresh(cx);
    }

    // ---- rendering ----

    /// A context-menu entry that runs `f` on the app with a copy of `subject`.
    /// The menu's own handlers only get `&mut App`, hence the weak handle.
    fn menu_item<T: Clone + 'static>(
        app: &WeakEntity<Self>,
        subject: &T,
        label: &'static str,
        icon: IconName,
        f: fn(&mut Self, T, &mut Window, &mut Context<Self>),
    ) -> PopupMenuItem {
        let (app, subject) = (app.clone(), subject.clone());
        PopupMenuItem::new(label).icon(icon).on_click(move |_, window, cx| {
            let subject = subject.clone();
            app.update(cx, |this, cx| f(this, subject, window, cx)).ok();
        })
    }

    fn note_menu(menu: PopupMenu, app: &WeakEntity<Self>, path: &PathBuf) -> PopupMenu {
        menu.item(Self::menu_item(app, path, "Open", IconName::FileText, Self::open_note))
            .item(Self::menu_item(app, path, "Move to folder…", IconName::FolderInput, |this, p, w, cx| {
                this.pick_folder(Overlay::MoveTo, p, w, cx)
            }))
            .item(Self::menu_item(app, path, "Copy to folder…", IconName::Copy, |this, p, w, cx| {
                this.pick_folder(Overlay::CopyTo, p, w, cx)
            }))
            .item(Self::menu_item(app, path, "Duplicate", IconName::CopyPlus, |this, p, _, cx| {
                this.duplicate_note(p, cx)
            }))
            .separator()
            .item(Self::menu_item(app, path, "Delete", IconName::Trash, Self::delete_note))
    }

    fn folder_menu(menu: PopupMenu, app: &WeakEntity<Self>, folder: &String) -> PopupMenu {
        menu.item(Self::menu_item(app, folder, "New note here", IconName::FilePlus, Self::new_note_in))
            .item(Self::menu_item(app, folder, "New subfolder", IconName::FolderPlus, Self::new_subfolder))
            .item(Self::menu_item(app, folder, "Rename…", IconName::Pencil, Self::rename_folder_prompt))
            .item(Self::menu_item(app, folder, "Import files here", IconName::Import, Self::import_dialog))
            .separator()
            .item(Self::menu_item(app, folder, "Delete folder", IconName::Trash, Self::delete_folder_now))
    }

    fn side_row(
        &self,
        id: impl Into<ElementId>,
        icon: IconName,
        label: String,
        count: Option<usize>,
        depth: usize,
        filter: Filter,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme();
        let selected = self.filter == filter;
        let (hover, active, muted) = (theme.list_hover, theme.list_active, theme.muted_foreground);
        h_flex()
            .id(id)
            .h(px(28.))
            .pl(px(12. + 14. * depth as f32))
            .pr_3()
            .gap_2()
            .cursor_pointer()
            .when(selected, |el| el.bg(active))
            .hover(|el| el.bg(hover))
            .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter.clone(), cx)))
            .child(Icon::new(icon).small().text_color(muted))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .children(count.map(|c| div().text_xs().text_color(muted).child(c.to_string())))
    }

    /// Make a sidebar row take notes, folders and Explorer files dropped on it,
    /// moving or importing them into `folder` ("" = top level).
    fn drop_target(row: Stateful<Div>, folder: String, cx: &mut Context<Self>) -> Stateful<Div> {
        let highlight = cx.theme().drop_target;
        let (for_notes, for_folders, for_files) = (folder.clone(), folder.clone(), folder);
        row.drag_over::<DraggedNote>(move |s, _, _, _| s.bg(highlight))
            .drag_over::<DraggedFolder>(move |s, _, _, _| s.bg(highlight))
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(highlight))
            .on_drop(cx.listener(move |this, d: &DraggedNote, window, cx| {
                this.move_note_to(d.path.clone(), for_notes.clone(), window, cx)
            }))
            .on_drop(cx.listener(move |this, d: &DraggedFolder, _, cx| {
                this.move_folder_to(d.folder.clone(), for_folders.clone(), cx)
            }))
            .on_drop(cx.listener(move |this, d: &ExternalPaths, window, cx| {
                this.import_paths(d.paths().to_vec(), for_files.clone(), window, cx)
            }))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (bg, border, muted) = (theme.sidebar, theme.sidebar_border, theme.muted_foreground);
        let header = move |text: &'static str| {
            div()
                .px_3()
                .pt_3()
                .pb_1()
                .text_xs()
                .text_color(muted)
                .child(text)
        };

        let mut rows = v_flex().id("sidebar-rows").flex_1().min_h_0().overflow_y_scroll();
        let all = self.side_row(
            "all",
            IconName::Inbox,
            "All notes".to_string(),
            Some(self.store.notes.len()),
            0,
            Filter::All,
            cx,
        );
        rows = rows.child(Self::drop_target(all, String::new(), cx));
        rows = rows.child(header("FOLDERS"));
        for (i, folder) in self.folders.iter().enumerate() {
            let depth = folder.matches('/').count();
            let name = folder.rsplit('/').next().unwrap_or(folder).to_string();
            let filter = Filter::Folder(folder.clone());
            let icon = if self.filter == filter { IconName::FolderOpen } else { IconName::Folder };
            let dragged = DraggedFolder { folder: folder.clone(), name: name.clone().into() };
            let row = self
                .side_row(("folder", i), icon, name, None, depth, filter, cx)
                .on_drag(dragged, |d, _, _, cx| cx.new(|_| DragPreview(d.name.clone())));
            let (app, menu_folder) = (cx.weak_entity(), folder.clone());
            rows = rows.child(
                Self::drop_target(row, folder.clone(), cx)
                    .context_menu(move |menu, _, _| Self::folder_menu(menu, &app, &menu_folder)),
            );
        }
        rows = rows.child(header("TAGS"));
        for (i, (tag, count)) in self.tags.iter().enumerate() {
            rows = rows.child(self.side_row(
                ("tag", i),
                IconName::Hash,
                tag.clone(),
                Some(*count),
                0,
                Filter::Tag(tag.clone()),
                cx,
            ));
        }

        v_flex()
            .w(px(230.))
            .h_full()
            .flex_shrink_0()
            .bg(bg)
            .border_r_1()
            .border_color(border)
            .child(
                h_flex()
                    .p_2()
                    .gap_2()
                    .child(
                        Button::new("new-note")
                            .small()
                            .primary()
                            .icon(IconName::FilePlus)
                            .tooltip("New note (Ctrl+N)")
                            .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx))),
                    )
                    .child(
                        Button::new("new-folder")
                            .small()
                            .icon(IconName::FolderPlus)
                            .tooltip("New folder (Ctrl+Shift+N)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_overlay(Overlay::NewFolder, window, cx)
                            })),
                    )
                    .child(
                        Button::new("import")
                            .small()
                            .icon(IconName::Import)
                            .tooltip(
                                "Import .md / .txt files (Ctrl+O) — copies them into this folder",
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                let folder = this.current_folder();
                                this.import_dialog(folder, window, cx)
                            })),
                    )
                    .child(
                        Button::new("find")
                            .small()
                            .icon(IconName::Search)
                            .tooltip("Find a note by title or text (Ctrl+P)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_overlay(Overlay::Switcher, window, cx)
                            })),
                    ),
            )
            .child(rows)
            .when(!self.status.is_empty(), |el| {
                el.child(
                    div()
                        .p_2()
                        .text_xs()
                        .text_color(muted)
                        .child(self.status.clone()),
                )
            })
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let list = uniform_list(
            "notes",
            self.hits.len(),
            cx.processor(|this, range: Range<usize>, _window, cx| {
                let theme = cx.theme();
                let (hover, active, muted, border) = (
                    theme.list_hover,
                    theme.list_active,
                    theme.muted_foreground,
                    theme.border,
                );
                let mut rows = Vec::new();
                for ix in range {
                    let Some(hit) = this.hits.get(ix) else { continue };
                    let note = &this.store.notes[hit.index];
                    let path = note.path.clone();
                    let selected = this.current.as_ref() == Some(&path);
                    let second = if !hit.snippet.is_empty() {
                        hit.snippet.clone()
                    } else if !note.folder.is_empty() {
                        note.folder.clone()
                    } else {
                        let first = note.content.lines().find(|l| !l.trim().is_empty());
                        first.unwrap_or("").trim().chars().take(160).collect()
                    };
                    let (open_path, menu_path) = (path.clone(), path.clone());
                    let dragged = DraggedNote { path: path.clone(), title: note.title.clone().into() };
                    let app = cx.weak_entity();
                    rows.push(
                        v_flex()
                            .id(ix)
                            .w_full()
                            .h(px(52.))
                            .px_3()
                            .justify_center()
                            .border_b_1()
                            .border_color(border)
                            .cursor_pointer()
                            .when(selected, |el| el.bg(active))
                            .hover(|el| el.bg(hover))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let query = this.search.read(cx).value().to_string();
                                this.open_hit(open_path.clone(), query, window, cx)
                            }))
                            .on_drag(dragged, |d, _, _, cx| {
                                cx.new(|_| DragPreview(d.title.clone()))
                            })
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(note.title.clone()),
                                    )
                                    .when(is_txt(&path), |el| el.child(txt_marker(muted))),
                            )
                            .child(div().truncate().text_xs().text_color(muted).child(second))
                            .context_menu(move |menu, _, _| Self::note_menu(menu, &app, &menu_path)),
                    );
                }
                rows
            }),
        )
        .flex_1()
        .min_h_0();

        v_flex()
            .w(px(300.))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(border)
            .child(div().p_2().child(
                Input::new(&self.search)
                    .prefix(Icon::new(IconName::Search).small().text_color(muted))
                    .cleanable(true),
            ))
            .child(if self.hits.is_empty() {
                div()
                    .p_3()
                    .text_sm()
                    .text_color(muted)
                    .child("No notes here.")
                    .into_any_element()
            } else {
                list.into_any_element()
            })
    }

    /// Panel toggles, the note title and its actions.
    fn render_toolbar(&self, has_note: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let markdown = self.current.as_deref().is_some_and(is_markdown);
        h_flex()
            .p_2()
            .gap_2()
            .child(
                Button::new("toggle-sidebar")
                    .small()
                    .ghost()
                    .selected(self.show_sidebar)
                    .icon(IconName::PanelLeft)
                    .tooltip("Folders panel (Ctrl+B)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_sidebar = !this.show_sidebar;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("toggle-list")
                    .small()
                    .ghost()
                    .selected(self.show_list)
                    .icon(IconName::FileText)
                    .tooltip("Note list (Ctrl+Alt+B)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_list = !this.show_list;
                        cx.notify();
                    })),
            )
            .child(div().flex_1().min_w_0().when(has_note, |el| el.child(Input::new(&self.title))))
            .when(has_note && markdown, |el| {
                el.child(
                    Button::new("preview")
                        .small()
                        .ghost()
                        .selected(self.preview)
                        .icon(if self.preview { IconName::EyeOff } else { IconName::Eye })
                        .tooltip("Markdown preview (Ctrl+Shift+V)")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.toggle_preview(window, cx)),
                        ),
                )
            })
            .when(has_note, |el| {
                el.child(
                    Button::new("move")
                        .small()
                        .ghost()
                        .icon(IconName::FolderInput)
                        .tooltip("Move to folder…")
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(path) = this.current.clone() {
                                this.pick_folder(Overlay::MoveTo, path, window, cx)
                            }
                        })),
                )
                .child(
                    Button::new("duplicate")
                        .small()
                        .ghost()
                        .icon(IconName::CopyPlus)
                        .tooltip("Duplicate note")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(path) = this.current.clone() {
                                this.duplicate_note(path, cx)
                            }
                        })),
                )
                .child(
                    Button::new("delete")
                        .small()
                        .ghost()
                        .icon(IconName::Trash)
                        .tooltip("Delete note (moves it to .trash)")
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(path) = this.current.clone() {
                                this.delete_note(path, window, cx)
                            }
                        })),
                )
            })
            .child(
                Button::new("zen")
                    .small()
                    .ghost()
                    .icon(IconName::Maximize)
                    .tooltip("Zen mode (Ctrl+K Z)")
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_zen(window, cx))),
            )
    }

    fn toggle_zen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.zen = !self.zen;
        if self.current.is_some() {
            self.focus_editor(window, cx);
        }
        cx.notify();
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = cx.theme().muted_foreground;
        let note = self
            .current
            .as_ref()
            .and_then(|p| self.store.index_of(p))
            .map(|i| &self.store.notes[i]);
        let column = v_flex()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .when(!self.zen, |el| el.child(self.render_toolbar(note.is_some(), cx)))
            .when(self.zen, |el| {
                el.child(
                    div().absolute().top_1().right_4().child(
                        Button::new("exit-zen")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Minimize)
                            .tooltip("Exit zen (Ctrl+K Z)")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_zen(window, cx)),
                            ),
                    ),
                )
            });
        let Some(note) = note else {
            return column.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .text_color(muted)
                    .child(Icon::new(IconName::FileText).size(px(56.)).text_color(muted))
                    .child("Select a note, or press Ctrl+N to create one."),
            );
        };
        let preview = self.preview && is_markdown(&note.path);
        let info = h_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_1()
                    .child(Icon::new(IconName::Folder).small().text_color(muted))
                    .child(if note.folder.is_empty() { "Notes" } else { note.folder.as_str() }.to_string()),
            )
            .children(note.tags.iter().map(|tag| format!("#{tag}")))
            .when(is_txt(&note.path), |el| el.child(txt_marker(muted)));
        let body = if preview {
            div()
                .id("preview")
                .flex_1()
                .min_h_0()
                .p_4()
                .child(
                    TextView::markdown("note-preview", self.editor.read(cx).value())
                        .scrollable(true)
                        .selectable(true),
                )
                .into_any_element()
        } else {
            div()
                .key_context("NoteEditor")
                .on_action(cx.listener(|this, _: &MoveLinesUp, window, cx| {
                    this.edit_lines(edit::move_lines_up, window, cx)
                }))
                .on_action(cx.listener(|this, _: &MoveLinesDown, window, cx| {
                    this.edit_lines(edit::move_lines_down, window, cx)
                }))
                .on_action(cx.listener(|this, _: &CopyLinesUp, window, cx| {
                    this.edit_lines(|t, s| Some(edit::copy_lines(t, s, true)), window, cx)
                }))
                .on_action(cx.listener(|this, _: &CopyLinesDown, window, cx| {
                    this.edit_lines(|t, s| Some(edit::copy_lines(t, s, false)), window, cx)
                }))
                .on_action(cx.listener(|this, _: &DeleteLines, window, cx| {
                    this.edit_lines(|t, s| Some(edit::delete_lines(t, s)), window, cx)
                }))
                .on_action(cx.listener(|this, _: &InsertLineBelow, window, cx| {
                    this.edit_lines(|t, s| Some(edit::insert_line(t, s, false)), window, cx)
                }))
                .on_action(cx.listener(|this, _: &InsertLineAbove, window, cx| {
                    this.edit_lines(|t, s| Some(edit::insert_line(t, s, true)), window, cx)
                }))
                .on_action(cx.listener(|this, _: &SelectLines, _, cx| {
                    this.select_in_editor(|t, s| Some(edit::select_lines(t, s)), false, cx)
                }))
                .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                    this.select_in_editor(edit::next_occurrence, true, cx)
                }))
                .flex_1()
                .min_h_0()
                .when(!self.zen, |el| el.p_2())
                .child(Editor::new(&self.editor).h_full().bordered(!self.zen))
                .into_any_element()
        };
        column
            .when(!self.zen, |el| {
                el.child(div().px_3().pb_1().text_xs().text_color(muted).child(info))
            })
            .child(body)
    }

    fn render_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (backdrop, panel_bg, border, active, muted) = (
            theme.overlay,
            theme.popover,
            theme.border,
            theme.list_active,
            theme.muted_foreground,
        );
        let mut results = v_flex();
        for (row, &i) in self.overlay_items.iter().take(OVERLAY_ROWS).enumerate() {
            // A switcher row is a note, a picker row a folder.
            let (title, detail) = if self.overlay == Overlay::Switcher {
                let note = &self.store.notes[i];
                let snippet = self.overlay_snips.get(row).filter(|s| !s.is_empty());
                (note.title.clone(), snippet.unwrap_or(&note.folder).clone())
            } else {
                (folder_label(&self.folder_choices[i]).to_string(), String::new())
            };
            results = results.child(
                h_flex()
                    .id(("switch", row))
                    .w_full()
                    .h(px(30.))
                    .px_3()
                    .gap_3()
                    .cursor_pointer()
                    .when(row == self.overlay_sel, |el| el.bg(active))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.overlay_sel = row;
                        this.confirm_overlay(window, cx);
                    }))
                    .child(div().truncate().child(title))
                    .child(div().truncate().text_xs().text_color(muted).child(detail)),
            );
        }
        div()
            .id("backdrop")
            .absolute()
            .inset_0()
            .bg(backdrop)
            .flex()
            .items_start()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_overlay(window, cx)
                }),
            )
            .child(
                v_flex()
                    .key_context("Overlay")
                    .on_action(cx.listener(|this, _: &OverlayUp, _, cx| this.move_overlay_sel(-1, cx)))
                    .on_action(cx.listener(|this, _: &OverlayDown, _, cx| this.move_overlay_sel(1, cx)))
                    .on_action(cx.listener(|this, _: &OverlayClose, window, cx| {
                        this.close_overlay(window, cx)
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .mt(px(90.))
                    .w(px(560.))
                    .bg(panel_bg)
                    .border_1()
                    .border_color(border)
                    .rounded_lg()
                    .shadow_lg()
                    .overflow_hidden()
                    .child(div().p_2().child(Input::new(&self.overlay_input)))
                    .child(results),
            )
    }
}

impl Render for NotesApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg) = (theme.background, theme.foreground);
        h_flex()
            .relative()
            .size_full()
            .items_start()
            .bg(bg)
            .text_color(fg)
            .text_sm()
            .on_action(cx.listener(|this, _: &QuickOpen, window, cx| {
                this.show_overlay(Overlay::Switcher, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewNote, window, cx| this.new_note(window, cx)))
            .on_action(cx.listener(|this, _: &NewFolder, window, cx| {
                this.show_overlay(Overlay::NewFolder, window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToLine, window, cx| {
                this.show_overlay(Overlay::GoToLine, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ImportFiles, window, cx| {
                let folder = this.current_folder();
                this.import_dialog(folder, window, cx)
            }))
            .on_action(cx.listener(|this, _: &TogglePreview, window, cx| {
                this.toggle_preview(window, cx)
            }))
            .on_drop(cx.listener(|this, d: &ExternalPaths, window, cx| {
                let folder = this.current_folder();
                this.import_paths(d.paths().to_vec(), folder, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SaveNow, _, cx| {
                this.dirty = this.current.is_some();
                this.flush_save(cx);
            }))
            .on_action(cx.listener(|this, _: &CloseNote, window, cx| this.close_note(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.show_sidebar = !this.show_sidebar || this.zen;
                this.zen = false;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleList, _, cx| {
                this.show_list = !this.show_list || this.zen;
                this.zen = false;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleZen, window, cx| this.toggle_zen(window, cx)))
            .on_action(|_: &ToggleFullscreen, window, _| window.toggle_fullscreen())
            .when(self.show_sidebar && !self.zen, |el| el.child(self.render_sidebar(cx)))
            .when(self.show_list && !self.zen, |el| el.child(self.render_list(cx)))
            .child(self.render_editor(cx))
            .when(self.overlay != Overlay::None, |el| el.child(self.render_overlay(cx)))
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

fn is_txt(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt"))
}

/// Small marker that tells a `.txt` note from a same-named `.md` note.
fn txt_marker(color: Hsla) -> Div {
    div().text_xs().text_color(color).child("txt")
}

fn folder_label(folder: &str) -> &str {
    if folder.is_empty() { "Notes (top level)" } else { folder }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Where `path` ends up when the folder `old` is renamed or moved to `new`.
fn rebase_folder(path: &str, old: &str, new: &str) -> Option<String> {
    if path == old {
        return Some(new.to_string());
    }
    let rest = path.strip_prefix(old)?;
    rest.starts_with('/').then(|| format!("{new}{rest}"))
}

fn notes_root() -> PathBuf {
    if let Some(arg) = std::env::args_os().nth(1) {
        return PathBuf::from(arg);
    }
    let home = std::env::var_os("USERPROFILE").unwrap_or_default();
    PathBuf::from(home).join("Documents").join("Notes")
}

fn main() {
    let root = notes_root();
    let title = format!("Notes — {}", root.display());
    // Launching again (shortcut key, Start menu) shows the open window.
    if single::focus_existing(&title) {
        return;
    }
    let store = Store::open(root.clone())
        .unwrap_or_else(|e| panic!("cannot open notes folder {}: {e}", root.display()));

    // The bundled icon set is needed by the editor's find/replace panel.
    let app = gpui_kit::application().with_assets(AppAssets);
    app.run(move |cx| {
        gpui_kit::init(cx);
        cx.bind_keys([
            // App-wide, as in VS Code.
            KeyBinding::new("ctrl-p", QuickOpen, None),
            KeyBinding::new("ctrl-n", NewNote, None),
            KeyBinding::new("ctrl-shift-n", NewFolder, None),
            KeyBinding::new("ctrl-shift-f", QuickOpen, None),
            KeyBinding::new("ctrl-o", ImportFiles, None),
            KeyBinding::new("ctrl-shift-v", TogglePreview, None),
            KeyBinding::new("ctrl-g", GoToLine, None),
            KeyBinding::new("ctrl-s", SaveNow, None),
            KeyBinding::new("ctrl-w", CloseNote, None),
            KeyBinding::new("ctrl-b", ToggleSidebar, None),
            KeyBinding::new("ctrl-alt-b", ToggleList, None),
            KeyBinding::new("ctrl-k z", ToggleZen, None),
            KeyBinding::new("f11", ToggleFullscreen, None),
            // Quick switcher.
            KeyBinding::new("up", OverlayUp, Some("Overlay > Input")),
            KeyBinding::new("down", OverlayDown, Some("Overlay > Input")),
            KeyBinding::new("escape", OverlayClose, Some("Overlay > Input")),
            // Editor line commands.
            KeyBinding::new("alt-up", MoveLinesUp, Some(EDITOR_KEYS)),
            KeyBinding::new("alt-down", MoveLinesDown, Some(EDITOR_KEYS)),
            KeyBinding::new("shift-alt-up", CopyLinesUp, Some(EDITOR_KEYS)),
            KeyBinding::new("shift-alt-down", CopyLinesDown, Some(EDITOR_KEYS)),
            KeyBinding::new("ctrl-shift-k", DeleteLines, Some(EDITOR_KEYS)),
            KeyBinding::new("ctrl-l", SelectLines, Some(EDITOR_KEYS)),
            KeyBinding::new("ctrl-enter", InsertLineBelow, Some(EDITOR_KEYS)),
            KeyBinding::new("ctrl-shift-enter", InsertLineAbove, Some(EDITOR_KEYS)),
            KeyBinding::new("ctrl-d", SelectNext, Some(EDITOR_KEYS)),
            KeyBinding::new("ctrl-shift-z", Redo, Some("Input")),
        ]);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Maximized(Bounds::centered(
                None,
                size(px(1200.), px(760.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some(title.clone().into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            cx.new(|cx| NotesApp::new(store, window, cx))
        })
        .expect("failed to open window");
        cx.on_window_closed(|cx, _| cx.quit()).detach();
    });
}
