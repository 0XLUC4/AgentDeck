//! Files page: the workspace folder as a tree (or a flat list of name matches), the selected file
//! previewed beside it, and a button typing its path into the active pane.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use agentdeck_core::logic::theme::FONT_MONO;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::Disableable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;


use super::viewer::ModelViewer;
use crate::orchestrator::runtime::send_prompt;
use crate::state::Deck;
use crate::theme::{hsla, palette, ui};

const SKIPPED_FOLDERS: &[&str] = &[".git", "node_modules", "target", "dist", ".next"];
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];
const MAX_TEXT_BYTES: u64 = 512 * 1024;
const MAX_LINES: usize = 2000;
const MAX_WALKED_FILES: usize = 5000;
const MAX_MATCHES: usize = 200;

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
    if ext == "md" {
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
    Markdown(String),
    Image,
    /// The first `MAX_LINES` lines, and how many the file has.
    Text { lines: Vec<SharedString>, total: usize },
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
    let Some(text) = decode_text(bytes) else {
        return Preview::Unavailable(size);
    };
    if kind == Kind::Markdown {
        return Preview::Markdown(text);
    }
    Preview::Text {
        total: text.lines().count(),
        // Tabs have no width in a plain text run.
        lines: text.lines().take(MAX_LINES).map(|l| l.replace('\t', "    ").into()).collect(),
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

    fn select(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.selected = Some(path.clone());
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
        self.loading = Some(cx.spawn(async move |this, cx| {
            let preview = cx.background_executor().spawn(async move { load_preview(&path) }).await;
            let _ = this.update(cx, |this, cx| {
                this.preview = preview;
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
                    .rounded(ui(6.0))
                    .cursor_pointer()
                    .when(current, |d| d.bg(hsla(p.raised)))
                    .when(!current, |d| d.hover(|s| s.bg(hsla(p.hover))))
                    .child(div().flex_shrink_0().text_size(ui(13.0)).text_color(hsla(p.subtle)).child(icon))
                    .child(div().truncate().child(entry.name.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if entry.dir {
                            this.toggle(entry.path.clone(), cx);
                        } else {
                            this.select(entry.path.clone(), cx);
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
                    .rounded(ui(6.0))
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
                    .on_click(cx.listener(move |this, _, _, cx| this.select(path.clone(), cx)))
                    .into_any_element()
            })
            .collect()
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
        let mut truncated = None;
        let content = match &self.preview {
            Preview::Empty | Preview::Loading => message("Reading…".into(), p.subtle),
            Preview::Error(e) => message(e.clone(), p.bad),
            Preview::Unavailable(size) => message(format!("No preview ({})", format_size(*size)), p.subtle),
            // One id per file: the view keeps its parsed text and scroll position by id.
            Preview::Markdown(text) => frame().p_4().child(
                TextView::markdown(SharedString::from(format!("file-{rel}")), text.clone())
                    .selectable(true)
                    .scrollable(true),
            ),
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
            Preview::Text { lines, total } => {
                if *total > lines.len() {
                    truncated = Some(format!("Showing the first {} of {total} lines.", lines.len()));
                }
                let rows = lines.iter().enumerate().map(|(i, line)| {
                    div()
                        .h(ui(18.0))
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .w(ui(48.0))
                                .flex_shrink_0()
                                .pr_3()
                                .text_right()
                                .text_color(hsla(p.subtle))
                                .child((i + 1).to_string()),
                        )
                        .child(div().flex_shrink_0().whitespace_nowrap().child(line.clone()))
                });
                frame().child(
                    div()
                        .id("file-text")
                        .size_full()
                        .overflow_scroll()
                        .py_2()
                        .font_family(FONT_MONO)
                        .text_size(ui(12.0))
                        .children(rows),
                )
            }
        };
        let can_send = self.deck.read(cx).state.active_pane_id.is_some();
        let typed = format!("@{rel} ");
        let hover = hsla(p.hover);
        let close = div()
            .id("files-preview-close")
            .p_1()
            .rounded(ui(6.0))
            .text_color(hsla(p.muted))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(IconName::X)
            // Back to the panes; the tree stays open.
            .on_click(cx.listener(|this, _, _, cx| {
                this.selected = None;
                this.viewer = None;
                cx.notify();
            }));
        side.child(
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .px_4()
                .py_2()
                .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(rel))
                .child(
                    Button::new("files-send")
                        .icon(IconName::Send)
                        .label("Send path to agent")
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
                .child(close),
        )
        .child(content)
        .when_some(truncated, |d, note| {
            d.child(div().flex_shrink_0().px_4().py_1().text_size(ui(12.0)).text_color(hsla(p.subtle)).child(note))
        })
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
                            .rounded(ui(6.0))
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

    use super::{Entry, Kind, decode_text, format_size, is_skipped, kind_of, name_matches, relative_path, sort_entries};

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
