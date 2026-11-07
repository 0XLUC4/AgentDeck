//! "Link workspace": pick a project from an open app, a Git repository or any folder, or clone one
//! from GitHub, then name it. Original: src/components/LinkWorkspace/LinkWorkspaceDialog.tsx.

use agentdeck_core::git;
use agentdeck_core::logic::deck::{WorkspaceGit, folder_name};
use agentdeck_core::workspace::{self, FolderInfo, OpenProject};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Icon, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use crate::state::Deck;
use crate::theme::{hsla, palette, ui};

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Apps,
    Git,
    Folder,
    Clone,
}

const SOURCES: [(Source, &str, IconName); 4] = [
    (Source::Apps, "Open app", IconName::AppWindow),
    (Source::Git, "Git project", IconName::GitBranch),
    (Source::Folder, "Existing folder", IconName::Folder),
    (Source::Clone, "Clone from GitHub", IconName::CloudDownload),
];

/// What will be linked. `git` is `None` when unknown or not a repository.
#[derive(Clone)]
struct Choice {
    path: String,
    name: String,
    git: Option<(Option<String>, Option<String>)>,
}

struct GhRepo {
    name_with_owner: String,
    description: String,
    is_private: bool,
}

fn parse_repos(value: &Value) -> Vec<GhRepo> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(GhRepo {
                name_with_owner: r["nameWithOwner"].as_str()?.to_owned(),
                description: r["description"].as_str().unwrap_or("").to_owned(),
                is_private: r["isPrivate"] == true,
            })
        })
        .collect()
}

fn group_by_app(projects: &[OpenProject]) -> Vec<(String, Vec<OpenProject>)> {
    let mut groups: Vec<(String, Vec<OpenProject>)> = Vec::new();
    for p in projects {
        match groups.iter_mut().find(|(app, _)| *app == p.app) {
            Some((_, list)) => list.push(p.clone()),
            None => groups.push((p.app.clone(), vec![p.clone()])),
        }
    }
    groups
}

/// Opens the dialog in `window`.
pub fn open_link_workspace(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| LinkWorkspace::new(deck, window, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(ui(544.0).to_pixels(window.rem_size()))
            .p_0()
            .close_button(false)
            .child(view.clone())
    });
}

pub struct LinkWorkspace {
    deck: Entity<Deck>,
    source: Source,
    choice: Option<Choice>,
    name: Entity<InputState>,
    /// Name to put in the input on the next render (async results have no window).
    pending_name: Option<String>,
    // Open app
    projects: Option<Vec<OpenProject>>,
    // Git project / Existing folder
    info: Option<FolderInfo>,
    accept_plain: bool,
    inspect_error: Option<String>,
    busy: bool,
    // Clone from GitHub
    repos: Option<Vec<GhRepo>>,
    list_error: Option<String>,
    repo: Entity<InputState>,
    parent: Option<String>,
    cloning: bool,
    clone_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl LinkWorkspace {
    pub fn new(deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Choose a project first"));
        let repo = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search your repos, or type owner/name or a git URL")
        });
        let subscriptions = vec![
            cx.subscribe_in(&name, window, |this, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::PressEnter { .. }) && this.choice.is_some() {
                    this.submit(cx);
                    window.close_dialog(cx);
                }
            }),
            cx.subscribe(&repo, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        name.update(cx, |s, cx| s.set_disabled(true, cx));
        let mut this = Self {
            deck,
            source: Source::Apps,
            choice: None,
            name,
            pending_name: None,
            projects: None,
            info: None,
            accept_plain: false,
            inspect_error: None,
            busy: false,
            repos: None,
            list_error: None,
            repo,
            parent: None,
            cloning: false,
            clone_error: None,
            _subscriptions: subscriptions,
        };
        this.refresh_apps(cx);
        this
    }

    fn pick(&mut self, next: Option<Choice>, cx: &mut Context<Self>) {
        let name = next.as_ref().map(|c| if c.name.is_empty() { folder_name(&c.path) } else { c.name.clone() });
        self.name.update(cx, |s, cx| s.set_disabled(next.is_none(), cx));
        self.pending_name = Some(name.unwrap_or_default());
        self.choice = next;
        cx.notify();
    }

    fn switch_source(&mut self, source: Source, cx: &mut Context<Self>) {
        self.source = source;
        self.info = None;
        self.accept_plain = false;
        self.inspect_error = None;
        self.pick(None, cx);
        if source == Source::Clone && self.repos.is_none() {
            self.load_repos(cx);
        }
    }

    fn refresh_apps(&mut self, cx: &mut Context<Self>) {
        self.projects = None;
        cx.spawn(async move |this, cx| {
            let projects = cx
                .background_executor()
                .spawn(async { workspace::list_open_projects() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.projects = Some(projects);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_repos(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let listed = cx.background_executor().spawn(async { git::gh_list_repos() }).await;
            let _ = this.update(cx, |this, cx| {
                match listed {
                    Ok(v) => this.repos = Some(parse_repos(&v)),
                    Err(e) => {
                        this.repos = Some(Vec::new());
                        this.list_error = Some(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Folder picker for "Git project" / "Existing folder", then inspection.
    fn browse_folder(&mut self, cx: &mut Context<Self>) {
        let require_git = self.source == Source::Git;
        let title = if require_git { "Choose a Git repository" } else { "Choose a folder" };
        cx.spawn(async move |this, cx| {
            let Some(dir) = rfd::AsyncFileDialog::new().set_title(title).pick_folder().await else {
                return;
            };
            let dir = dir.path().to_string_lossy().into_owned();
            let ok = this.update(cx, |this, cx| {
                this.busy = true;
                this.info = None;
                this.accept_plain = false;
                this.inspect_error = None;
                cx.notify();
            });
            if ok.is_err() {
                return;
            }
            let path = dir.clone();
            let inspected = cx
                .background_executor()
                .spawn(async move { workspace::inspect_folder(path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match inspected {
                    Ok(info) => {
                        let git = info.is_git.then(|| (info.branch.clone(), info.remote.clone()));
                        let choice = (!require_git || info.is_git).then(|| Choice {
                            path: info.path.clone(),
                            name: info.name.clone(),
                            git,
                        });
                        this.info = Some(info);
                        this.pick(choice, cx);
                    }
                    Err(e) => {
                        // Still link the plain path.
                        this.inspect_error = Some(e);
                        this.pick(
                            Some(Choice {
                                name: folder_name(&dir),
                                path: dir,
                                git: None,
                            }),
                            cx,
                        );
                    }
                }
            });
        })
        .detach();
    }

    fn browse_parent(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            if let Some(dir) = rfd::AsyncFileDialog::new().set_title("Clone into folder").pick_folder().await {
                let _ = this.update(cx, |this, cx| {
                    this.parent = Some(dir.path().to_string_lossy().into_owned());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn clone_repo(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.read(cx).value().trim().to_owned();
        let Some(parent) = self.parent.clone().filter(|_| !repo.is_empty()) else {
            return;
        };
        self.cloning = true;
        self.clone_error = None;
        self.pick(None, cx);
        cx.spawn(async move |this, cx| {
            let cloned = cx
                .background_executor()
                .spawn(async move {
                    let path = git::git_clone(repo, parent)?;
                    let info = workspace::inspect_folder(path.clone()).ok();
                    Ok::<_, String>((path, info))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloning = false;
                match cloned {
                    Ok((path, info)) => this.pick(
                        Some(Choice {
                            name: info.as_ref().map_or_else(|| folder_name(&path), |i| i.name.clone()),
                            git: Some((
                                info.as_ref().and_then(|i| i.branch.clone()),
                                info.as_ref().and_then(|i| i.remote.clone()),
                            )),
                            path,
                        }),
                        cx,
                    ),
                    Err(e) => this.clone_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(choice) = self.choice.clone() else {
            return;
        };
        let name = self.name.read(cx).value().trim().to_owned();
        self.deck.update(cx, |d, cx| {
            if let Some(existing) = d.state.workspace_for_path(&choice.path).cloned() {
                d.state.select_workspace(&existing.id);
                d.toast(format!("\"{}\" is already linked.", existing.name), cx);
            } else {
                let git = choice.git.map(|(branch, _)| WorkspaceGit { branch });
                d.state
                    .link_workspace(&choice.path, (!name.is_empty()).then_some(name.as_str()), git);
                d.save(cx);
            }
            cx.notify();
        });
    }

    fn git_line(branch: &Option<String>, remote: &Option<String>, cx: &App) -> Div {
        let p = palette(cx);
        div()
            .flex()
            .min_w_0()
            .items_center()
            .gap(ui(6.0))
            .text_xs()
            .text_color(hsla(p.muted))
            .child(Icon::new(IconName::GitBranch).size(ui(13.0)))
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(hsla(p.fg))
                    .child(branch.clone().unwrap_or_else(|| "Detached HEAD".into())),
            )
            .children(
                remote
                    .clone()
                    .map(|r| div().truncate().text_color(hsla(p.subtle)).child(r)),
            )
    }

    fn warning(text: String, cx: &App) -> Div {
        let p = palette(cx);
        div()
            .flex()
            .items_start()
            .gap_2()
            .py_3()
            .text_xs()
            .text_color(hsla(p.muted))
            .child(Icon::new(IconName::TriangleAlert).size(ui(14.0)).text_color(hsla(p.wait)))
            .child(text)
    }

    fn outline_button(id: &'static str, icon: IconName, label: String, disabled: bool, cx: &App) -> Stateful<Div> {
        let p = palette(cx);
        div()
            .id(id)
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .h_9()
            .px_3()
            .rounded(ui(6.0))
            .border_1()
            .border_color(hsla(p.line))
            .text_color(hsla(p.fg))
            .when(disabled, |el| el.opacity(0.5))
            .when(!disabled, |el| el.cursor_pointer().hover(|s| s.bg(hsla(p.hover))))
            .child(Icon::new(icon).size(ui(16.0)))
            .child(label)
    }

    fn apps_panel(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let selected = self.choice.as_ref().map(|c| c.path.clone());
        let list: AnyElement = match &self.projects {
            None => div()
                .flex()
                .flex_col()
                .gap_1()
                .children((0..3).map(|_| {
                    div()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .gap(ui(6.0))
                        .h_11()
                        .px_2()
                        .child(div().h(ui(10.0)).w_1_3().rounded(ui(4.0)).bg(hsla(p.raised)))
                        .child(div().h_2().w_2_3().rounded(ui(4.0)).bg(hsla(p.raised)).opacity(0.6))
                }))
                .into_any_element(),
            Some(projects) if projects.is_empty() => div()
                .py_6()
                .flex()
                .justify_center()
                .text_color(hsla(p.muted))
                .child("No apps with an open project")
                .into_any_element(),
            Some(projects) => div()
                .children(group_by_app(projects).into_iter().map(|(app, list)| {
                    let icon = if app.to_lowercase().contains("explor") { IconName::FolderOpen } else { IconName::Code };
                    div()
                        .mb_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(ui(6.0))
                                .px_2()
                                .py_1()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(hsla(p.subtle))
                                .child(Icon::new(icon).size(ui(13.0)))
                                .child(app.clone()),
                        )
                        .children(list.into_iter().map(|project| {
                            let pressed = selected.as_deref() == Some(project.path.as_str());
                            let id = SharedString::from(format!("app-{app}-{}", project.path));
                            let choice = Choice {
                                path: project.path.clone(),
                                name: project.name.clone(),
                                git: project.is_git.then_some((None, None)),
                            };
                            div()
                                .id(id)
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_2()
                                .py(ui(6.0))
                                .rounded(ui(6.0))
                                .cursor_pointer()
                                .when(pressed, |el| el.bg(hsla(p.raised)))
                                .hover(|s| s.bg(hsla(p.hover)))
                                .on_click(cx.listener(move |this, _, _, cx| this.pick(Some(choice.clone()), cx)))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .truncate()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(hsla(p.fg))
                                                .child(project.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .font_family(agentdeck_core::logic::theme::FONT_MONO)
                                                .text_xs()
                                                .text_color(hsla(p.subtle))
                                                .child(project.path.clone()),
                                        ),
                                )
                                .when(project.is_git, |el| {
                                    el.child(
                                        div()
                                            .flex()
                                            .flex_shrink_0()
                                            .items_center()
                                            .gap_1()
                                            .text_xs()
                                            .text_color(hsla(p.muted))
                                            .child(Icon::new(IconName::GitBranch).size(ui(13.0)))
                                            .child("Git"),
                                    )
                                })
                        }))
                }))
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(hsla(p.subtle))
                            .child("Projects open in your editors and File Explorer."),
                    )
                    .child(
                        Button::new("apps-refresh")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::RefreshCw))
                            .tooltip("Refresh")
                            .disabled(self.projects.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_apps(cx))),
                    ),
            )
            .child(div().id("apps-list").max_h(ui(256.0)).overflow_y_scroll().child(list))
    }

    fn folder_panel(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let require_git = self.source == Source::Git;
        let not_git = require_git && !self.accept_plain && self.info.as_ref().is_some_and(|i| !i.is_git);
        let label = if self.busy { "Inspecting\u{2026}" } else { "Choose a folder\u{2026}" };
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_3()
            .child({
                let busy = self.busy;
                Self::outline_button("folder-browse", IconName::FolderOpen, label.into(), busy, cx).on_click(
                    cx.listener(move |this, _, _, cx| {
                        if !busy {
                            this.browse_folder(cx);
                        }
                    }),
                )
            });
        if self.info.as_ref().is_some_and(|i| !i.exists) {
            panel = panel.child(div().text_xs().text_color(hsla(p.bad)).child("This folder does not exist."));
        }
        if let (true, Some(info)) = (not_git, &self.info) {
            let choice = Choice {
                path: info.path.clone(),
                name: info.name.clone(),
                git: None,
            };
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded(ui(6.0))
                    .bg(crate::theme::hsla_a(p.wait, 0.1))
                    .px_3()
                    .py(ui(10.0))
                    .text_xs()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_color(hsla(p.wait))
                            .child(Icon::new(IconName::TriangleAlert).size(ui(14.0)))
                            .child("This folder is not a Git repository"),
                    )
                    .child(
                        div()
                            .truncate()
                            .font_family(agentdeck_core::logic::theme::FONT_MONO)
                            .text_color(hsla(p.subtle))
                            .child(info.path.clone()),
                    )
                    .child(
                        div()
                            .id("use-plain")
                            .text_color(hsla(p.fg))
                            .underline()
                            .cursor_pointer()
                            .child("Use it as a plain folder")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.accept_plain = true;
                                this.pick(Some(choice.clone()), cx);
                            })),
                    ),
            );
        } else if let Some(choice) = &self.choice {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .truncate()
                            .font_family(agentdeck_core::logic::theme::FONT_MONO)
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .child(choice.path.clone()),
                    )
                    .children(choice.git.as_ref().map(|(b, r)| Self::git_line(b, r, cx))),
            );
        }
        if let Some(e) = &self.inspect_error {
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(hsla(p.subtle))
                    .child(format!("Git info unavailable ({e}). The folder will be linked as is.")),
            );
        }
        panel
    }

    fn clone_panel(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let query = self.repo.read(cx).value().trim().to_lowercase();
        let current = self.repo.read(cx).value().to_string();
        let list: AnyElement = match (&self.repos, &self.list_error) {
            (None, _) => div()
                .py_3()
                .text_xs()
                .text_color(hsla(p.subtle))
                .child("Loading your repositories\u{2026}")
                .into_any_element(),
            (Some(_), Some(e)) => Self::warning(
                format!("Repository list unavailable ({e}). You can still type owner/name or a git URL."),
                cx,
            )
            .into_any_element(),
            (Some(repos), None) => {
                let shown: Vec<&GhRepo> = repos
                    .iter()
                    .filter(|r| r.name_with_owner.to_lowercase().contains(&query))
                    .take(50)
                    .collect();
                if shown.is_empty() {
                    div()
                        .py_3()
                        .text_xs()
                        .text_color(hsla(p.subtle))
                        .child("No matching repository.")
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .children(shown.into_iter().map(|r| {
                            let name = r.name_with_owner.clone();
                            let cloning = self.cloning;
                            div()
                                .id(SharedString::from(format!("repo-{name}")))
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .py(ui(6.0))
                                .rounded(ui(6.0))
                                .cursor_pointer()
                                .when(current == name, |el| el.bg(hsla(p.raised)))
                                .hover(|s| s.bg(hsla(p.hover)))
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |this, _, window, cx| {
                                        if !cloning {
                                            this.repo.update(cx, |s, cx| s.set_value(name.clone(), window, cx));
                                            cx.notify();
                                        }
                                    }
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .truncate()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(hsla(p.fg))
                                                .child(name),
                                        )
                                        .when(!r.description.is_empty(), |el| {
                                            el.child(
                                                div()
                                                    .truncate()
                                                    .text_xs()
                                                    .text_color(hsla(p.subtle))
                                                    .child(r.description.clone()),
                                            )
                                        }),
                                )
                                .when(r.is_private, |el| {
                                    el.child(Icon::new(IconName::Lock).size(ui(13.0)).text_color(hsla(p.subtle)))
                                })
                        }))
                        .into_any_element()
                }
            }
        };
        let can_clone = !self.cloning && self.parent.is_some() && !query.is_empty();
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(Input::new(&self.repo).disabled(self.cloning))
            .child(div().id("repo-list").max_h(ui(160.0)).overflow_y_scroll().child(list))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child({
                        let cloning = self.cloning;
                        let label = if self.parent.is_some() { "Change folder\u{2026}" } else { "Clone into\u{2026}" };
                        Self::outline_button("clone-parent", IconName::FolderOpen, label.into(), cloning, cx).on_click(
                            cx.listener(move |this, _, _, cx| {
                                if !cloning {
                                    this.browse_parent(cx);
                                }
                            }),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(agentdeck_core::logic::theme::FONT_MONO)
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .child(self.parent.clone().unwrap_or_else(|| "No destination folder".into())),
                    )
                    .child(
                        Self::outline_button(
                            "clone-run",
                            IconName::CloudDownload,
                            if self.cloning { "Cloning\u{2026}" } else { "Clone" }.into(),
                            !can_clone,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if can_clone {
                                this.clone_repo(cx);
                            }
                        })),
                    ),
            );
        if let Some(e) = &self.clone_error {
            panel = panel.child(div().text_xs().text_color(hsla(p.bad)).child(e.clone()));
        }
        if let Some(choice) = &self.choice {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xs().text_color(hsla(p.ok)).child("Cloned. Link it below."))
                    .child(
                        div()
                            .truncate()
                            .font_family(agentdeck_core::logic::theme::FONT_MONO)
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .child(choice.path.clone()),
                    )
                    .children(choice.git.as_ref().map(|(b, r)| Self::git_line(b, r, cx))),
            );
        }
        panel
    }
}

impl Render for LinkWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(name) = self.pending_name.take() {
            let placeholder = self
                .choice
                .as_ref()
                .map_or_else(|| "Choose a project first".to_owned(), |c| folder_name(&c.path));
            self.name.update(cx, |s, cx| {
                s.set_value(name, window, cx);
                s.set_placeholder(placeholder, window, cx);
            });
        }
        let p = palette(cx);
        let has_choice = self.choice.is_some();
        let tabs: Vec<_> = SOURCES.into_iter().map(|(source, label, icon)| {
            let active = self.source == source;
            div()
                .id(label)
                .flex()
                .h_8()
                .items_center()
                .justify_center()
                .gap(ui(6.0))
                .rounded(ui(6.0))
                .text_xs()
                .cursor_pointer()
                .when(active, |el| {
                    el.bg(hsla(p.raised))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(hsla(p.fg))
                })
                .when(!active, |el| {
                    el.text_color(hsla(p.muted))
                        .hover(|s| s.text_color(hsla(p.fg)))
                })
                .on_click(cx.listener(move |this, _, _, cx| this.switch_source(source, cx)))
                .child(Icon::new(icon).size(ui(14.0)))
                .child(label)
        }).collect();
        let panel = match self.source {
            Source::Apps => self.apps_panel(cx),
            Source::Clone => self.clone_panel(cx),
            Source::Git | Source::Folder => self.folder_panel(cx),
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .p_5()
            .text_size(ui(13.0))
            .text_color(hsla(p.fg))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(ui(15.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Link workspace"),
                    )
                    .child(
                        Button::new("link-close")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::X))
                            .tooltip("Close")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(4)
                    .gap_1()
                    .rounded(ui(8.0))
                    .bg(hsla(p.base))
                    .p_1()
                    .children(tabs),
            )
            .child(div().min_h(ui(112.0)).child(panel))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(ui(6.0))
                    .child(div().text_xs().text_color(hsla(p.muted)).child("Name"))
                    .child(Input::new(&self.name).disabled(!has_choice)),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("link-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("link-submit")
                            .primary()
                            .label("Link workspace")
                            .disabled(!has_choice)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.choice.is_some() {
                                    this.submit(cx);
                                    window.close_dialog(cx);
                                }
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenProject, group_by_app, parse_repos};

    #[test]
    fn repos_and_groups() {
        let raw = r#"[{ "nameWithOwner": "a/b", "description": null, "isPrivate": true }, { "description": "no name" }]"#;
        let repos = parse_repos(&serde_json::from_str(raw).expect("valid json"));
        assert_eq!(repos.len(), 1);
        assert!(repos[0].is_private && repos[0].description.is_empty());
        let p = |app: &str, path: &str| OpenProject {
            app: app.into(),
            name: path.into(),
            path: path.into(),
            is_git: false,
        };
        let groups = group_by_app(&[p("VS Code", "a"), p("Explorer", "b"), p("VS Code", "c")]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].1.len(), 2);
    }
}
