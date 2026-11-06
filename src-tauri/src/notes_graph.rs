//! Graph of the workspace's Markdown notes: one node per `.md` file, one edge per link between two notes
//! (`[[wikilink]]`, `[[name|alias]]`, `[[name#heading]]` or a relative `[text](other.md)` link).

use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::assets::{inside, walk};

#[derive(Debug, Serialize)]
pub struct NoteNode {
    /// Path relative to the workspace, with `/` separators.
    pub id: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
pub struct NotesGraph {
    pub nodes: Vec<NoteNode>,
    pub edges: Vec<(String, String)>,
}

/// Link targets written in a note, as typed (no resolution).
fn link_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for chunk in text.split("[[").skip(1) {
        if let Some(end) = chunk.find("]]") {
            let name = chunk[..end].split(['|', '#']).next().unwrap_or("").trim();
            if !name.is_empty() {
                targets.push(name.to_string());
            }
        }
    }
    for chunk in text.split("](").skip(1) {
        let Some(end) = chunk.find(')') else { continue };
        let href = chunk[..end].split('#').next().unwrap_or("").trim();
        if href.to_ascii_lowercase().ends_with(".md") && !href.contains("://") {
            targets.push(href.replace("%20", " "));
        }
    }
    targets
}

/// `a/b/../c.md` -> `a/c.md`, without touching the disk.
fn normalize(path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    parts.join("/")
}

/// File name without `.md`, as written.
fn title(id: &str) -> &str {
    let name = id.rsplit('/').next().unwrap_or(id);
    name.strip_suffix(".md").unwrap_or(name)
}

/// Builds the graph from (relative id, text) pairs. Relative links resolve by path from the linking note,
/// wikilinks by file name (case-insensitive); links to missing notes are dropped.
fn build(notes: &[(String, String)]) -> NotesGraph {
    let ids: BTreeSet<&str> = notes.iter().map(|(id, _)| id.as_str()).collect();
    let mut by_name: HashMap<String, &str> = HashMap::new();
    for (id, _) in notes {
        by_name.entry(title(id).to_lowercase()).or_insert(id);
    }
    let mut edges = BTreeSet::new();
    for (id, text) in notes {
        let dir = Path::new(id).parent().unwrap_or(Path::new(""));
        for target in link_targets(text) {
            let relative = normalize(&dir.join(&target));
            let resolved = if ids.contains(relative.as_str()) {
                Some(relative.as_str())
            } else {
                by_name
                    .get(&title(&target.replace('\\', "/")).to_lowercase())
                    .copied()
            };
            if let Some(to) = resolved.filter(|to| to != id) {
                // Undirected: one edge per pair.
                let (a, b) = if id.as_str() < to {
                    (id.as_str(), to)
                } else {
                    (to, id.as_str())
                };
                edges.insert((a.to_string(), b.to_string()));
            }
        }
    }
    let nodes = notes
        .iter()
        .map(|(id, _)| NoteNode {
            id: id.clone(),
            title: title(id).to_string(),
        })
        .collect();
    NotesGraph {
        nodes,
        edges: edges.into_iter().collect(),
    }
}

/// Notes and their links for the whole workspace folder (same skipped folders and cap as the assets list).
#[tauri::command(async)]
pub fn notes_graph(cwd: String) -> Result<NotesGraph, String> {
    let root = PathBuf::from(&cwd);
    if !root.is_dir() {
        return Err(format!("{cwd} is not a folder"));
    }
    let mut files = Vec::new();
    walk(&root, &root, 0, &["md"], &mut files);
    let notes: Vec<(String, String)> = files
        .iter()
        .filter_map(|f| {
            Some((
                f.rel.replace('\\', "/"),
                std::fs::read_to_string(&f.path).ok()?,
            ))
        })
        .collect();
    Ok(build(&notes))
}

/// Text of one note of the workspace, for the side preview.
#[tauri::command(async)]
pub fn note_read(cwd: String, id: String) -> Result<String, String> {
    let file = inside(&cwd, &Path::new(&cwd).join(&id).to_string_lossy())?;
    std::fs::read_to_string(&file).map_err(|e| format!("cannot read {id}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_wikilinks_and_relative_links() {
        let notes = vec![
            (
                "index.md".to_string(),
                "See [[Ideas|my ideas]] and [setup](docs/setup.md), [[missing]].".to_string(),
            ),
            (
                "notes/ideas.md".to_string(),
                "Back to [home](../index.md) and [[ideas#self]].".to_string(),
            ),
            (
                "docs/setup.md".to_string(),
                "https://example.com/a.md is not a note".to_string(),
            ),
        ];
        let graph = build(&notes);
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.nodes[1].title, "ideas");
        assert_eq!(
            graph.edges,
            vec![
                ("docs/setup.md".to_string(), "index.md".to_string()),
                ("index.md".to_string(), "notes/ideas.md".to_string()),
            ]
        );
    }
}
