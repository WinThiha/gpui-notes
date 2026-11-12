#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod edit;
mod store;

use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{
    Editor, EditorState, Input, InputEvent, InputState, Position, Redo, RopeExt,
};
use gpui_kit::component::{ActiveTheme, Selectable, Sizable, Theme, h_flex, v_flex};
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
        FocusSearch,
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

    search: Entity<InputState>,
    title: Entity<InputState>,
    editor: Entity<EditorState>,

    overlay: Overlay,
    overlay_input: Entity<InputState>,
    overlay_items: Vec<usize>,
    overlay_sel: usize,

    _subs: Vec<Subscription>,
}

impl NotesApp {
    fn new(store: Store, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search all notes  (Ctrl+Shift+F)"));
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
            search,
            title,
            editor,
            overlay: Overlay::None,
            overlay_input,
            overlay_items: Vec::new(),
            overlay_sel: 0,
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
        self.title.update(cx, |s, cx| s.set_value(title, window, cx));
        self.editor.update(cx, |s, cx| s.set_value(content, window, cx));
        self.dirty = false;
        cx.notify();
    }

    /// Open a note from the list. With a search query, jump to and select
    /// the first match inside the note.
    fn open_hit(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_note(path.clone(), window, cx);
        let query = self.search.read(cx).value().to_string();
        let found = self
            .store
            .index_of(&path)
            .and_then(|i| self.store.match_range(i, &query));
        if let Some(range) = found {
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

    fn delete_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.current.clone() else { return };
        self.dirty = false;
        match self.store.delete(&path) {
            Ok(()) => {
                self.close_note(window, cx);
                self.status = "Moved to the .trash folder".into();
            }
            Err(e) => self.fail("Could not delete", e, cx),
        }
        self.refresh(cx);
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

    // ---- overlay (quick switcher / new folder / go to line) ----

    fn show_overlay(&mut self, overlay: Overlay, window: &mut Window, cx: &mut Context<Self>) {
        if overlay == Overlay::GoToLine && self.current.is_none() {
            return;
        }
        self.overlay = overlay;
        let placeholder = match overlay {
            Overlay::NewFolder => "New folder name  (use / for subfolders)",
            Overlay::GoToLine => "Go to line number",
            _ => "Jump to note…",
        };
        self.overlay_input.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.set_placeholder(placeholder, window, cx);
            s.focus(window, cx);
        });
        self.update_overlay_items(cx);
    }

    fn update_overlay_items(&mut self, cx: &mut Context<Self>) {
        self.overlay_items = if self.overlay == Overlay::Switcher {
            let query = self.overlay_input.read(cx).value().to_string();
            self.store.fuzzy_titles(&query)
        } else {
            Vec::new()
        };
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
                    self.open_note(path, window, cx);
                }
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
                    self.open_hit(path, window, cx);
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
        self.refresh(cx);
    }

    // ---- rendering ----

    fn side_row(
        &self,
        id: impl Into<ElementId>,
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
            .child(div().flex_1().min_w_0().truncate().child(label))
            .children(count.map(|c| div().text_xs().text_color(muted).child(c.to_string())))
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
        rows = rows.child(self.side_row(
            "all",
            "All notes".to_string(),
            Some(self.store.notes.len()),
            0,
            Filter::All,
            cx,
        ));
        rows = rows.child(header("FOLDERS"));
        for (i, folder) in self.folders.iter().enumerate() {
            let depth = folder.matches('/').count();
            let name = folder.rsplit('/').next().unwrap_or(folder).to_string();
            rows = rows.child(self.side_row(
                ("folder", i),
                name,
                None,
                depth,
                Filter::Folder(folder.clone()),
                cx,
            ));
        }
        rows = rows.child(header("TAGS"));
        for (i, (tag, count)) in self.tags.iter().enumerate() {
            rows = rows.child(self.side_row(
                ("tag", i),
                format!("#{tag}"),
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
                            .label("+ Note")
                            .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx))),
                    )
                    .child(
                        Button::new("new-folder")
                            .small()
                            .label("+ Folder")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_overlay(Overlay::NewFolder, window, cx)
                            })),
                    )
                    .child(
                        Button::new("jump")
                            .small()
                            .label("Go to…")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_overlay(Overlay::Switcher, window, cx)
                            })),
                    ),
            )
            .child(rows)
            .child(
                div()
                    .p_2()
                    .text_xs()
                    .text_color(muted)
                    .child(if self.status.is_empty() {
                        SharedString::from("Ctrl+P jump · Ctrl+B panels · Ctrl+K Z zen")
                    } else {
                        self.status.clone()
                    }),
            )
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
                                this.open_hit(path.clone(), window, cx)
                            }))
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(note.title.clone()),
                            )
                            .child(div().truncate().text_xs().text_color(muted).child(second)),
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
            .child(div().p_2().child(Input::new(&self.search).cleanable(true)))
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
        h_flex()
            .p_2()
            .gap_2()
            .child(
                Button::new("toggle-sidebar")
                    .small()
                    .ghost()
                    .selected(self.show_sidebar)
                    .label("Folders")
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
                    .label("Notes")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_list = !this.show_list;
                        cx.notify();
                    })),
            )
            .child(div().flex_1().min_w_0().when(has_note, |el| el.child(Input::new(&self.title))))
            .when(has_note, |el| {
                el.child(
                    Button::new("delete")
                        .small()
                        .ghost()
                        .label("Delete")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.delete_current(window, cx)),
                        ),
                )
            })
            .child(
                Button::new("zen")
                    .small()
                    .ghost()
                    .label("Zen")
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
                            .label("Exit zen")
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
                    .text_color(muted)
                    .child("Select a note, or press Ctrl+N to create one."),
            );
        };
        let mut info = if note.folder.is_empty() {
            "Notes".to_string()
        } else {
            note.folder.clone()
        };
        for tag in &note.tags {
            info.push_str("   #");
            info.push_str(tag);
        }
        column
            .when(!self.zen, |el| {
                el.child(div().px_3().pb_1().text_xs().text_color(muted).child(info))
            })
            .child(
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
                    .child(Editor::new(&self.editor).h_full().bordered(!self.zen)),
            )
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
            let note = &self.store.notes[i];
            let path = note.path.clone();
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
                        this.open_note(path.clone(), window, cx);
                        this.close_overlay(window, cx);
                    }))
                    .child(div().truncate().child(note.title.clone()))
                    .child(div().truncate().text_xs().text_color(muted).child(note.folder.clone())),
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
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.zen = false;
                this.show_list = true;
                this.search.update(cx, |s, cx| {
                    s.focus(window, cx);
                    s.select_all(window, cx);
                });
                cx.notify();
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

fn notes_root() -> PathBuf {
    if let Some(arg) = std::env::args_os().nth(1) {
        return PathBuf::from(arg);
    }
    let home = std::env::var_os("USERPROFILE").unwrap_or_default();
    PathBuf::from(home).join("Documents").join("Notes")
}

fn main() {
    let root = notes_root();
    let store = Store::open(root.clone())
        .unwrap_or_else(|e| panic!("cannot open notes folder {}: {e}", root.display()));

    // The bundled icon set is needed by the editor's find/replace panel.
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.run(move |cx| {
        gpui_kit::init(cx);
        cx.bind_keys([
            // App-wide, as in VS Code.
            KeyBinding::new("ctrl-p", QuickOpen, None),
            KeyBinding::new("ctrl-n", NewNote, None),
            KeyBinding::new("ctrl-shift-n", NewFolder, None),
            KeyBinding::new("ctrl-shift-f", FocusSearch, None),
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
                title: Some(format!("Notes — {}", root.display()).into()),
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
