//! Note storage: plain `.md` / `.txt` files under one root folder.
//! Everything is kept in memory so search and tag filtering are instant.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct Note {
    pub path: PathBuf,
    /// Folder relative to the root, with `/` separators. Empty for the root.
    pub folder: String,
    pub title: String,
    pub content: String,
    lower: String,
    pub tags: Vec<String>,
    pub modified: SystemTime,
    /// The file uses Windows line endings; they are kept when saving.
    crlf: bool,
}

impl Note {
    fn load(root: &Path, path: PathBuf) -> Option<Note> {
        let raw = String::from_utf8_lossy(&fs::read(&path).ok()?).into_owned();
        let crlf = raw.contains("\r\n");
        let content = raw.replace("\r\n", "\n");
        let modified = fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let folder = path
            .parent()
            .and_then(|p| p.strip_prefix(root).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let title = path.file_stem()?.to_string_lossy().into_owned();
        let mut note = Note {
            path,
            folder,
            title,
            content: String::new(),
            lower: String::new(),
            tags: Vec::new(),
            modified,
            crlf,
        };
        note.set_content(content);
        Some(note)
    }

    fn set_content(&mut self, content: String) {
        self.lower = content.to_lowercase();
        self.tags = extract_tags(&content);
        self.content = content;
    }
}

pub struct SearchHit {
    pub index: usize,
    /// The first line that matched, trimmed. Empty when only the title matched.
    pub snippet: String,
}

pub struct Store {
    pub root: PathBuf,
    pub notes: Vec<Note>,
}

impl Store {
    pub fn open(root: PathBuf) -> io::Result<Store> {
        fs::create_dir_all(&root)?;
        let mut store = Store {
            root,
            notes: Vec::new(),
        };
        store.reload();
        Ok(store)
    }

    /// Re-read every note from disk.
    pub fn reload(&mut self) {
        let mut paths = Vec::new();
        collect_files(&self.root, &mut paths);
        self.notes = paths
            .into_iter()
            .filter_map(|p| Note::load(&self.root, p))
            .collect();
        self.sort();
    }

    fn sort(&mut self) {
        self.notes.sort_by(|a, b| b.modified.cmp(&a.modified));
    }

    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.notes.iter().position(|n| n.path == path)
    }

    /// All folders that contain notes (plus their parents), sorted.
    pub fn folders(&self) -> Vec<String> {
        let mut set = std::collections::BTreeSet::new();
        collect_dirs(&self.root, &self.root, &mut set);
        set.into_iter().collect()
    }

    /// Every tag with the number of notes using it, sorted by name.
    pub fn tags(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for note in &self.notes {
            for tag in &note.tags {
                *counts.entry(tag).or_default() += 1;
            }
        }
        counts.into_iter().map(|(t, c)| (t.to_string(), c)).collect()
    }

    /// Full-text search. Every whitespace-separated term must appear in the
    /// title or the content. Title matches come first, then newest first.
    /// `folder` and `tag` narrow the result when set.
    pub fn search(&self, query: &str, folder: Option<&str>, tag: Option<&str>) -> Vec<SearchHit> {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut hits: Vec<(bool, SearchHit)> = Vec::new();
        for (index, note) in self.notes.iter().enumerate() {
            if let Some(folder) = folder {
                let inside = note.folder == folder
                    || note
                        .folder
                        .strip_prefix(folder)
                        .is_some_and(|rest| rest.starts_with('/'));
                if !inside {
                    continue;
                }
            }
            if let Some(tag) = tag {
                if !note.tags.iter().any(|t| t == tag) {
                    continue;
                }
            }
            let title = note.title.to_lowercase();
            if !terms
                .iter()
                .all(|t| title.contains(t) || note.lower.contains(t))
            {
                continue;
            }
            let in_title = !terms.is_empty() && terms.iter().all(|t| title.contains(t));
            let snippet = terms
                .iter()
                .find_map(|t| note.lower.find(t.as_str()))
                .map(|pos| line_at(&note.content, &note.lower, pos))
                .unwrap_or_default();
            hits.push((in_title, SearchHit { index, snippet }));
        }
        // Notes are already newest first; a stable sort keeps that order.
        hits.sort_by_key(|(in_title, _)| !*in_title);
        hits.into_iter().map(|(_, hit)| hit).collect()
    }

    /// Byte range in the note's content of the first place `query` matches,
    /// so the editor can jump to it.
    pub fn match_range(&self, index: usize, query: &str) -> Option<std::ops::Range<usize>> {
        let note = &self.notes[index];
        let (pos, term) = query
            .split_whitespace()
            .map(str::to_lowercase)
            .find_map(|t| note.lower.find(&t).map(|p| (p, t)))?;
        // Lowercasing can change byte lengths, so map through character counts.
        let skip = note.lower[..pos].chars().count();
        let mut bounds = note
            .content
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(note.content.len()));
        let start = bounds.nth(skip)?;
        let end = bounds.nth(term.chars().count() - 1)?;
        Some(start..end)
    }

    /// Fuzzy match on titles for the quick switcher, best match first.
    pub fn fuzzy_titles(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        let mut scored: Vec<(i32, usize)> = self
            .notes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| fuzzy_score(&query, &n.title.to_lowercase()).map(|s| (s, i)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().map(|(_, i)| i).collect()
    }

    /// Create an empty note. `folder` is relative to the root. Returns its path.
    pub fn create(&mut self, folder: &str, title: &str) -> io::Result<PathBuf> {
        let name = sanitize(title);
        let name = if name.is_empty() { "Untitled".to_string() } else { name };
        let dir = self.root.join(folder);
        fs::create_dir_all(&dir)?;
        let mut path = dir.join(format!("{name}.md"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{name} {n}.md"));
            n += 1;
        }
        fs::write(&path, "")?;
        if let Some(note) = Note::load(&self.root, path.clone()) {
            self.notes.insert(0, note);
        }
        Ok(path)
    }

    pub fn create_folder(&mut self, folder: &str) -> io::Result<()> {
        let clean: Vec<String> = folder
            .split(['/', '\\'])
            .map(sanitize)
            .filter(|p| !p.is_empty())
            .collect();
        if clean.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(self.root.join(clean.join("/")))
    }

    /// Write new content for a note to disk and update the index.
    pub fn save(&mut self, path: &Path, content: &str) -> io::Result<()> {
        let i = self.index_of(path);
        if i.is_some_and(|i| self.notes[i].crlf) {
            fs::write(path, content.replace('\n', "\r\n"))?;
        } else {
            fs::write(path, content)?;
        }
        if let Some(i) = i {
            let note = &mut self.notes[i];
            note.set_content(content.to_string());
            note.modified = SystemTime::now();
            self.sort();
        }
        Ok(())
    }

    /// Rename a note's file, keeping its folder and extension. Returns the new path.
    pub fn rename(&mut self, path: &Path, title: &str) -> io::Result<PathBuf> {
        let name = sanitize(title);
        if name.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty title"));
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("md");
        let new_path = path.with_file_name(format!("{name}.{ext}"));
        if new_path == path {
            return Ok(new_path);
        }
        if new_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a note with that name already exists",
            ));
        }
        fs::rename(path, &new_path)?;
        if let Some(i) = self.index_of(path) {
            self.notes[i].path = new_path.clone();
            self.notes[i].title = name;
        }
        Ok(new_path)
    }

    /// Move a note into the hidden `.trash` folder under the root, so a
    /// delete can be undone by moving the file back.
    pub fn delete(&mut self, path: &Path) -> io::Result<()> {
        let trash = self.root.join(".trash");
        fs::create_dir_all(&trash)?;
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let mut target = trash.join(&name);
        let mut n = 2;
        while target.exists() {
            target = trash.join(format!("{n} {name}"));
            n += 1;
        }
        fs::rename(path, target)?;
        self.notes.retain(|n| n.path != path);
        Ok(())
    }
}

fn is_note_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(),
        Some("md" | "txt")
    )
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_hidden(&path) {
            continue;
        }
        if path.is_dir() {
            collect_files(&path, out);
        } else if is_note_file(&path) {
            out.push(path);
        }
    }
}

fn collect_dirs(root: &Path, dir: &Path, out: &mut std::collections::BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && !is_hidden(&path) {
            if let Ok(rel) = path.strip_prefix(root) {
                out.insert(rel.to_string_lossy().replace('\\', "/"));
            }
            collect_dirs(root, &path, out);
        }
    }
}

/// Remove characters Windows does not allow in file names.
fn sanitize(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') && !c.is_control())
        .collect::<String>()
        .trim()
        .trim_end_matches('.')
        .to_string()
}

/// `#tag` words: `#` at the start of a word, followed by letters, digits,
/// `_`, `-` or `/`, containing at least one letter. Returned lowercase.
fn extract_tags(content: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut prev_is_space = true;
    let mut chars = content.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '#' && prev_is_space {
            let rest = &content[i + 1..];
            let end = rest
                .find(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '-' | '/')))
                .unwrap_or(rest.len());
            let tag = &rest[..end];
            if tag.chars().any(char::is_alphabetic) {
                let tag = tag.to_lowercase();
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
        }
        prev_is_space = c.is_whitespace();
    }
    tags.sort();
    tags
}

/// The line of `content` around byte offset `pos` of its lowercase form.
fn line_at(content: &str, lower: &str, pos: usize) -> String {
    // Lowercasing can change byte lengths, so locate the line by its number.
    let line_no = lower[..pos].matches('\n').count();
    let line = content.lines().nth(line_no).unwrap_or("").trim();
    line.chars().take(160).collect()
}

/// Subsequence match. Higher is better; `None` when `query` does not match.
fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let mut score = 0;
    let mut last: Option<usize> = None;
    let mut chars = text.chars().enumerate();
    for q in query.chars().filter(|c| !c.is_whitespace()) {
        let (i, _) = chars.by_ref().find(|(_, c)| *c == q)?;
        score += match last {
            Some(l) if i == l + 1 => 5,
            _ => 1,
        };
        if i == 0 {
            score += 3;
        }
        last = Some(i);
    }
    if text.contains(query) {
        score += 20;
    }
    Some(score - text.chars().count() as i32 / 8)
}
