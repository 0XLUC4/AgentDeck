//! Files page: the workspace folder as a tree (or a flat list of name matches), the selected file
//! beside it (text in a code editor: highlighting, line numbers, Ctrl+S saves; Markdown in a visual
//! editor by default, its source one click away), and a button typing its path into the active
//! pane. A file changed on disk (by an agent) while open and unedited is reloaded.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use agentdeck_core::logic::markdown_blocks::Doc;
use agentdeck_core::logic::theme::FONT_MONO;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState, Position};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::Disableable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::markdown_editor::MarkdownEditor;
use super::viewer::ModelViewer;
use super::{EDITOR_CONTEXT, SaveAgentFile, confirm};
use crate::orchestrator::runtime::send_prompt;
use crate::state::Deck;
use crate::theme::{hsla, palette, ui};
use agentdeck_core::logic::theme::{RADIUS_LG, RADIUS_MD};

const SKIPPED_FOLDERS: &[&str] = &[".git", "node_modules", "target", "dist", ".next"];
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];
const MAX_TEXT_BYTES: u64 = 512 * 1024;
const MAX_WALKED_FILES: usize = 5000;
const MAX_MATCHES: usize = 200;
/// How often the open file is checked for changes made outside the editor.
const WATCH_EVERY: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    name: String,
    dir: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Markdown,
    Image,
    /// Text when small enough and valid UTF-8, see [`decode_text`].
    Other,
}

fn is_skipped(folder_name: &str) -> bool {
    SKIPPED_FOLDERS.contains(&folder_name)
}

/// Folders first, then files, each alphabetical without case.
fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by_cached_key(|e| (!e.dir, e.name.to_lowercase()));
}

fn name_matches(name: &str, query: &str) -> bool {
    name.to_lowercase().contains(&query.to_lowercase())
}

/// `path` relative to `root` with forward slashes (what agents expect after `@`).
fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn kind_of(path: &Path) -> Kind {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "md" || ext == "markdown" {
        Kind::Markdown
    } else if IMAGE_EXTS.contains(&ext.as_str()) {
        Kind::Image
    } else {
        Kind::Other
    }
}

/// The bytes as text, or None for a binary file (not UTF-8, or with a NUL byte).
fn decode_text(bytes: Vec<u8>) -> Option<String> {
    String::from_utf8(bytes).ok().filter(|text| !text.contains('\0'))
}

/// Last modification time of a file, None when it cannot be read.
fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// Name the highlighter looks up: the extension (`rs`, `ts`), else the file name (`Makefile`).
fn language_token(path: &Path) -> String {
    path.extension().or_else(|| path.file_name()).map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Language shown in the header.
fn language_label(path: &Path) -> String {
    let token = language_token(path);
    let known = match token.as_str() {
        "rs" => "Rust",
        "ts" | "mts" | "cts" => "TypeScript",
        "tsx" => "TSX",
        "js" | "mjs" | "cjs" => "JavaScript",
        "jsx" => "JSX",
        "py" | "pyi" => "Python",
        "json" | "jsonc" => "JSON",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "md" | "mdx" => "Markdown",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "scss" => "SCSS",
        "go" => "Go",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" => "C++",
        "cs" => "C#",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "rb" => "Ruby",
        "php" => "PHP",
        "lua" => "Lua",
        "sh" | "bash" | "zsh" => "Shell",
        "ps1" | "psm1" => "PowerShell",
        "sql" => "SQL",
        "xml" | "svg" => "XML",
        "zig" => "Zig",
        "txt" | "" => "Plain text",
        _ => return token.to_uppercase(),
    };
    known.to_owned()
}

fn format_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// Sorted content of one folder, without the skipped folders. A symlink counts as a file, so a
/// link loop cannot be walked.
fn read_folder(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let dir = item.file_type().is_ok_and(|t| t.is_dir());
        if dir && is_skipped(&name) {
            continue;
        }
        entries.push(Entry { path: item.path(), name, dir });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

/// Files of the project whose name contains `query`. Stops after `MAX_WALKED_FILES` files seen or
/// `MAX_MATCHES` found; unreadable folders are skipped.
fn find_files(root: &Path, query: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut walked = 0;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in read_folder(&dir).unwrap_or_default() {
            if entry.dir {
                pending.push(entry.path);
                continue;
            }
            walked += 1;
            if name_matches(&entry.name, query) {
                found.push(entry.path);
            }
            if walked >= MAX_WALKED_FILES || found.len() >= MAX_MATCHES {
                return found;
            }
        }
    }
    found
}

enum Preview {
    /// No file selected.
    Empty,
    Loading,
    Image,
    /// Text in `FilesPage::editor` (and `markdown` for a Markdown file); holds the content last
    /// read or saved (dirty check).
    Text(String),
    /// A 3D model, shown by `FilesPage::viewer`.
    Model,
    /// Binary or too large: only the size in bytes.
    Unavailable(u64),
    Error(String),
}

fn load_preview(path: &Path) -> Preview {
    let kind = kind_of(path);
    if kind == Kind::Image {
        return Preview::Image;
    }
    let size = match fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(e) => return Preview::Error(e.to_string()),
    };
    if size > MAX_TEXT_BYTES {
        return Preview::Unavailable(size);
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => return Preview::Error(e.to_string()),
    };
    match decode_text(bytes) {
        Some(text) => Preview::Text(text),
        None => Preview::Unavailable(size),
    }
}

/// One line of the tree.
enum Row {
    Entry(Entry),
    /// State of an open folder: reading, empty, or the read error.
    Note(String),
}

pub struct FilesPage {
    deck: Entity<Deck>,
    workspace_id: String,
    pub focus: FocusHandle,
    /// The workspace folder; empty when none is linked.
    root: PathBuf,
    /// Content of the folders read so far.
    folders: HashMap<PathBuf, Result<Vec<Entry>, String>>,
    expanded: HashSet<PathBuf>,
    find: Entity<InputState>,
    /// Result of the last name search; None while it runs.
    matches: Option<Vec<PathBuf>>,
    selected: Option<PathBuf>,
    preview: Preview,
    /// The 3D viewer of the selected model (same one as the Assets page), redrawn as it loads.
    viewer: Option<(Entity<ModelViewer>, Subscription)>,
    /// Code editor of the selected text file, redrawn as it is typed in (dirty mark).
    editor: Option<(Entity<EditorState>, Subscription)>,
    /// Visual editor of a Markdown file, beside `editor` (its source view); the shown one holds
    /// the current text, the other catches up on a switch.
    markdown: Option<(Entity<MarkdownEditor>, Subscription)>,
    /// A Markdown file is shown as source (`editor`) rather than in the visual editor.
    source_mode: bool,
    /// The headings list of a Markdown file is open.
    outline: bool,
    /// Modification time of the open file when last read or saved; another one means it changed.
    disk_mtime: Option<SystemTime>,
    /// Checks the open text file for changes made on disk.
    watching: Option<Task<()>>,
    /// 0-based line to put the cursor on once the file is in the editor.
    goto_line: Option<u32>,
    /// Opened from a git review: "Back to review" closes the page (the panel comes back).
    pub(super) from_review: bool,
    // Replacing one of these drops (cancels) the previous run.
    searching: Option<Task<()>>,
    loading: Option<Task<()>>,
    _find_events: Subscription,
}

impl FilesPage {
    pub fn new(deck: Entity<Deck>, workspace_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root = deck
            .read(cx)
            .state
            .workspaces
            .iter()
            .find(|w| w.id == workspace_id)
            .map(|w| PathBuf::from(&w.path))
            .unwrap_or_default();
        Self::at(deck, workspace_id, root, window, cx)
    }

    /// The page on any folder (a pane's repository or worktree, for the git review).
    pub(super) fn at(deck: Entity<Deck>, workspace_id: String, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let find = cx.new(|cx| InputState::new(window, cx).placeholder("Find files"));
        let find_events = cx.subscribe_in(&find, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.search(cx);
            }
        });
        let mut page = Self {
            deck,
            workspace_id,
            focus: cx.focus_handle(),
            root: root.clone(),
            folders: HashMap::new(),
            expanded: HashSet::new(),
            find,
            matches: None,
            selected: None,
            preview: Preview::Empty,
            viewer: None,
            editor: None,
            markdown: None,
            source_mode: false,
            outline: false,
            disk_mtime: None,
            watching: None,
            goto_line: None,
            from_review: false,
            searching: None,
            loading: None,
            _find_events: find_events,
        };
        if !root.as_os_str().is_empty() {
            page.load_folder(root, cx);
        }
        page
    }

    fn load_folder(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let read = dir.clone();
            let result = cx.background_executor().spawn(async move { read_folder(&read) }).await;
            let _ = this.update(cx, |this, cx| {
                this.folders.insert(dir, result);
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        if !self.expanded.remove(&dir) {
            self.expanded.insert(dir.clone());
            if !self.folders.contains_key(&dir) {
                self.load_folder(dir, cx);
            }
        }
        cx.notify();
    }

    fn query(&self, cx: &App) -> String {
        self.find.read(cx).value().trim().to_string()
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.matches = None;
        self.searching = (!query.is_empty()).then(|| {
            let root = self.root.clone();
            cx.spawn(async move |this, cx| {
                let found = cx.background_executor().spawn(async move { find_files(&root, &query) }).await;
                let _ = this.update(cx, |this, cx| {
                    this.matches = Some(found);
                    cx.notify();
                });
            })
        });
        cx.notify();
    }

    /// The shown editor holds changes not written to the file yet.
    pub(super) fn has_unsaved(&self, cx: &App) -> bool {
        let Preview::Text(saved) = &self.preview else { return false };
        match (&self.markdown, &self.editor) {
            (Some((markdown, _)), _) if !self.source_mode => markdown.read(cx).text() != saved.as_str(),
            (_, Some((editor, _))) => editor.read(cx).value().as_ref() != saved.as_str(),
            _ => false,
        }
    }

    /// Text of the shown editor.
    fn current_text(&self, cx: &App) -> Option<String> {
        match (&self.markdown, &self.editor) {
            (Some((markdown, _)), _) if !self.source_mode => Some(markdown.read(cx).text().to_owned()),
            (_, Some((editor, _))) => Some(editor.read(cx).value().to_string()),
            _ => None,
        }
    }

    /// Markdown as source (code editor) or visual; both show the same text.
    fn set_source_mode(&mut self, source: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((markdown, _)), Some((editor, _))) = (&self.markdown, &self.editor) else { return };
        if source == self.source_mode {
            return;
        }
        let (markdown, editor) = (markdown.clone(), editor.clone());
        if source {
            let text = markdown.read(cx).text().to_owned();
            editor.update(cx, |editor, cx| {
                if editor.value().as_ref() != text.as_str() {
                    editor.set_value(text, window, cx);
                }
                editor.focus(window, cx);
            });
        } else {
            let text = editor.read(cx).value().to_string();
            markdown.update(cx, |markdown, cx| markdown.set_text(&text, window, cx));
        }
        self.source_mode = source;
        cx.notify();
    }

    /// Checks the open file every [`WATCH_EVERY`]: changed on disk while not edited here, it is
    /// read again.
    fn watch(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.disk_mtime = modified(&path);
        self.watching = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(WATCH_EVERY).await;
                let file = path.clone();
                let mtime = cx.background_executor().spawn(async move { modified(&file) }).await;
                let Ok(stale) = this.update_in(cx, |this, _, cx| mtime.is_some() && mtime != this.disk_mtime && !this.has_unsaved(cx))
                else {
                    break;
                };
                if !stale {
                    continue;
                }
                let file = path.clone();
                let text = cx.background_executor().spawn(async move { fs::read(&file).ok().and_then(decode_text) }).await;
                if this.update_in(cx, |this, window, cx| this.reload(text, mtime, window, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// The file as just read from disk replaces the (unedited) content of both editors.
    fn reload(&mut self, text: Option<String>, mtime: Option<SystemTime>, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_unsaved(cx) {
            return;
        }
        self.disk_mtime = mtime;
        let (Some(text), Some((editor, _))) = (text, &self.editor) else { return };
        let editor = editor.clone();
        editor.update(cx, |editor, cx| {
            if editor.value().as_ref() != text.as_str() {
                editor.set_value(text, window, cx);
            }
        });
        // As the editor holds it (line endings normalized), like on open.
        let saved = editor.read(cx).value().to_string();
        if matches!(&self.preview, Preview::Text(old) if *old == saved) {
            return;
        }
        if let Some((markdown, _)) = &self.markdown {
            markdown.update(cx, |markdown, cx| markdown.set_text(&saved, window, cx));
        }
        self.preview = Preview::Text(saved);
        cx.notify();
    }

    /// Runs `then` now, or once the user agrees to drop unsaved changes.
    fn leave(
        &mut self,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_unsaved(cx) {
            return then(self, window, cx);
        }
        cx.spawn_in(window, async move |this, cx| {
            if confirm("Discard your changes?".into(), "Discard", cx).await {
                let _ = this.update_in(cx, then);
            }
        })
        .detach();
    }

    fn select(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.leave(move |this, window, cx| this.open(path, window, cx), window, cx);
    }

    fn close_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave(
            |this, _, cx| {
                this.selected = None;
                this.viewer = None;
                this.editor = None;
                this.markdown = None;
                this.watching = None;
                cx.notify();
            },
            window,
            cx,
        );
    }

    /// Shows `text` in the code editor (Markdown: in the visual editor, unless a line is asked
    /// for); it becomes the saved content.
    fn edit(&mut self, path: &Path, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let editor = cx.new(|cx| EditorState::new(window, cx).language(language_token(path)).default_value(text.to_owned()));
        let changes = cx.subscribe(&editor, |_, _, _: &InputEvent, cx| cx.notify());
        // As the editor holds it (line endings normalized), so an untouched file is not dirty.
        let saved = editor.read(cx).value().to_string();
        self.markdown = (kind_of(path) == Kind::Markdown).then(|| {
            let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let view = cx.new(|cx| MarkdownEditor::new(&saved, dir, window, cx));
            let redraw = cx.observe(&view, |_, _, cx| cx.notify());
            (view, redraw)
        });
        self.source_mode = self.markdown.is_none() || self.goto_line.is_some();
        self.preview = Preview::Text(saved);
        self.watch(path.to_path_buf(), window, cx);
        if let Some(line) = self.goto_line.take() {
            let at = Position::new(line, 0);
            editor.update(cx, |e, cx| e.set_cursor_position(at, window, cx));
            // The editor scrolls to its cursor only once laid out: again after its first frames.
            let weak = editor.downgrade();
            cx.spawn_in(window, async move |_, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                let _ = cx.update(|window, cx| weak.update(cx, |e, cx| e.set_cursor_position(at, window, cx)));
            })
            .detach();
        }
        self.editor = Some((editor, changes));
        cx.notify();
    }

    /// Opens `path` in the editor with the cursor on `line` (0-based), from a git review.
    pub(super) fn open_at(&mut self, path: PathBuf, line: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.from_review = true;
        self.goto_line = Some(line);
        self.open(path, window, cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let (Some(path), Some(text)) = (self.selected.clone(), self.current_text(cx)) else { return };
        if !self.has_unsaved(cx) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let (file, content) = (path.clone(), text.clone());
            let result = cx.background_executor().spawn(async move { fs::write(&file, content).map(|()| modified(&file)) }).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    // Still the same file: what was written is the new saved content (not a change
                    // made on disk by someone else).
                    Ok(mtime) if this.selected.as_ref() == Some(&path) => {
                        this.preview = Preview::Text(text);
                        this.disk_mtime = mtime;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        this.deck.update(cx, |deck, cx| deck.toast(format!("Could not save {name}: {e}"), cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(path.clone());
        self.editor = None;
        self.markdown = None;
        self.watching = None;
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        self.viewer = super::assets::MODEL_EXTS.contains(&ext.as_str()).then(|| {
            let (cwd, file) = (self.root.to_string_lossy().into_owned(), path.to_string_lossy().into_owned());
            let viewer = cx.new(|cx| ModelViewer::new(cwd, file, ext, cx));
            let redraw = cx.observe(&viewer, |_, _, cx| cx.notify());
            (viewer, redraw)
        });
        if self.viewer.is_some() {
            self.preview = Preview::Model;
            self.loading = None;
            cx.notify();
            return;
        }
        self.preview = Preview::Loading;
        self.loading = Some(cx.spawn_in(window, async move |this, cx| {
            let read = path.clone();
            let preview = cx.background_executor().spawn(async move { load_preview(&read) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match preview {
                    Preview::Text(text) => this.edit(&path, &text, window, cx),
                    preview => this.preview = preview,
                }
                this.goto_line = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// The visible lines of the tree under `dir`, depth first.
    fn push_rows(&self, dir: &Path, depth: usize, rows: &mut Vec<(usize, Row)>) {
        match self.folders.get(dir) {
            None => rows.push((depth, Row::Note("Reading…".into()))),
            Some(Err(e)) => rows.push((depth, Row::Note(e.clone()))),
            Some(Ok(entries)) if entries.is_empty() => rows.push((depth, Row::Note("Empty".into()))),
            Some(Ok(entries)) => {
                for entry in entries {
                    rows.push((depth, Row::Entry(entry.clone())));
                    if entry.dir && self.expanded.contains(&entry.path) {
                        self.push_rows(&entry.path, depth + 1, rows);
                    }
                }
            }
        }
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = palette(cx);
        let mut rows = Vec::new();
        self.push_rows(&self.root, 0, &mut rows);
        rows.into_iter()
            .enumerate()
            .map(|(i, (depth, row))| {
                let line = div().h(ui(26.0)).flex().items_center().gap(ui(6.0)).pl(ui(8.0 + 14.0 * depth as f32)).pr_2();
                let entry = match row {
                    Row::Entry(entry) => entry,
                    Row::Note(note) => {
                        return line
                            .text_size(ui(12.0))
                            .text_color(hsla(p.subtle))
                            .child(div().truncate().child(note))
                            .into_any_element();
                    }
                };
                let current = self.selected.as_ref() == Some(&entry.path);
                let icon = match (entry.dir, self.expanded.contains(&entry.path)) {
                    (true, true) => IconName::ChevronDown,
                    (true, false) => IconName::ChevronRight,
                    (false, _) => IconName::FileText,
                };
                line.id(("file-row", i))
                    .rounded(ui(RADIUS_MD))
                    .cursor_pointer()
                    .when(current, |d| d.bg(hsla(p.raised)))
                    .when(!current, |d| d.hover(|s| s.bg(hsla(p.hover))))
                    .child(div().flex_shrink_0().text_size(ui(13.0)).text_color(hsla(p.subtle)).child(icon))
                    .child(div().truncate().child(entry.name.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if entry.dir {
                            this.toggle(entry.path.clone(), cx);
                        } else {
                            this.select(entry.path.clone(), window, cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect()
    }

    fn render_matches(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = palette(cx);
        let note = |text: &'static str| {
            vec![div().px_2().py_1().text_size(ui(12.0)).text_color(hsla(p.subtle)).child(text).into_any_element()]
        };
        let Some(matches) = &self.matches else {
            return note("Searching…");
        };
        if matches.is_empty() {
            return note("No file with this name.");
        }
        matches
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let current = self.selected.as_ref() == Some(path);
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let path = path.clone();
                div()
                    .id(("file-match", i))
                    .flex()
                    .flex_col()
                    .rounded(ui(RADIUS_MD))
                    .px_2()
                    .py(ui(4.0))
                    .cursor_pointer()
                    .when(current, |d| d.bg(hsla(p.raised)))
                    .when(!current, |d| d.hover(|s| s.bg(hsla(p.hover))))
                    .child(div().truncate().child(name))
                    .child(
                        div()
                            .truncate()
                            .text_size(ui(11.0))
                            .text_color(hsla(p.subtle))
                            .child(relative_path(&self.root, &path)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.select(path.clone(), window, cx)))
                    .into_any_element()
            })
            .collect()
    }

    /// Header controls of a Markdown file: the source | visual toggle and the outline button.
    fn render_mode_toggle(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let button = |id: &'static str, icon: IconName, tip: &'static str, pressed: bool| {
            div()
                .id(id)
                .size(ui(24.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(ui(RADIUS_MD))
                .text_size(ui(13.0))
                .cursor_pointer()
                .when(pressed, |d| d.bg(hsla(p.base)).text_color(hsla(p.fg)))
                .when(!pressed, |d| d.text_color(hsla(p.muted)).hover(|s| s.text_color(hsla(p.fg))))
                .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                .child(icon)
        };
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .child(
                div()
                    .flex()
                    .gap(px(2.0))
                    .p(px(2.0))
                    .rounded(ui(RADIUS_LG))
                    .bg(hsla(p.raised))
                    .child(
                        button("files-md-source", IconName::CodeXml, "Markdown source", self.source_mode)
                            .on_click(cx.listener(|this, _, window, cx| this.set_source_mode(true, window, cx))),
                    )
                    .child(
                        button("files-md-visual", IconName::Pencil, "Visual editor", !self.source_mode)
                            .on_click(cx.listener(|this, _, window, cx| this.set_source_mode(false, window, cx))),
                    ),
            )
            .child(
                button("files-md-outline", IconName::TableOfContents, "Outline", self.outline).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.outline = !this.outline;
                        cx.notify();
                    },
                )),
            )
    }

    /// Headings of the Markdown file, indented by level; a click goes to one.
    fn render_outline(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette(cx);
        // (target, level, text): a block index in the visual editor, a line in the source.
        let headings: Vec<(usize, u8, String)> = match (&self.markdown, &self.editor) {
            (Some((markdown, _)), _) if !self.source_mode => markdown.read(cx).outline(),
            (_, Some((editor, _))) => {
                let doc = Doc::parse(&editor.read(cx).value());
                doc.headings().into_iter().map(|(i, level, text)| (doc.line_of(i), level, text.to_owned())).collect()
            }
            _ => Vec::new(),
        };
        let empty = headings.is_empty();
        let rows: Vec<Stateful<Div>> = headings
            .into_iter()
            .enumerate()
            .map(|(n, (target, level, text))| {
                div()
                    .id(("files-outline-row", n))
                    .py(ui(3.0))
                    .pr_2()
                    .pl(ui(8.0 + 12.0 * level.saturating_sub(1) as f32))
                    .rounded(ui(RADIUS_MD))
                    .truncate()
                    .cursor_pointer()
                    .when(level == 1, |d| d.font_weight(FontWeight::MEDIUM))
                    .when(level > 1, |d| d.text_color(hsla(p.muted)))
                    .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                    .child(if text.is_empty() { "Untitled".to_owned() } else { text })
                    .on_click(cx.listener(move |this, _, window, cx| this.go_to_heading(target, window, cx)))
            })
            .collect();
        div()
            .id("files-outline")
            .flex_shrink_0()
            .w(ui(220.0))
            .h_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.panel))
            .p_2()
            .text_size(ui(12.0))
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .text_size(ui(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(p.subtle))
                    .child("Outline"),
            )
            .when(empty, |d| d.child(div().px_2().text_color(hsla(p.subtle)).child("No headings")))
            .children(rows)
    }

    /// Outline click: the heading's block (visual) or line (source) gets the caret.
    fn go_to_heading(&mut self, target: usize, window: &mut Window, cx: &mut Context<Self>) {
        match (&self.markdown, &self.editor) {
            (Some((markdown, _)), _) if !self.source_mode => {
                let markdown = markdown.clone();
                markdown.update(cx, |markdown, cx| markdown.jump_to(target, window, cx));
            }
            (_, Some((editor, _))) => {
                let editor = editor.clone();
                let at = Position::new(target as u32, 0);
                editor.update(cx, |editor, cx| editor.set_cursor_position(at, window, cx));
            }
            _ => {}
        }
    }

    fn render_preview(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let side = div().flex_1().min_w_0().min_h_0().flex().flex_col();
        // Edge to edge under the file's line: no box inside the page.
        let frame = || div().flex_1().min_h_0().overflow_hidden().border_t_1().border_color(hsla(p.line)).bg(hsla(p.base));
        let message = |text: String, color: u32| {
            frame().flex().items_center().justify_center().px_6().text_color(hsla(color)).child(text)
        };
        let Some(path) = self.selected.clone() else {
            return side.child(message("Select a file to preview it.".into(), p.subtle));
        };
        let rel = relative_path(&self.root, &path);
        let content = match &self.preview {
            Preview::Empty | Preview::Loading => message("Reading…".into(), p.subtle),
            Preview::Error(e) => message(e.clone(), p.bad),
            Preview::Unavailable(size) => message(format!("No preview ({})", format_size(*size)), p.subtle),
            Preview::Image => frame().flex().items_center().justify_center().p_4().child(
                img(path.clone())
                    .max_w_full()
                    .max_h_full()
                    .object_fit(ObjectFit::Contain)
                    .with_fallback(move || {
                        div().text_color(hsla(p.bad)).child("This image could not be read.").into_any_element()
                    }),
            ),
            Preview::Model => match &self.viewer {
                Some((viewer, _)) => {
                    let (error, loading) = {
                        let v = viewer.read(cx);
                        (v.error.clone(), v.loading)
                    };
                    let note = error.or_else(|| (!loading.is_empty()).then(|| loading.to_owned()));
                    frame().relative().bg(hsla(super::assets::BACKDROP)).child(viewer.clone()).when_some(note, |d, note| {
                        d.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .px_6()
                                .text_color(hsla(0x55555a))
                                .child(note),
                        )
                    })
                }
                None => message("No preview".into(), p.subtle),
            },
            Preview::Text(_) => {
                let body = match (&self.markdown, &self.editor) {
                    (Some((markdown, _)), _) if !self.source_mode => Some(markdown.clone().into_any_element()),
                    (_, Some((editor, _))) => Some(Editor::new(editor).bordered(false).size_full().into_any_element()),
                    _ => None,
                };
                match body {
                    Some(body) => frame()
                        .flex()
                        .child(div().flex_1().min_w_0().h_full().child(body))
                        .when(self.outline && self.markdown.is_some(), |d| d.child(self.render_outline(cx))),
                    None => message("Reading…".into(), p.subtle),
                }
            }
        };
        let is_text = matches!(self.preview, Preview::Text(_));
        let is_markdown = is_text && self.markdown.is_some();
        let dirty = self.has_unsaved(cx);
        let can_send = self.deck.read(cx).state.active_pane_id.is_some();
        let typed = format!("@{rel} ");
        let (folder, name) = rel.rsplit_once('/').map_or((None, rel.clone()), |(d, n)| (Some(format!("{d}/")), n.to_owned()));
        let file_icon = match self.preview {
            Preview::Image => IconName::Image,
            Preview::Text(_) if !is_markdown => IconName::FileCode,
            _ => IconName::FileText,
        };
        let close = crate::panes::close_button("files-preview-close", "Close (Esc)")
            // Back to the panes (the tree stays open), or to the git review it came from.
            .on_click({
                let this = cx.entity().downgrade();
                move |window, cx| {
                    let _ = this.update(cx, |this, cx| {
                        if this.from_review {
                            super::close(window, cx);
                        } else {
                            this.close_file(window, cx);
                        }
                    });
                }
            });
        let from_review = self.from_review;
        let header = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .h(ui(40.0))
            .pl(ui(if from_review { 8.0 } else { 16.0 }))
            .pr_2()
            .when(from_review, |d| {
                d.child(
                    Button::new("files-back-to-review")
                        .ghost()
                        .icon(IconName::ArrowLeft)
                        .label("Back to review")
                        .tooltip("Close the editor and return to the Git panel (asks first if unsaved)")
                        .on_click(|_, window, cx| super::close(window, cx)),
                )
            })
            .child(div().flex_shrink_0().text_size(ui(14.0)).text_color(hsla(p.subtle)).child(file_icon))
            // Folder dimmed, file name in front: the name stays readable when the path is cut.
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .flex_1()
                    .items_center()
                    .gap_2()
                    .font_family(FONT_MONO)
                    .text_size(ui(12.0))
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .children(folder.map(|f| div().min_w_0().truncate().text_color(hsla(p.subtle)).child(f)))
                            .child(div().flex_shrink_0().font_weight(FontWeight::MEDIUM).child(name)),
                    )
                    .when(dirty, |d| {
                        // Unsaved changes, as in an editor tab.
                        d.child(div().flex_shrink_0().size(ui(8.0)).rounded_full().bg(hsla(p.wait)))
                    }),
            )
            .when(is_text && !is_markdown, |d| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .px_2()
                        .py(ui(2.0))
                        .rounded(ui(RADIUS_MD))
                        .bg(hsla(p.raised))
                        .text_size(ui(11.0))
                        .text_color(hsla(p.muted))
                        .child(language_label(&path)),
                )
            })
            .when(is_markdown, |d| d.child(self.render_mode_toggle(cx)))
            .when(is_text, |d| {
                d.child(
                    Button::new("files-save")
                        .primary()
                        .icon(IconName::Save)
                        .label("Save")
                        .tooltip("Save (Ctrl+S)")
                        .disabled(!dirty)
                        .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                )
            })
            .child(
                Button::new("files-send")
                    .ghost()
                    .icon(IconName::Send)
                    .label("Send path")
                    .tooltip("Types the path in the active pane, without sending it")
                    .disabled(!can_send)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let Some(pane_id) = this.deck.read(cx).state.active_pane_id.clone() else {
                            return;
                        };
                        send_prompt(&pane_id, &typed, false, cx);
                        this.deck.update(cx, |deck, cx| deck.toast("Path typed in the active pane", cx));
                    })),
            )
            .child(close);
        side.key_context(EDITOR_CONTEXT)
            .on_action(cx.listener(|this, _: &SaveAgentFile, _, cx| this.save(cx)))
            .child(header)
            .child(content)
    }
}

impl FilesPage {
    /// A file is picked: its preview takes the pane area.
    pub(super) fn has_main(&self) -> bool {
        self.selected.is_some()
    }

    /// The side column: find field and folder tree.
    pub(super) fn side(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let rows = if self.root.as_os_str().is_empty() {
            vec![div().p_2().text_xs().text_color(hsla(p.subtle)).child("Link a folder to this project to browse its files.").into_any_element()]
        } else if self.query(cx).is_empty() {
            self.render_tree(cx)
        } else {
            self.render_matches(cx)
        };
        let hover = hsla(p.hover);
        div()
            .id("files-side")
            .track_focus(&self.focus)
            .key_context(super::DIALOG_CONTEXT)
            .on_action(|_: &super::CloseSkillsDialog, window, cx| super::close(window, cx))
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap_2()
            .w(ui(280.0))
            .min_h_0()
            .p_2()
            .border_r_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.panel))
            .text_size(ui(13.0))
            .text_color(hsla(p.fg))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pl_1()
                    .child(div().text_size(ui(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(hsla(p.muted)).child("Files"))
                    .child(
                        div()
                            .id("files-side-close")
                            .p_1()
                            .rounded(ui(RADIUS_MD))
                            .text_color(hsla(p.muted))
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .child(IconName::X)
                            .on_click(|_, window, cx| super::close(window, cx)),
                    ),
            )
            .child(Input::new(&self.find))
            .child(div().id("files-list").flex_1().min_h_0().overflow_y_scroll().children(rows))
            .into_any_element()
    }
}

impl Render for FilesPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .occlude()
            .bg(hsla(p.panel))
            .text_color(hsla(p.fg))
            .text_size(ui(13.0))
            .font_family(agentdeck_core::logic::theme::FONT_SANS)
            .child(div().flex().flex_1().min_h_0().child(self.render_preview(cx)))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's glob exports its own `test` attribute.
    use std::path::{Path, PathBuf};

    use super::{
        Entry, Kind, decode_text, format_size, is_skipped, kind_of, language_label, language_token, name_matches,
        relative_path, sort_entries,
    };

    #[test]
    fn names_the_language_from_the_extension_or_file_name() {
        assert_eq!(language_token(Path::new("src/Main.RS")), "rs");
        assert_eq!(language_token(Path::new("Makefile")), "makefile");
        assert_eq!(language_label(Path::new("a.tsx")), "TSX");
        assert_eq!(language_label(Path::new("notes.txt")), "Plain text");
        assert_eq!(language_label(Path::new("x.proto")), "PROTO");
    }

    fn entry(name: &str, dir: bool) -> Entry {
        Entry { path: PathBuf::from(name), name: name.into(), dir }
    }

    #[test]
    fn sorts_folders_first_then_names_without_case() {
        let mut entries = [entry("b.rs", false), entry("src", true), entry("A.md", false), entry("Docs", true)];
        sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Docs", "src", "A.md", "b.rs"]);
    }

    #[test]
    fn skips_build_and_vcs_folders_only() {
        assert!(is_skipped(".git") && is_skipped("node_modules") && is_skipped("target"));
        assert!(is_skipped("dist") && is_skipped(".next"));
        assert!(!is_skipped("src") && !is_skipped(".github"));
    }

    #[test]
    fn matches_names_without_case() {
        assert!(name_matches("ReadMe.md", "readme"));
        assert!(name_matches("main.rs", "N.R"));
        assert!(!name_matches("main.rs", "lib"));
    }

    #[test]
    fn relative_path_uses_forward_slashes() {
        let root = Path::new("C:\\proj");
        assert_eq!(relative_path(root, &root.join("src").join("a.rs")), "src/a.rs");
        assert_eq!(relative_path(root, Path::new("elsewhere\\b.rs")), "elsewhere/b.rs");
    }

    #[test]
    fn decides_the_preview_kind() {
        assert_eq!(kind_of(Path::new("notes/README.MD")), Kind::Markdown);
        assert_eq!(kind_of(Path::new("logo.SVG")), Kind::Image);
        assert_eq!(kind_of(Path::new("photo.jpeg")), Kind::Image);
        assert_eq!(kind_of(Path::new("main.rs")), Kind::Other);
        assert_eq!(kind_of(Path::new("Makefile")), Kind::Other);
        assert_eq!(decode_text(b"fn main() {}".to_vec()).as_deref(), Some("fn main() {}"));
        assert_eq!(decode_text(vec![0xff, 0xfe, 0x00]), None);
        assert_eq!(decode_text(b"a\0b".to_vec()), None);
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(3 * 1_048_576), "3.0 MB");
    }
}
