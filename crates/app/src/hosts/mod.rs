//! SSH hosts: the saved list (hosts.json, edited in Settings > Hosts), the last "Test connection"
//! results, the status bar item and the remote files dialog. Model and ssh calls:
//! `agentdeck_core::hosts`.

mod files;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use agentdeck_core::hosts::{self, Host};
use gpui_kit::assets::IconName;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

use crate::theme::ui;
pub use files::open_files;

#[derive(Default)]
struct Hosts {
    list: Vec<Host>,
    /// Last "Test connection" result per host id.
    reachable: HashMap<String, bool>,
    data_dir: PathBuf,
}

impl Global for Hosts {}

/// Hosts whose tmux was already looked for in this run.
static TMUX_CHECKED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Loads hosts.json. Called once from `main`.
pub fn init(data_dir: PathBuf, cx: &mut App) {
    let list = hosts::load_hosts(&data_dir).unwrap_or_else(|e| {
        eprintln!("hosts not loaded: {e}");
        Vec::new()
    });
    cx.set_global(Hosts { list, reachable: HashMap::new(), data_dir });
}

pub fn list(cx: &App) -> Vec<Host> {
    cx.try_global::<Hosts>().map(|h| h.list.clone()).unwrap_or_default()
}

pub fn get(id: &str, cx: &App) -> Option<Host> {
    cx.try_global::<Hosts>()?.list.iter().find(|h| h.id == id).cloned()
}

/// Last test result of a host (None = never tested in this run).
pub fn reachable(id: &str, cx: &App) -> Option<bool> {
    cx.try_global::<Hosts>()?.reachable.get(id).copied()
}

/// Replaces the list and writes hosts.json.
pub fn save(list: Vec<Host>, cx: &mut App) -> Result<(), String> {
    let hosts = cx.default_global::<Hosts>();
    hosts.reachable.retain(|id, _| list.iter().any(|h| &h.id == id));
    hosts.list = list;
    let result = hosts::save_hosts(&hosts.data_dir, &hosts.list);
    cx.refresh_windows();
    result
}

pub fn set_reachable(id: &str, ok: bool, cx: &mut App) {
    cx.default_global::<Hosts>().reachable.insert(id.to_owned(), ok);
    cx.refresh_windows();
}

/// True the first time it is asked for `host_id` in this run (tmux is looked for once per host).
pub fn first_tmux_check(host_id: &str) -> bool {
    let mut checked = TMUX_CHECKED.lock().unwrap_or_else(PoisonError::into_inner);
    checked.get_or_insert_default().insert(host_id.to_owned())
}

/// Ends the tmux session of a pane closed for good (its agent stops), in the background.
pub fn end_session(host: Host, pane_id: String, cx: &App) {
    cx.background_spawn(async move {
        if let Err(e) = host.kill_session(&pane_id) {
            eprintln!("tmux session of {pane_id} on {} not ended: {e}", host.name);
        }
    })
    .detach();
}

/// Status bar: server icon and "N hosts"; the tooltip says which answered their last test.
pub fn status_item(cx: &App) -> Option<AnyElement> {
    let hosts = cx.try_global::<Hosts>().filter(|h| !h.list.is_empty())?;
    let count = hosts.list.len();
    let lines: Vec<String> = hosts
        .list
        .iter()
        .map(|h| {
            let state = match hosts.reachable.get(&h.id) {
                Some(true) => "reachable",
                Some(false) => "unreachable",
                None => "not tested",
            };
            format!("{} ({}): {state}", h.name, h.target)
        })
        .collect();
    let tip = SharedString::from(lines.join("\n"));
    Some(
        div()
            .id("status-hosts")
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(ui(6.0))
            .whitespace_nowrap()
            .child(div().text_size(ui(13.0)).child(IconName::Server))
            .child(if count == 1 { "1 host".to_owned() } else { format!("{count} hosts") })
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .into_any_element(),
    )
}
